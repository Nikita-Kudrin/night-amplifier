use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::RwLockReadGuard;
use tracing::{debug, warn};

use super::channel::{CapturedFrame, QueueDepth};
use super::drop_log::DropLog;
use crate::camera::RawFrame;
use crate::disk_writer::WritingSessionType;
use crate::frame::Frame;
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CaptureMode, CaptureSession, CaptureSettings, ConnectedCameraInfo,
};
use crate::stacking::StackingType;

/// Dedicated storage task running on its own OS thread.
///
/// Receives `CapturedFrame` messages from the storage channel and saves
/// raw frames to disk via the existing `DiskWriterHandle`. The storage
/// channel has independent capacity and dropping logic from the stacking
/// channel.
pub fn run_storage_task(
    state: Arc<AppState>,
    storage_rx: mpsc::Receiver<CapturedFrame>,
    storage_depth: QueueDepth,
    rt: tokio::runtime::Handle,
) {
    debug!("Storage task started");

    let mut warnings = StorageWarnings::default();

    while let Ok(msg) = storage_rx.recv() {
        storage_depth.taken();

        let CapturedFrame {
            frame,
            frame_number,
            settings,
            camera_info,
        } = msg;

        // Only save if raw frame saving is still enabled for the mode this frame was
        // captured in — the settings travel with the frame, so a mode change mid-flight
        // does not retroactively decide the fate of frames already in the queue.
        if !settings.saves_raw_frames() || !state.disk_writer.is_enabled() {
            continue;
        }

        rt.block_on(save_frame_to_disk(
            &state,
            &frame,
            frame_number,
            &settings,
            &camera_info,
            &mut warnings,
        ));
    }

    warnings.finish(&state);

    debug!("Storage task ended");
}

/// How far behind the disk is, as the storage task has reported it so far.
///
/// One value threaded through the loop rather than three: the SSE warning latch, the
/// throttle behind it, and the drop counter are all the same story about the same disk,
/// and splitting them across parameters was what pushed this past a readable signature.
struct StorageWarnings {
    /// Whether the frontend currently believes the queue is backed up.
    active: bool,
    /// When the last `DiskWriterWarning` event went out.
    last_sent: std::time::Instant,
    /// Frames the writer would not take, rate-limited for the log.
    drops: DropLog,
}

impl Default for StorageWarnings {
    fn default() -> Self {
        Self {
            active: false,
            last_sent: std::time::Instant::now(),
            drops: DropLog::default(),
        }
    }
}

impl StorageWarnings {
    /// How often the frontend is told the queue is still backed up.
    const RESEND_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

    /// Report a frame the writer had no room for.
    fn record_drop(&mut self, frame_number: u64, error: &crate::disk_writer::DiskWriterError) {
        // Rate-limited for the same reason the capture-side drop is: at the short
        // exposures raw-saving Live view allows, a disk that cannot keep up turns away
        // most frames, and a line each buries everything else in the log.
        if let Some(dropped) = self.drops.record() {
            warn!(error = %error, frame_number, dropped, "Frames dropped: could not be queued for saving");
        }
    }

    /// Tell the frontend where the queue stands, no more often than the interval.
    fn observe_depth(&mut self, state: &AppState, queue_depth: usize) {
        use crate::disk_writer::QUEUE_WARNING_THRESHOLD;

        if queue_depth > QUEUE_WARNING_THRESHOLD {
            let now = std::time::Instant::now();
            if self.active && now.duration_since(self.last_sent) < Self::RESEND_INTERVAL {
                return;
            }
            self.active = true;
            self.last_sent = now;
            let _ = state
                .events
                .send(ServerEvent::DiskWriterWarning { queue_depth });
            return;
        }

        if self.active {
            self.active = false;
            state.disk_writer.clear_queue_warning();
            let _ = state.events.send(ServerEvent::DiskWriterWarningCleared);
        }
    }

    /// Close the books at the end of the session: report the tail of a drop burst the
    /// interval swallowed, and clear a warning the frontend would otherwise keep showing.
    fn finish(&mut self, state: &AppState) {
        if let Some(dropped) = self.drops.flush() {
            warn!(
                dropped,
                "Frames dropped since the last report: could not be queued for saving"
            );
        }
        if self.active {
            state.disk_writer.clear_queue_warning();
            let _ = state.events.send(ServerEvent::DiskWriterWarningCleared);
        }
    }
}

