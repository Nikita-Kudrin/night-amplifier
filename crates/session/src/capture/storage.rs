use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, warn};

use super::channel::{CapturedFrame, QueueDepth};
use super::drop_log::DropLog;
use night_amplifier_core::camera::RawFrame;
use night_amplifier_core::disk_writer::WritingSessionType;
use night_amplifier_core::frame::Frame;
use night_amplifier_core::plugins::Plugins;
use crate::events::ServerEvent;
use crate::state::{
    AppState, CaptureMode, CaptureSettings, ConnectedCameraInfo,
};
use night_amplifier_core::stacking::StackingType;

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

        save_frame_to_disk(&state, &frame, frame_number, &settings, &camera_info, &mut warnings);
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
    fn record_drop(&mut self, frame_number: u64, error: &night_amplifier_core::disk_writer::DiskWriterError) {
        // Rate-limited for the same reason the capture-side drop is: at the short
        // exposures raw-saving Live view allows, a disk that cannot keep up turns away
        // most frames, and a line each buries everything else in the log.
        if let Some(dropped) = self.drops.record() {
            warn!(error = %error, frame_number, dropped, "Frames dropped: could not be queued for saving");
        }
    }

    /// Tell the frontend where the queue stands, no more often than the interval.
    fn observe_depth(&mut self, state: &AppState, queue_depth: usize) {
        use night_amplifier_core::disk_writer::QUEUE_WARNING_THRESHOLD;

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
    state.roster.get(camera_id)
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
    let settings = state.settings.snapshot();
    state.disk_writer.set_enabled(settings.disk_writing_enabled());

    // A session can still be open here: a settings update lands between the capture
    // state flipping and this call, or a previous capture ended without one. Either way
    // this capture opens its own, so let go of the old one rather than letting
    // `ensure_session` adopt it later.
    state.disk_writer.abandon_session();
    if !settings.main_capture_saves() {
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

/// Keeps the disk writer in sync with settings changed after
/// `initialize_capture_session` ran: enabled flag, open session, and
/// name-matches-mode are one decision. Rolls the directory on a mode change —
/// Live view then Stacking without stopping is ordinary, and `ensure_session`
/// alone would leave stacked subs in a folder named `-live`. The caller passes the
/// settings and capture state it decided on, so all three agree.
pub async fn sync_disk_session(
    state: &AppState,
    settings: &CaptureSettings,
    capture_active: bool,
) {
    state.disk_writer.set_enabled(settings.disk_writing_enabled());

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

    // Guide saving alone keeps the writer enabled, but files into the guide's own session.
    if settings.main_capture_saves() {
        if let Err(e) = state
            .disk_writer
            .ensure_session(session_type, mode.session_dir_suffix())
        {
            warn!(error = %e, "Could not open a capture directory for saving");
            state.send_error(format!("Saving is on but no folder could be created: {}", e));
        }
    }

    // A reconnect rejoins the directory recorded in the resume plan. Left stale, it would
    // rejoin the folder this call just abandoned.
    let dir = state.disk_writer.session_dir();
    state.resume.edit_plan(|plan| plan.disk_session_dir = dir);
}

/// Build the FITS header for one raw frame.
///
/// Takes the camera's own hardware profile rather than the flat settings: with a guide
/// camera connected the two cameras have different exposures and gains, and a guide sub
/// stamped with the imaging camera's `EXPTIME` is a file that lies about itself.
pub(crate) fn raw_frame_metadata(
    state: &AppState,
    profile: &crate::state::CameraCaptureProfile,
    camera_info: &ConnectedCameraInfo,
    frame_number: u64,
) -> night_amplifier_core::fits::FitsMetadata {
    use night_amplifier_core::fits::FitsMetadata;
    use chrono::Utc;

    let mut metadata = FitsMetadata::new()
        .with_exposure_us(profile.exposure_us)
        .with_gain(profile.gain)
        .with_offset(profile.offset)
        .with_camera(&camera_info.info.name)
        .with_frame_number(frame_number)
        .with_binning(profile.bin)
        .with_date_obs(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string());

    if camera_info.info.has_cooler {
        if let Some(set_temp) = profile.target_temp_c {
            metadata = metadata.with_set_temp(set_temp);
        }
        if let Some(status) = state.get_camera_status(&camera_info.info.name) {
            metadata = metadata.with_temperature(status.temperature_c);
        }
    }

    metadata
}

/// The FITS header for a guide-camera sub.
pub(crate) fn guide_frame_metadata(
    state: &AppState,
    settings: &CaptureSettings,
    camera_info: &ConnectedCameraInfo,
    frame_number: u64,
) -> night_amplifier_core::fits::FitsMetadata {
    raw_frame_metadata(state, &settings.guide_camera, camera_info, frame_number)
}

/// Save a frame to disk and handle queue warnings
fn save_frame_to_disk(
    state: &AppState,
    frame: &Arc<RawFrame>,
    frame_number: u64,
    settings: &CaptureSettings,
    camera_info: &ConnectedCameraInfo,
    warnings: &mut StorageWarnings,
) {
    let raw_frame = Arc::clone(frame);
    let metadata =
        raw_frame_metadata(state, &settings.main_camera_profile(), camera_info, frame_number);

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
/// Uses a sliding window (`SessionStats::record_failure`) rather than the
/// lifetime-cumulative `rejected_count`, so a camera that failed sporadically
/// across an otherwise-healthy multi-hour session never trips this — only a
/// real, currently-active failure burst does (e.g. ~10 capture failures within
/// a second, consistent with a genuine disconnect rather than a hiccup).
pub fn should_stop_on_errors(state: &AppState) -> bool {
    state.stats.failing() && state.stats.counts().stacked == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::REJECTION_RATE_THRESHOLD;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn should_stop_on_errors_false_when_healthy() {
        let (state, _disk_writer) = AppState::new_for_testing();
        assert!(!should_stop_on_errors(&state));
    }

    #[tokio::test]
    async fn should_stop_on_errors_true_on_burst_with_no_stacked_frames() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let now = Instant::now();
        for i in 0..REJECTION_RATE_THRESHOLD {
            state.stats.record_failure(now + Duration::from_millis(i as u64));
        }
        assert!(should_stop_on_errors(&state));
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
        state.settings.update(|s| {
            s.raw_frame_saving = crate::state::RawFrameSaving {
                stacking: true,
                ..Default::default()
            }
        });
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

    /// The 2026-09-20 field settings: guide subs and the stack saved, raw main subs not.
    /// Guide saving keeps the writer enabled, but the guide files into a session of its
    /// own; counting it opened an empty `-live` folder on every Live view start (23 that
    /// night). Stacking does name a session, for the stack's file, and still no folder.
    #[tokio::test]
    async fn guide_saving_opens_no_main_session_in_live_view() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.settings.update(|s| {
            s.stacking = false;
            s.save_stacked_image = true;
            s.raw_frame_saving = crate::state::RawFrameSaving {
                guide: true,
                ..Default::default()
            }
        });

        initialize_capture_session(&state, None).await.unwrap();
        assert!(state.disk_writer.is_enabled(), "the guide's frames need the writer on");
        assert_eq!(state.disk_writer.session_dir(), None);

        state.settings.update(|s| s.stacking = true);
        sync_disk_session(&state, &state.settings.snapshot(), true).await;
        let dir = state.disk_writer.session_dir().expect("the stack is named after a session");
        assert!(!dir.exists(), "{dir:?} was created with no raw frame to hold");
    }

    /// A mode roll the main capture does not save in must not leave the resume plan on
    /// the folder it rolled away from, or a reconnect would rejoin it.
    #[tokio::test]
    async fn a_roll_into_a_mode_that_saves_nothing_clears_the_resume_folder() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.settings.update(|s| {
            s.stacking = true;
            s.raw_frame_saving = crate::state::RawFrameSaving {
                stacking: true,
                ..Default::default()
            }
        });
        initialize_capture_session(&state, None).await.unwrap();
        state.resume.record(crate::state::SessionResumePlan {
            camera_id: "main".to_string(),
            settings: (*state.settings.snapshot()).clone(),
            disk_session_dir: state.disk_writer.session_dir(),
            next_frame: 1,
        });
        assert!(state.resume.plan().unwrap().disk_session_dir.is_some());

        state.settings.update(|s| s.stacking = false);
        sync_disk_session(&state, &state.settings.snapshot(), true).await;

        assert_eq!(state.disk_writer.session_dir(), None);
        assert_eq!(state.resume.plan().unwrap().disk_session_dir, None);
    }

    /// Switching to Stacking mid-capture names a `-stacking` session at once; stopped
    /// before a sub is saved, it must leave no folder (two of the 2026-09-20 empties).
    #[tokio::test]
    async fn a_stacking_roll_stopped_before_a_sub_leaves_no_folder() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.settings.update(|s| {
            s.stacking = false;
            s.raw_frame_saving = crate::state::RawFrameSaving {
                stacking: true,
                ..Default::default()
            }
        });
        initialize_capture_session(&state, None).await.unwrap();
        assert_eq!(state.disk_writer.session_dir(), None, "Live view saves nothing");

        state.settings.update(|s| s.stacking = true);
        sync_disk_session(&state, &state.settings.snapshot(), true).await;
        let dir = state.disk_writer.session_dir().expect("a -stacking session");
        assert_eq!(
            CaptureMode::from_session_dir_name(&state.disk_writer.session_name().unwrap()),
            Some(CaptureMode::Stacking)
        );

        state.disk_writer.end_session();
        assert!(!dir.exists(), "{dir:?} was left behind with nothing in it");
    }

    /// The worker's failure port reaches the observer: a disk that stops taking frames
    /// mid-session was only ever logged, a line per frame.
    #[tokio::test]
    async fn a_failed_write_reaches_the_observer() {
        let (state, disk_writer) = AppState::new_for_testing();
        let mut events = state.events.subscribe();
        let writer_task = std::thread::spawn(move || disk_writer.run());
        let raw = state.disk_writer.raw_dir();
        // A folder under a plain file cannot be created, by root either.
        std::fs::write(raw.join("blocker"), b"").unwrap();
        let doomed = night_amplifier_core::disk_writer::OpenSession {
            dir: raw.join("blocker").join("doomed-live"),
            session_type: WritingSessionType::IndividualFrames,
        };
        for n in 1..=3 {
            let frame = Arc::new(RawFrame {
                data: night_amplifier_core::camera::BufferPool::new().get(8 * 8 * 2),
                width: 8,
                height: 8,
                format: night_amplifier_core::camera::ImageFormat::Raw16,
            });
            state
                .disk_writer
                .queue_raw_frame_in(
                    Some(doomed.clone()),
                    frame,
                    n,
                    night_amplifier_core::fits::FitsMetadata::new(),
                    night_amplifier_core::camera::SensorType::Mono,
                    None,
                )
                .unwrap();
        }
        drop(state);
        tokio::task::spawn_blocking(move || writer_task.join().unwrap()).await.unwrap();
        let _ = std::fs::remove_dir_all(raw.parent().unwrap());

        let mut errors = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let ServerEvent::Error { message } = event {
                errors.push(message);
            }
        }
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("not being saved"), "{errors:?}");
    }

    /// The `stacked_count == 0` gate must survive the switch from a lifetime
    /// count to a windowed one — a session that has stacked at least one
    /// frame should never auto-stop on a rejection burst.
    #[tokio::test]
    async fn should_stop_on_errors_false_once_a_frame_has_stacked() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let now = Instant::now();
        for i in 0..REJECTION_RATE_THRESHOLD {
            state.stats.record_failure(now + Duration::from_millis(i as u64));
        }
        state.stats.frame_captured(true, true);
        assert!(!should_stop_on_errors(&state));
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
        let frame = night_amplifier_core::frame::Frame::from_f32_vec(data, 32, 32, 3).unwrap();

        let (rgb8, width, _height) = render_stacked_png(frame, &settings, &Plugins::none(), 1, None).unwrap();

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

    /// A left-to-right sky gradient with mild noise: the thing background removal exists
    /// to flatten.
    fn gradient_frame() -> night_amplifier_core::frame::Frame {
        let (w, h) = (256, 256);
        let plane = w * h;
        let mut data = vec![0.0f32; plane * 3];
        let mut seed: u32 = 13579;
        for y in 0..h {
            for x in 0..w {
                let sky = 0.05 + 0.15 * x as f32 / (w - 1) as f32;
                for c in 0..3 {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.004;
                    data[c * plane + y * w + x] = sky + noise;
                }
            }
        }
        night_amplifier_core::frame::Frame::from_f32_vec(data, w, h, 3).unwrap()
    }

    /// Mean byte of the leftmost minus the rightmost 16 columns, all channels.
    fn edge_difference(rgb8: &[u8], width: usize, height: usize) -> f64 {
        let band_mean = |x0: usize| {
            let mut sum = 0u64;
            for y in 0..height {
                for x in x0..x0 + 16 {
                    let i = (y * width + x) * 3;
                    sum += rgb8[i..i + 3].iter().map(|&v| v as u64).sum::<u64>();
                }
            }
            sum as f64 / (height * 16 * 3) as f64
        };
        band_mean(0) - band_mean(width - 16)
    }

    /// Both tests above switch background subtraction off, so nothing guarded that the
    /// saved PNG loses the gradient the live view removes. Stretch stays off so the
    /// ramp is compared in linear bytes (~35 levels edge to edge without removal). The
    /// default profile's preset has `aggressiveness: 0.7`, so ~30 % of the ramp is meant
    /// to stay: 12.2 levels measured, against 34.6 with removal off.
    #[test]
    fn render_stacked_png_removes_the_background_gradient() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = false;
        settings.denoise.chroma = false;
        settings.denoise.luma_strength = 0.0;

        settings.background_subtraction = false;
        let (kept, w, h) = render_stacked_png(gradient_frame(), &settings, &Plugins::none(), 1, None).unwrap();
        settings.background_subtraction = true;
        let (removed, _, _) = render_stacked_png(gradient_frame(), &settings, &Plugins::none(), 1, None).unwrap();

        let kept = edge_difference(&kept, w as usize, h as usize).abs();
        let removed = edge_difference(&removed, w as usize, h as usize).abs();
        assert!(kept > 30.0, "fixture gradient too weak to test against: {kept:.1} levels");
        assert!(
            removed < kept * 0.5,
            "background removal did not reach the saved PNG: edge difference {removed:.1} \
             levels with it on, {kept:.1} with it off"
        );
    }

    /// The saved PNG is byte-for-byte what the render task streams for the same stack at
    /// sensor resolution, with every display stage on: background neutralization and
    /// removal, SCNR, stretch, saturation, contrast, eyepiece darkening, black floor,
    /// dither, denoise — and the stack's coverage map, which both sides are handed. A stage
    /// added to the live path but not the export fails here; the map only changes pixels
    /// with the filters registered, so Pro's `denoise_encoding_tests` carries that half.
    #[test]
    fn render_stacked_png_matches_the_live_render_with_every_stage_on() {
        use night_amplifier_core::render::denoise::DenoiseScratch;
        use night_amplifier_core::render::display::frame_to_rgb8_downsampled_with;
        use night_amplifier_core::render::display::RenderReadyFrame;
        use crate::capture::analysis::{AnalysisContext, PreviewAnalysis};
        use crate::capture::pipeline::process_preview_frame_with_analysis;

        // A drifting session: the left quarter holds half the stack.
        let cells = 256 / night_amplifier_core::frame::NOISE_REDUCTION;
        let cover = (0..cells * cells).map(|i| if i % cells < cells / 4 { 0.5 } else { 1.0 }).collect();
        let coverage = night_amplifier_core::frame::NoiseField::coverage_only(cover, cells, cells, 3, 256, 256).unwrap();

        let mut settings = CaptureSettings::default();
        settings.background_subtraction = true;
        settings.auto_stretch = true;
        settings.saturation_boost = true;
        settings.eyepiece.intensity = 0.7;
        settings.eyepiece.black_floor = -0.03;
        settings.eyepiece.dither = true;
        settings.denoise.chroma = true;
        settings.denoise.luma_strength = 1.0;

        let (exported, _, _) =
            render_stacked_png(gradient_frame(), &settings, &Plugins::none(), 40, Some(coverage.clone())).unwrap();

        let mut live = gradient_frame();
        let rendered = process_preview_frame_with_analysis(
            &mut live,
            &settings,
            &Plugins::none(),
            AnalysisContext {
                showing_stack: true,
                stack_depth: 40,
            },
            &mut PreviewAnalysis::new(),
        )
        .unwrap();
        let ready = RenderReadyFrame {
            linear_frame: Arc::new(live),
            noise: Some(Arc::new(coverage)),
            pipeline_config: rendered.pipeline_config,
            stretch_result: rendered.stretch_result,
        };
        let (streamed, _, _) = frame_to_rgb8_downsampled_with(
            &ready,
            u32::MAX,
            u32::MAX,
            &mut DenoiseScratch::default(),
        )
        .unwrap();

        assert!(exported == streamed, "saved PNG bytes differ from the live render");
    }

    /// The depth is a render input, not a caption. This is the half the parity test
    /// above cannot check — it hands the same number to both sides, so it would pass
    /// just as happily if the export took its depth from the session counters and they
    /// disagreed with the stack by a frame or by a reset.
    #[test]
    fn the_saved_png_is_tone_curved_by_the_depth_it_is_given() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = true;
        settings.denoise.chroma = false;
        settings.denoise.luma_strength = 0.0;

        let (shallow, _, _) = render_stacked_png(gradient_frame(), &settings, &Plugins::none(), 1, None).unwrap();
        let (deep, _, _) = render_stacked_png(gradient_frame(), &settings, &Plugins::none(), 64, None).unwrap();

        assert!(
            shallow != deep,
            "the same stack rendered identically at 1 and 64 frames — the export is not              passing its depth to the stretch"
        );
    }
}

