use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use tokio::sync::RwLockReadGuard;
use tracing::{debug, info, warn};

use super::channel::CapturedFrame;
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

        queue_warning_active = rt.block_on(save_frame_to_disk(
            &state,
            &frame,
            frame_number,
            &settings,
            &camera_info,
            queue_warning_active,
        ));
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
pub async fn initialize_capture_session(state: &AppState) -> Result<(), String> {
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

        state
            .disk_writer
            .start_session(session_type)
            .map_err(|e| format!("Failed to create capture directory: {}", e))?;
    }
    Ok(())
}

/// Save a frame to disk and handle queue warnings
pub async fn save_frame_to_disk(
    state: &AppState,
    frame: &Frame,
    frame_number: u64,
    settings: &CaptureSettings,
    camera_info: &ConnectedCameraInfo,
    mut queue_warning_active: bool,
) -> bool {
    use crate::disk_writer::QUEUE_WARNING_THRESHOLD;
    use crate::fits::FitsMetadata;
    use chrono::Utc;

    let raw_frame = frame.clone();
    let metadata = FitsMetadata::new()
        .with_exposure_us(settings.exposure_us)
        .with_gain(settings.gain)
        .with_offset(settings.offset)
        .with_camera(&camera_info.info.name)
        .with_frame_number(frame_number)
        .with_binning(settings.bin)
        .with_date_obs(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string());

    if let Err(e) = state
        .disk_writer
        .queue_raw_frame(raw_frame, frame_number, metadata)
    {
        warn!(error = %e, frame_number = frame_number, "Failed to queue frame for saving");
    }

    let queue_depth = state.disk_writer.queue_depth();
    if queue_depth > QUEUE_WARNING_THRESHOLD && !queue_warning_active {
        queue_warning_active = true;
        let _ = state
            .events
            .send(ServerEvent::DiskWriterWarning { queue_depth });
    } else if queue_depth <= QUEUE_WARNING_THRESHOLD && queue_warning_active {
        queue_warning_active = false;
        state.disk_writer.clear_queue_warning();
        let _ = state.events.send(ServerEvent::DiskWriterWarningCleared);
    }

    queue_warning_active
}

/// Check if we should stop due to too many errors
pub async fn should_stop_on_errors(state: &AppState) -> bool {
    let session: RwLockReadGuard<'_, CaptureSession> = state.session.read().await;
    session.rejected_count > 10 && session.stacked_count == 0
}

/// Save stacked result if stacking was enabled and we have frames
pub async fn save_stacked_result(
    state: &AppState,
    last_processed_frame: Option<Frame>,
    camera_info: &ConnectedCameraInfo,
) {
    use super::pipeline::process_preview_frame;
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

        let metadata = FitsMetadata::new()
            .with_exposure_us(settings.exposure_us)
            .with_gain(settings.gain)
            .with_camera(&camera_info.info.name)
            .with_stacked_frames(stacked_count)
            .with_date_obs(Utc::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string());

        if let Err(e) = state.disk_writer.queue_stacked_frame(fits_frame, metadata) {
            warn!(error = %e, "Failed to queue stacked FITS frame for saving");
        }

        let mut stretched_frame = stacked_frame;
        if let Err(e) = process_preview_frame(&mut stretched_frame, &settings) {
            warn!(error = %e, "Failed to process frame for PNG output");
            return;
        }

        if let Err(e) = state
            .disk_writer
            .queue_stacked_png(stretched_frame, stacked_count)
        {
            warn!(error = %e, "Failed to queue stretched PNG for saving");
        }
    }
}