/// Get camera info from state
pub async fn get_camera_info(state: &AppState, camera_id: &str) -> Option<ConnectedCameraInfo> {
    let cameras: RwLockReadGuard<'_, HashMap<String, ConnectedCameraInfo>> =
        state.cameras.read().await;
    cameras.get(camera_id).cloned()
}

/// Initialize capture session (disk writer, etc.)
///
/// `resume_dir` rejoins an existing raw-frame directory instead of creating a
/// timestamped one, so a session interrupted by a device fault stays in a
/// single folder across the reconnect.
pub async fn initialize_capture_session(
    state: &AppState,
    resume_dir: Option<std::path::PathBuf>,
) -> Result<(), String> {
    let settings: RwLockReadGuard<'_, CaptureSettings> = state.settings.read().await;
    let enabled = settings.disk_writing_enabled();
    state.disk_writer.set_enabled(enabled);

    // A session can still be open here: a settings update lands between the capture
    // state flipping and this call, or a previous capture ended without one. Either way
    // this capture opens its own, so let go of the old one rather than letting
    // `ensure_session` adopt it later.
    state.disk_writer.abandon_session();
    if !enabled {
        return Ok(());
    }

    let session_type = session_type_for(&settings);

    match resume_dir {
        Some(dir) => state
            .disk_writer
            .resume_session(dir, session_type)
            .map_err(|e| format!("Failed to reopen capture directory: {}", e))?,
        None => state
            .disk_writer
            .start_session(session_type, settings.capture_mode().session_dir_suffix())
            .map_err(|e| format!("Failed to create capture directory: {}", e))?,
    };
    Ok(())
}

/// The container a session's raw frames go into.
///
/// Keyed on the stacking *type*, not the capture mode: a Planetary live-view run wants
/// the same SER container a Planetary stacking run does.
pub fn session_type_for(settings: &CaptureSettings) -> WritingSessionType {
    match settings.stacking_type {
        StackingType::Planetary => WritingSessionType::VideoContainer,
        _ => WritingSessionType::IndividualFrames,
    }
}

/// Bring the disk writer in line with settings that just changed. `POST
/// /api/settings` can turn saving on or change capture mode long after
/// `initialize_capture_session` ran — the writer's enabled flag, whether a session
/// directory is open, and whether its name still matches the mode are one decision,
/// made in one place. Rolling the directory on a mode change stops a folder from
/// lying about its contents: Live view then Stacking without stopping is ordinary,
/// and `ensure_session` alone would leave stacked subs in a folder named `-live`.
///
/// # Locking
/// Both arguments are passed in, not read here: `AppState::frame_processed` takes
/// `session` *then* `settings`, so reading either lock under a settings guard would
/// invert that order — the caller resolves both before taking the guard and calls
/// this after dropping it.
pub async fn sync_disk_session(
    state: &AppState,
    settings: &CaptureSettings,
    capture_active: bool,
) {
    let enabled = settings.disk_writing_enabled();
    state.disk_writer.set_enabled(enabled);
    if !enabled {
        return;
    }

    // Nothing is being captured, so there is nothing to file yet: opening a directory
    // now would leave an empty one behind every time a switch is flipped between
    // sessions, and capture start opens the right one anyway.
    if !capture_active {
        return;
    }

    let session_type = session_type_for(settings);
    let mode = settings.capture_mode();

    // A directory whose name names no mode predates the suffixes or was renamed by hand;
    // leave it alone rather than rolling a session on every settings update.
    let open_mode = state
        .disk_writer
        .session_name()
        .and_then(|name| CaptureMode::from_session_dir_name(&name));
    if let Some(open_mode) = open_mode {
        if open_mode != mode {
            debug!(?open_mode, new_mode = ?mode, "Capture mode changed, rolling raw session directory");
            // Abandoned rather than ended: `end_session` waits on the writer, which is
            // the wrong thing to do on a request thread. Frames already queued keep the
            // directory they were stamped with, and a SER container is closed out by the
            // worker as soon as a frame from the new session reaches it.
            state.disk_writer.abandon_session();
        }
    }

    if let Err(e) = state
        .disk_writer
        .ensure_session(session_type, mode.session_dir_suffix())
    {
        warn!(error = %e, "Could not open a capture directory for saving");
        state.send_error(format!("Saving is on but no folder could be created: {}", e));
        return;
    }

    // A reconnect rejoins the directory recorded in the resume plan. Left stale, it would
    // rejoin the folder this call just abandoned.
    if let Some(plan) = state.session_resume_plan.write().await.as_mut() {
        plan.disk_session_dir = state.disk_writer.session_dir();
    }
}

