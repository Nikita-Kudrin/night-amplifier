//! Decoupled asynchronous capture pipeline
//!
//! The capture loop is decomposed into four independent tasks connected by
//! bounded MPSC channels:
//!
//! - **CaptureTask** (dedicated thread) — acquires frames from the camera
//! - **StorageTask** (dedicated thread) — saves raw frames to disk
//! - **StackingTask** (dedicated thread) — registration + accumulation
//! - **RenderTask** (dedicated thread) — preview rendering + encoding
//!
//! `Arc<Frame>` provides zero-copy frame sharing between channels.
//! Channel capacities are calculated from a 2 GB memory budget divided by
//! the actual frame size.
//!
//! Each spawned OS thread receives a `tokio::runtime::Handle` captured from
//! the async orchestrator, so it can call `handle.block_on()` for async
//! state access and `handle.spawn()` for fire-and-forget async work.

pub mod channel;
mod context;
mod pipeline;
mod render_task;
mod solving;
mod stacking_task;
mod storage;

use render_task::run_render_task;
use stacking_task::run_stacking_task;

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::encoding::encode_rgb8_lz4;
use super::state::{AppState, CaptureState, StackingType};

use crate::frame::Frame;
use crate::stacking::CometContext;
pub use context::{PlanetaryStackingContext, StackingContext};

use channel::{max_queue_capacity, CapturedFrame, StackedFrame};

/// Cadence for polling cooled-camera status from the capture thread.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Override the capture format with the best raw format advertised by the
/// camera (`Raw16` preferred, `Raw8` as fallback). Leaves the config untouched
/// if neither is advertised, letting the provider surface a clear SDK error.
fn apply_best_raw_format(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if let Some(format) = crate::camera::ImageFormat::best_raw_format(&info.supported_formats) {
        if config.format != format {
            debug!(
                camera = %camera_name,
                selected = ?format,
                requested = ?config.format,
                supported = ?info.supported_formats,
                "Adjusted capture format to best available raw format"
            );
            config.format = format;
        }
    } else {
        warn!(
            camera = %camera_name,
            supported = ?info.supported_formats,
            "Camera advertises neither Raw16 nor Raw8 — capture may fail"
        );
    }
}

