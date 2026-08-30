use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::RwLockReadGuard;
use tracing::{debug, warn};

use super::channel::CapturedFrame;
use crate::camera::RawFrame;
use crate::disk_writer::WritingSessionType;
use crate::frame::Frame;
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CaptureSession, CaptureSettings, ConnectedCameraInfo};
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
    rt: tokio::runtime::Handle,
) {
    debug!("Storage task started");

    let mut queue_warning_active = false;
    let mut last_warning_time = std::time::Instant::now();

    while let Ok(msg) = storage_rx.recv() {
        let CapturedFrame {
            frame,
            frame_number,
            settings,
            camera_info,
        } = msg;

        // Only save if raw frame saving is still enabled
        let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
        if !settings.save_raw_frames || !is_stacking_mode || !state.disk_writer.is_enabled() {
            continue;
        }

        let (new_warning_active, new_last_time) = rt.block_on(save_frame_to_disk(
            &state,
            &frame,
            frame_number,
            &settings,
            &camera_info,
            queue_warning_active,
            last_warning_time,
        ));
        queue_warning_active = new_warning_active;
        last_warning_time = new_last_time;
    }

    if queue_warning_active {
        state.disk_writer.clear_queue_warning();
        let _ = state.events.send(ServerEvent::DiskWriterWarningCleared);
    }

    debug!("Storage task ended");
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
    let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
    let save_raw = settings.save_raw_frames;
    let save_stacked = settings.save_stacked_image;

    let save_enabled = is_stacking_mode && (save_raw || save_stacked);
    if save_enabled {
        state.disk_writer.set_enabled(true);

        // Map StackingType to WritingSessionType
        let session_type = match settings.stacking_type {
            StackingType::Planetary => WritingSessionType::VideoContainer,
            _ => WritingSessionType::IndividualFrames,
        };

        match resume_dir {
            Some(dir) => state
                .disk_writer
                .resume_session(dir, session_type)
                .map_err(|e| format!("Failed to reopen capture directory: {}", e))?,
            None => state
                .disk_writer
                .start_session(session_type)
                .map_err(|e| format!("Failed to create capture directory: {}", e))?,
        };
    }
    Ok(())
}

/// Save a frame to disk and handle queue warnings
pub async fn save_frame_to_disk(
    state: &AppState,
    frame: &Arc<RawFrame>,
    frame_number: u64,
    settings: &CaptureSettings,
    camera_info: &ConnectedCameraInfo,
    mut queue_warning_active: bool,
    mut last_warning_time: std::time::Instant,
) -> (bool, std::time::Instant) {
    use crate::disk_writer::QUEUE_WARNING_THRESHOLD;
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
        warn!(error = %e, frame_number = frame_number, "Failed to queue frame for saving");
    }

    let queue_depth = state.disk_writer.queue_depth();
    if queue_depth > QUEUE_WARNING_THRESHOLD {
        let now = std::time::Instant::now();
        if !queue_warning_active || now.duration_since(last_warning_time).as_secs() >= 2 {
            queue_warning_active = true;
            last_warning_time = now;
            let _ = state
                .events
                .send(ServerEvent::DiskWriterWarning { queue_depth });
        }
    } else if queue_depth <= QUEUE_WARNING_THRESHOLD && queue_warning_active {
        queue_warning_active = false;
        state.disk_writer.clear_queue_warning();
        let _ = state.events.send(ServerEvent::DiskWriterWarningCleared);
    }

    (queue_warning_active, last_warning_time)
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

/// Fully render a stacked frame for PNG export, producing the exact interleaved RGB8
/// bytes a live viewer at the "Original" resolution tier would see: background,
/// stretch, saturation, contrast, spatial denoise and 8-bit display quantization.
///
/// Routes through `process_preview_frame` and
/// `crate::server::encoding::frame_to_rgb8_downsampled` — the same two calls the render
/// task makes per frame for a connected client — with the bounding box left unbounded so
/// nothing downsamples. This used to call `RenderPipeline::process` directly instead,
/// which only knows the four *pipeline* stages (background, stretch, saturation,
/// contrast). Spatial denoise and the display pedestal/dither live only in the streaming
/// encoders (see AGENTS.md's *Spatial denoising* and *The f32 -> 8-bit boundary* — both
/// are deliberately not pipeline stages), so that path silently produced a PNG missing
/// whatever noise reduction the operator had been looking at live. Going through the
/// encoder is what picks those up instead of reimplementing them a second time here.
///
/// This runs once per stacking session, not per frame, so paying for the plain
/// (non-scratch-reusing) conversion is the right trade — see
/// `frame_to_rgb8_downsampled`'s own doc comment.
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
    if !settings.save_stacked_image || !settings.stacking || settings.wanderer_mode {
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