/// Save a frame to disk and handle queue warnings
async fn save_frame_to_disk(
    state: &AppState,
    frame: &Arc<RawFrame>,
    frame_number: u64,
    settings: &CaptureSettings,
    camera_info: &ConnectedCameraInfo,
    warnings: &mut StorageWarnings,
) {
    use crate::fits::FitsMetadata;
    use chrono::Utc;

    let raw_frame = Arc::clone(frame);
    let mut metadata = FitsMetadata::new()
        .with_exposure_us(settings.exposure_us)
        .with_gain(settings.gain)
        .with_offset(settings.offset)
        .with_camera(&camera_info.info.name)
        .with_frame_number(frame_number)
        .with_binning(settings.bin)
        .with_date_obs(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string());

    if camera_info.info.has_cooler {
        if let Some(set_temp) = settings.target_temp_c {
            metadata = metadata.with_set_temp(set_temp);
        }
        if let Some(status) = state.get_camera_status(&camera_info.info.name).await {
            metadata = metadata.with_temperature(status.temperature_c);
        }
    }

    if let Err(e) = state.disk_writer.queue_raw_frame(
        raw_frame,
        frame_number,
        metadata,
        camera_info.info.sensor_type,
        camera_info.info.bayer_pattern,
    ) {
        warnings.record_drop(frame_number, &e);
    }

    warnings.observe_depth(state, state.disk_writer.queue_depth());
}