/// Fully renders a stacked frame for PNG export: the exact RGB8 bytes a live
/// viewer would see, via `process_preview_frame_with_analysis` +
/// `frame_to_rgb8_downsampled` (the render task's own calls) rather than
/// `RenderPipeline::process` directly, which skips denoise/pedestal-dither (those
/// live only in the streaming encoders, AGENTS.md's *Spatial denoising*) — the
/// direct path once silently dropped noise reduction. `pub` so Pro can test that
/// regression directly. `coverage`: without it a thin border saved grainier than
/// observed live (5.7% of bytes differed over a thinly covered quarter).
pub fn render_stacked_png(
    mut frame: Frame,
    settings: &CaptureSettings,
    plugins: &Plugins,
    stack_depth: u32,
    coverage: Option<night_amplifier_core::frame::NoiseField>,
) -> night_amplifier_core::error::Result<(Vec<u8>, u32, u32)> {
    use super::analysis::{AnalysisContext, PreviewAnalysis};
    use super::pipeline::process_preview_frame_with_analysis;
    use night_amplifier_core::error::StackError;
    use night_amplifier_core::render::display::frame_to_rgb8_downsampled;
    use night_amplifier_core::render::display::RenderReadyFrame;

    // The depth is part of the render, not bookkeeping: the stretch spends it on how
    // calm the sky is (`render::autostretch::depth_grain_gain`), so an export that left
    // it at the default would tone-curve the same stack differently from the live view.
    let rendered = process_preview_frame_with_analysis(
        &mut frame,
        settings,
        plugins,
        AnalysisContext {
            showing_stack: true,
            stack_depth,
        },
        &mut PreviewAnalysis::new(),
    )?;
    let ready_frame = RenderReadyFrame {
        linear_frame: Arc::new(frame),
        pipeline_config: rendered.pipeline_config,
        stretch_result: rendered.stretch_result,
        noise: coverage.map(Arc::new),
    };

    frame_to_rgb8_downsampled(&ready_frame, u32::MAX, u32::MAX)
        .map_err(StackError::InvalidConfiguration)
}