/// The main capture orchestrator.
///
/// Takes the long-lived camera handle from `AppState` (opened at connect
/// time), creates bounded channels, spawns four independent worker threads,
/// and awaits their completion. On shutdown, returns the handle to
/// `AppState` so the monitor thread can resume — unless the capture task
/// lost the handle due to a hard error, in which case the lifecycle layer
/// finalizes a disconnect.
pub async fn run_capture_loop(state: Arc<AppState>, camera_id: String) {
    use crate::server::camera_session::lifecycle;

    // Transition to capturing state
    state.set_capture_state(CaptureState::Capturing).await;

    debug!(camera_id = %camera_id, "Capture pipeline starting");

    // Capture the tokio runtime handle — this will be passed to all spawned
    // OS threads so they can call handle.block_on() and handle.spawn().
    let rt_handle = tokio::runtime::Handle::current();

    // Get camera info for opening
    let camera_info = match storage::get_camera_info(&state, &camera_id).await {
        Some(info) => info,
        None => {
            error!(camera_id = %camera_id, "Camera not found in capture loop");
            state.send_error("Camera not found".to_string());
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Initialize capture session
    if let Err(e) = storage::initialize_capture_session(&state).await {
        error!(error = %e, "Failed to initialize capture session");
        state.send_error(e);
        state.set_capture_state(CaptureState::Idle).await;
        return;
    }

    // Take the handle from AppState (held by camera_session since connect).
    // This cancels any in-progress warmup and flips the phase to Capturing.
    let camera_name = camera_info.info.name.clone();
    let mut camera = match lifecycle::take_for_capture(&state, &camera_name).await {
        Ok(cam) => {
            debug!(
                camera_id = %camera_id,
                provider = %camera_info.provider,
                "Camera handle taken for capture"
            );
            cam
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Failed to take camera handle for capture");
            state.send_error(format!("Failed to take camera handle: {}", e));
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Register active camera cancel token in state
    state
        .set_active_camera_token(camera.cancel_token())
        .await;

    // Capture a probe frame to determine dimensions and channel capacities
    let settings = state.settings.read().await.clone();
    let mut capture_config = settings.to_capture_config();
    apply_best_raw_format(&mut capture_config, &camera_info.info, &camera_name);
    let probe_frame = match camera.capture(&capture_config) {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "Failed to capture probe frame for pipeline setup");
            state.send_error(format!("Failed to capture initial frame: {}", e));
            state.clear_active_camera_token().await;
            lifecycle::return_from_capture(&state, &camera_name, Some(camera)).await;
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    let frame_memory = probe_frame.memory_size();
    let channel_capacity = max_queue_capacity(frame_memory);
    info!(
        frame_memory_bytes = frame_memory,
        channel_capacity = channel_capacity,
        width = probe_frame.width(),
        height = probe_frame.height(),
        channels = probe_frame.channels(),
        "Pipeline channel capacity calculated"
    );

    // Create bounded channels
    let (stacking_tx, stacking_rx) = mpsc::sync_channel::<CapturedFrame>(channel_capacity);
    let (storage_tx, storage_rx) = mpsc::sync_channel::<CapturedFrame>(channel_capacity);
    let (render_tx, render_rx) = mpsc::sync_channel::<StackedFrame>(channel_capacity);

    // Send the probe frame as the first frame through the pipeline
    let first_frame = Arc::new(probe_frame);
    let first_msg = CapturedFrame {
        frame: Arc::clone(&first_frame),
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let first_msg_storage = CapturedFrame {
        frame: first_frame,
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let _ = stacking_tx.send(first_msg);
    let _ = storage_tx.send(first_msg_storage);

    // Spawn worker threads — each gets a clone of the tokio Handle
    let state_capture = Arc::clone(&state);
    let state_stacking = Arc::clone(&state);
    let state_render = Arc::clone(&state);
    let state_storage = Arc::clone(&state);

    let rt_capture = rt_handle.clone();
    let rt_stacking = rt_handle.clone();
    let rt_render = rt_handle.clone();
    let rt_storage = rt_handle.clone();

    let capture_handle = std::thread::Builder::new()
        .name("capture-task".into())
        .spawn(move || {
            run_capture_task(state_capture, camera, stacking_tx, storage_tx, rt_capture)
        })
        .expect("Failed to spawn capture thread");

    let stacking_handle = std::thread::Builder::new()
        .name("stacking-task".into())
        .spawn(move || {
            run_stacking_task(state_stacking, stacking_rx, render_tx, rt_stacking);
        })
        .expect("Failed to spawn stacking thread");

    let render_handle = std::thread::Builder::new()
        .name("render-task".into())
        .spawn(move || {
            run_render_task(state_render, render_rx, rt_render);
        })
        .expect("Failed to spawn render thread");

    let storage_handle = std::thread::Builder::new()
        .name("storage-task".into())
        .spawn(move || {
            storage::run_storage_task(state_storage, storage_rx, rt_storage);
        })
        .expect("Failed to spawn storage thread");

    // Wait for all threads to complete (blocking join wrapped in spawn_blocking
    // to avoid blocking the tokio runtime). The capture task returns the
    // handle so it can be returned to the session; downstream threads
    // produce no output.
    let returned_camera = tokio::task::spawn_blocking(move || {
        let cam = match capture_handle.join() {
            Ok(cam) => cam,
            Err(e) => {
                error!("Capture thread panicked: {:?}", e);
                None
            }
        };
        // Once capture is done (senders dropped), downstream threads will drain and exit
        if let Err(e) = storage_handle.join() {
            error!("Storage thread panicked: {:?}", e);
        }
        if let Err(e) = stacking_handle.join() {
            error!("Stacking thread panicked: {:?}", e);
        }
        if let Err(e) = render_handle.join() {
            error!("Render thread panicked: {:?}", e);
        }
        cam
    })
    .await
    .unwrap_or(None);

    // End capture session
    state.disk_writer.end_session();

    info!(camera_id = %camera_id, "Capture pipeline ended");

    // Return the camera handle to the session (or finalize disconnect if lost).
    state.clear_active_camera_token().await;
    lifecycle::return_from_capture(&state, &camera_name, returned_camera).await;
    state.set_capture_state(CaptureState::Idle).await;
}

// =============================================================================
// CaptureTask
// =============================================================================

/// Camera capture loop running on a dedicated OS thread.
///
/// Acquires frames from the camera and sends them (as `Arc<Frame>`) to the
/// stacking and storage channels. Uses `try_send` on the stacking channel
/// to avoid blocking when the pipeline can't keep up — frames are dropped
/// and counted. The storage channel uses `try_send` independently.
fn run_capture_task(
    state: Arc<AppState>,
    mut camera: Box<dyn crate::camera::Camera>,
    stacking_tx: mpsc::SyncSender<CapturedFrame>,
    storage_tx: mpsc::SyncSender<CapturedFrame>,
    rt: tokio::runtime::Handle,
) -> Option<Box<dyn crate::camera::Camera>> {
    debug!("Capture task started");

    // Frame numbering continues from 1 (probe frame was #1)
    let mut frame_number: u64 = 1;
    let mut last_status_at = Instant::now()
        .checked_sub(STATUS_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut camera_ok = true;

    loop {
        if state.is_cancelled() {
            break;
        }

        // Read settings snapshot for this frame
        let settings = rt.block_on(state.settings.read()).clone();
        let mut capture_config = settings.to_capture_config();

        // Get camera info
        let camera_info = {
            let cameras = rt.block_on(state.cameras.read());
            cameras
                .values()
                .find(|c| c.info.name == camera.info().name)
                .cloned()
        };
        let camera_info = match camera_info {
            Some(info) => info,
            None => {
                warn!("Camera info not found, stopping capture");
                break;
            }
        };

        apply_best_raw_format(&mut capture_config, &camera_info.info, &camera.info().name);

        // Capture a frame (blocking FFI call)
        let capture_result = {
            let _span = tracing::info_span!(
                "camera_capture",
                frame_number = frame_number + 1,
                exposure_us = capture_config.exposure_us,
                gain = capture_config.gain,
                bin = capture_config.bin,
            )
            .entered();
            camera.capture(&capture_config)
        };
        let frame = match capture_result {
            Ok(f) => f,
            Err(e) => {
                if let crate::camera::CameraError::Cancelled = e {
                    debug!(
                        "Capture cancelled (likely due to settings update), starting next frame"
                    );
                    camera
                        .cancel_token()
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    continue;
                }

                // Hard disconnect errors invalidate the handle — don't return it.
                if let crate::camera::CameraError::Disconnected = e {
                    error!(error = %e, "Camera disconnected during capture");
                    state.send_error(format!("Camera disconnected: {}", e));
                    camera_ok = false;
                    break;
                }

                warn!(error = %e, "Frame capture failed");
                rt.block_on(state.frame_rejected(format!("Capture failed: {}", e)));
                if rt.block_on(storage::should_stop_on_errors(&state)) {
                    error!("Too many capture failures, stopping");
                    state.send_error("Too many capture failures, stopping".to_string());
                    break;
                }
                continue;
            }
        };

        if state.is_cancelled() {
            break;
        }

        if camera.info().has_cooler && last_status_at.elapsed() >= STATUS_POLL_INTERVAL {
            poll_camera_status(&state, camera.as_ref(), settings.target_temp_c, &rt);
            last_status_at = Instant::now();
        }

        frame_number += 1;
        let arc_frame = Arc::new(frame);

        // Send to stacking channel (non-blocking — drop frame if full)
        let stacking_msg = CapturedFrame {
            frame: Arc::clone(&arc_frame),
            frame_number,
            settings: settings.clone(),
            camera_info: camera_info.clone(),
        };
        if stacking_tx.try_send(stacking_msg).is_err() {
            state.frame_dropped();
            debug!(frame_number, "Frame dropped: stacking pipeline busy");
        }

        // Send to storage channel (non-blocking — independent dropping)
        let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
        if settings.save_raw_frames && is_stacking_mode && state.disk_writer.is_enabled() {
            let storage_msg = CapturedFrame {
                frame: arc_frame,
                frame_number,
                settings,
                camera_info,
            };
            if storage_tx.try_send(storage_msg).is_err() {
                warn!(
                    frame_number,
                    "Raw frame dropped: storage pipeline busy"
                );
            }
        }
    }

    debug!("Capture task ended");
    // stacking_tx and storage_tx are dropped here, signaling downstream to exit.
    // Return the handle so the orchestrator can hand it back to the camera
    // session (or drop it on a hard disconnect).
    if camera_ok {
        Some(camera)
    } else {
        let _ = camera.close();
        None
    }
}

/// Read the camera's live status, cache it, and broadcast a `CameraStatusUpdated` event.
///
/// Status reads run from inside the capture thread, naturally serialized with
/// `camera.capture()` calls — this avoids contention with vendor SDKs that
/// require a single handle per device. Errors are logged and swallowed because
/// status reporting is best-effort and must not interrupt capture.
fn poll_camera_status(
    state: &Arc<AppState>,
    camera: &dyn crate::camera::Camera,
    target_temp_c: Option<f64>,
    rt: &tokio::runtime::Handle,
) {
    match camera.status() {
        Ok(status) => {
            let name = camera.info().name.clone();
            rt.block_on(state.update_camera_status(&name, status, target_temp_c));
        }
        Err(e) => {
            debug!(error = %e, "Failed to read camera status");
        }
    }
}