/// Check if we should stop due to a burst of *current* camera-capture failures.
///
/// Uses a sliding window (`CaptureSession::record_rejection`) rather than the
/// lifetime-cumulative `rejected_count`, so a camera that failed sporadically
/// across an otherwise-healthy multi-hour session never trips this — only a
/// real, currently-active failure burst does (e.g. ~10 capture failures within
/// a second, consistent with a genuine disconnect rather than a hiccup).
pub async fn should_stop_on_errors(state: &AppState) -> bool {
    let session: RwLockReadGuard<'_, CaptureSession> = state.session.read().await;
    session.rejection_rate_exceeded() && session.stacked_count == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::REJECTION_RATE_THRESHOLD;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn should_stop_on_errors_false_when_healthy() {
        let (state, _disk_writer) = AppState::new_for_testing();
        assert!(!should_stop_on_errors(&state).await);
    }

    #[tokio::test]
    async fn should_stop_on_errors_true_on_burst_with_no_stacked_frames() {
        let (state, _disk_writer) = AppState::new_for_testing();
        {
            let mut session = state.session.write().await;
            let now = Instant::now();
            for i in 0..REJECTION_RATE_THRESHOLD {
                session.record_rejection(now + Duration::from_millis(i as u64));
            }
        }
        assert!(should_stop_on_errors(&state).await);
    }

    /// A capture must not inherit whatever directory happened to be open. A settings
    /// update can land between the capture state changing and this call, and a previous
    /// capture can end without one being closed — either way `ensure_session` would
    /// later adopt the stale folder and file this session's frames into it.
    #[tokio::test]
    async fn initialize_capture_session_never_adopts_an_open_session() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state
            .disk_writer
            .start_session(WritingSessionType::IndividualFrames, "-live")
            .unwrap();
        let stale = state.disk_writer.session_dir().unwrap();

        // Saving is off in every mode by default, so this opens nothing of its own.
        initialize_capture_session(&state, None).await.unwrap();

        assert_eq!(
            state.disk_writer.session_dir(),
            None,
            "the capture kept {stale:?} open, so a later ensure_session would adopt it"
        );
    }

    /// With saving on, the capture opens its own directory rather than continuing the
    /// one that was already there.
    #[tokio::test]
    async fn initialize_capture_session_opens_a_directory_of_its_own() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.settings.write().await.raw_frame_saving = crate::server::state::RawFrameSaving {
            stacking: true,
            ..Default::default()
        };
        state
            .disk_writer
            .start_session(WritingSessionType::IndividualFrames, "-live")
            .unwrap();
        let stale = state.disk_writer.session_dir().unwrap();

        initialize_capture_session(&state, None).await.unwrap();

        let opened = state.disk_writer.session_dir().expect("a session");
        assert_ne!(opened, stale);
        assert_eq!(
            CaptureMode::from_session_dir_name(&state.disk_writer.session_name().unwrap()),
            Some(CaptureMode::Stacking)
        );
    }

    /// The `stacked_count == 0` gate must survive the switch from a lifetime
    /// count to a windowed one — a session that has stacked at least one
    /// frame should never auto-stop on a rejection burst.
    #[tokio::test]
    async fn should_stop_on_errors_false_once_a_frame_has_stacked() {
        let (state, _disk_writer) = AppState::new_for_testing();
        {
            let mut session = state.session.write().await;
            let now = Instant::now();
            for i in 0..REJECTION_RATE_THRESHOLD {
                session.record_rejection(now + Duration::from_millis(i as u64));
            }
            session.stacked_count = 1;
        }
        assert!(!should_stop_on_errors(&state).await);
    }

    /// Regression guard: the stacked-PNG export path must apply the full tone-curve
    /// stretch, not just background/black-point subtraction. `process_preview_frame`
    /// defers the stretch for the live-view fused encoders; if `render_stacked_png`
    /// were ever routed through it without also applying the returned `StretchResult`
    /// (which `frame_to_rgb8_downsampled`'s row tail does), this would fail — the pixel
    /// would stay near its dim linear input instead of landing near the auto-stretch
    /// target background.
    #[test]
    fn render_stacked_png_applies_the_stretch() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = true;
        settings.background_subtraction = false;

        // A perfectly uniform frame makes the black-point solver degenerate (median
        // equals every pixel, sigma is ~0), so black-point subtraction alone nearly
        // cancels it out regardless of whether the tone curve ever runs — that would
        // pass even against the buggy code by accident. Inject small per-pixel noise,
        // matching `render::autostretch::tests::test_auto_stretch_frame_end_to_end`,
        // so the solver has real statistics and the tone curve has actual work to do.
        let background = 0.02;
        let mut data = vec![0.0f32; 32 * 32 * 3];
        let mut seed: u32 = 54321;
        let plane = 32 * 32;
        for i in 0..(32 * 32) {
            for c in 0..3 {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.005;
                data[c * plane + i] = background + noise;
            }
        }
        let frame = crate::frame::Frame::from_f32_vec(data, 32, 32, 3).unwrap();

        let (rgb8, width, _height) = render_stacked_png(frame, &settings).unwrap();

        // A real auto-stretch targets a background around ~0.05-0.15 (see
        // `AutoStretchConfig::from_profile`); a ~0.02 input must end up well above
        // its original value once the tone curve is actually applied to the bytes that
        // get written to disk, not just prepared and discarded.
        let idx = (16 * width as usize + 16) * 3;
        let stretched = rgb8[idx] as f32 / 255.0;
        assert!(
            stretched > 0.07,
            "stacked PNG frame was not stretched: pixel stayed at {stretched} (background was ~{background})"
        );
    }

    /// Regression guard for the bug `render_stacked_png` replaced: routing the export
    /// through `RenderPipeline::process` directly compiled and passed the stretch test
    /// above, but `RenderPipelineConfig::denoise` is not one of that pipeline's stages —
    /// it only means anything to the streaming encoders (see AGENTS.md's *Spatial
    /// denoising*) — so the saved file carried none of the noise reduction the operator
    /// had been looking at live. A single-pixel assertion can't catch a silently-skipped
    /// stage; comparing denoise-on against denoise-off on the same noisy input can.
    #[test]
    fn render_stacked_png_applies_denoising() {
        fn noisy_frame() -> crate::frame::Frame {
            let (w, h) = (64, 64);
            let mut data = vec![0.0f32; w * h * 3];
            let mut seed: u32 = 98765;
            let plane = w * h;
            for i in 0..(w * h) {
                for c in 0..3 {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.1;
                    data[c * plane + i] = (0.2 + noise).clamp(0.0, 1.0);
                }
            }
            crate::frame::Frame::from_f32_vec(data, w, h, 3).unwrap()
        }

        fn byte_sigma(rgb8: &[u8]) -> f64 {
            let n = rgb8.len() as f64;
            let mean = rgb8.iter().map(|&v| v as f64).sum::<f64>() / n;
            (rgb8.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / n).sqrt()
        }

        let mut settings = CaptureSettings::default();
        settings.auto_stretch = false;
        settings.background_subtraction = false;

        settings.denoise.chroma = true;
        settings.denoise.luma = true;
        let (denoised, _, _) = render_stacked_png(noisy_frame(), &settings).unwrap();

        settings.denoise.chroma = false;
        settings.denoise.luma = false;
        let (plain, _, _) = render_stacked_png(noisy_frame(), &settings).unwrap();

        assert!(
            byte_sigma(&denoised) < byte_sigma(&plain) * 0.9,
            "denoise settings had no measurable effect on the saved PNG bytes \
             (denoised sigma {:.3}, plain sigma {:.3})",
            byte_sigma(&denoised),
            byte_sigma(&plain)
        );
    }
}