/// Save stacked result if stacking was enabled and we have frames.
///
/// `coverage` travels with the frame for the PNG — see [`render_stacked_png`].
pub fn save_stacked_result(
    state: &AppState,
    last_processed_frame: Option<Frame>,
    stack_depth: u32,
    coverage: Option<night_amplifier_core::frame::NoiseField>,
    camera_info: &ConnectedCameraInfo,
) {
    use night_amplifier_core::fits::FitsMetadata;
    use chrono::Utc;

    let settings = state.settings.snapshot();
    if !settings.saves_stacked_image() {
        return;
    }

    let stacked_count = state.stats.counts().stacked;

    if stacked_count == 0 {
        return;
    }

    if let Some(stacked_frame) = last_processed_frame {
        let mut fits_frame = stacked_frame.clone();

        // Apply background subtraction to FITS if enabled
        if settings.background_subtraction {
            use super::pipeline::get_render_pipeline_config;
            use night_amplifier_core::render::RenderPipeline;

            let pipeline_config = get_render_pipeline_config(&settings, &state.plugins, true);
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
            if let Some(status) = state.get_camera_status(&camera_info.info.name) {
                metadata = metadata.with_temperature(status.temperature_c);
            }
        }

        if let Err(e) = state
            .disk_writer
            .queue_stacked_frame(Arc::new(fits_frame), metadata)
        {
            warn!(error = %e, "Failed to queue stacked FITS frame for saving");
        }

        match render_stacked_png(stacked_frame, &settings, &state.plugins, stack_depth, coverage) {
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