/// Fully render a stacked frame for PNG export: the exact interleaved RGB8 bytes a
/// live viewer at the "Original" tier would see (background, stretch, saturation,
/// contrast, spatial denoise, 8-bit quantization). Routes through
/// `process_preview_frame` and `frame_to_rgb8_downsampled` — the render task's own
/// per-frame calls, bounding box left unbounded — rather than calling
/// `RenderPipeline::process` directly, which only knows the four *pipeline* stages;
/// denoise and pedestal/dither live only in the streaming encoders (see AGENTS.md's
/// *Spatial denoising*), so the direct path silently produced a PNG missing whatever
/// noise reduction was visible live. Runs once per session, not per frame, so the
/// plain (non-scratch-reusing) conversion is the right trade.
fn render_stacked_png(
    mut frame: Frame,
    settings: &CaptureSettings,
) -> crate::error::Result<(Vec<u8>, u32, u32)> {
    use super::pipeline::process_preview_frame;
    use crate::error::StackError;
    use crate::server::encoding::frame_to_rgb8_downsampled;
    use crate::server::state::RenderReadyFrame;

    let (pipeline_config, stretch_result) = process_preview_frame(&mut frame, settings)?;
    let ready_frame = RenderReadyFrame {
        linear_frame: Arc::new(frame),
        pipeline_config,
        stretch_result,
    };

    frame_to_rgb8_downsampled(&ready_frame, u32::MAX, u32::MAX)
        .map_err(StackError::InvalidConfiguration)
}

/// Save stacked result if stacking was enabled and we have frames
pub async fn save_stacked_result(
    state: &AppState,
    last_processed_frame: Option<Frame>,
    camera_info: &ConnectedCameraInfo,
) {
    use crate::fits::FitsMetadata;
    use chrono::Utc;

    let settings: RwLockReadGuard<'_, CaptureSettings> = state.settings.read().await;
    if !settings.saves_stacked_image() {
        return;
    }

    let session: RwLockReadGuard<'_, CaptureSession> = state.session.read().await;
    let stacked_count = session.stacked_count;
    drop(session);

    if stacked_count == 0 {
        return;
    }

    if let Some(stacked_frame) = last_processed_frame {
        let mut fits_frame = stacked_frame.clone();

        // Apply background subtraction to FITS if enabled
        if settings.background_subtraction {
            use super::pipeline::get_render_pipeline_config;
            use crate::render::RenderPipeline;

            let pipeline_config = get_render_pipeline_config(&settings, true);
            let pipeline = RenderPipeline::new(pipeline_config);
            if let Err(e) = pipeline.process(&mut fits_frame) {
                warn!(error = %e, "Failed to apply background subtraction to FITS");
            }
        }

        let mut metadata = FitsMetadata::new()
            .with_exposure_us(settings.exposure_us)
            .with_gain(settings.gain)
            .with_camera(&camera_info.info.name)
            .with_stacked_frames(stacked_count)
            .with_date_obs(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string());

        if camera_info.info.has_cooler {
            if let Some(set_temp) = settings.target_temp_c {
                metadata = metadata.with_set_temp(set_temp);
            }
            if let Some(status) = state.get_camera_status(&camera_info.info.name).await {
                metadata = metadata.with_temperature(status.temperature_c);
            }
        }

        if let Err(e) = state
            .disk_writer
            .queue_stacked_frame(Arc::new(fits_frame), metadata)
        {
            warn!(error = %e, "Failed to queue stacked FITS frame for saving");
        }

        match render_stacked_png(stacked_frame, &settings) {
            Ok((rgb8, width, height)) => {
                if let Err(e) = state.disk_writer.queue_stacked_png(
                    Arc::new(rgb8),
                    width,
                    height,
                    stacked_count,
                ) {
                    warn!(error = %e, "Failed to queue stretched PNG for saving");
                }
            }
            Err(e) => warn!(error = %e, "Failed to process frame for PNG output"),
        }
    }
}
