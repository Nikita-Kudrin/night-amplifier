use super::channel;
use super::config_overrides::*;
use super::drop_log::DropLog;
use super::watchdog::*;
use crate::frame::Frame;
use crate::server::capture::channel::{pipeline_capacities, QueueDepth};
use crate::server::capture::channel::{CapturedFrame, StackedFrame};
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CameraRole, CaptureState, SessionResumePlan, StackingType};
use crate::stacking::CometContext;
use crate::telemetry::metrics as telemetry_metrics;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::pipeline;
use super::render_task::run_render_task;
use super::stacking_task::{run_stacking_task, StackingChannels};
use super::storage;
/// Run one capture session to completion.
///
/// `resume` is set when a reconnect is picking up a session a device fault
/// interrupted: it rejoins that session's raw-frame directory and carries its
/// stacking accumulators forward instead of starting a new observation.
pub async fn run_capture_loop(
    state: Arc<AppState>,
    camera_id: String,
    resume: Option<SessionResumePlan>,
) {
    use crate::server::camera_session::lifecycle;

    debug!(camera_id = %camera_id, resumed = resume.is_some(), "Capture pipeline starting");

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
    let resume_dir = resume.as_ref().and_then(|p| p.disk_session_dir.clone());
    if let Err(e) = storage::initialize_capture_session(&state, resume_dir).await {
        error!(error = %e, "Failed to initialize capture session");
        state.send_error(e);
        state.set_capture_state(CaptureState::Idle).await;
        return;
    }

    // Only now, with the session directory in place. `sync_disk_session` opens no
    // directory unless a capture is active, so flipping this earlier let a settings
    // update land on a capture whose directory did not exist yet.
    state.set_capture_state(CaptureState::Capturing).await;

    // Snapshot what a resume would need, now that the disk session exists and
    // the settings for this run are fixed. Recorded for every capture, because
    // a dropout can happen in any of them.
    let settings = {
        let settings = state.settings.read().await.clone();
        *state.session_resume_plan.write().await = Some(SessionResumePlan {
            camera_id: camera_id.clone(),
            settings: settings.clone(),
            disk_session_dir: state.disk_writer.session_dir(),
        });
        settings
    };

    // Startup is not instantaneous, and a settings update that arrived during it saw an
    // inactive capture and left the writer alone. Reconcile once against the settings
    // this run is actually starting with.
    storage::sync_disk_session(&state, &settings, true).await;

    // Take the handle from AppState (held by camera_session since connect).
    // This cancels any in-progress warmup and flips the phase to Capturing.
    let camera_name = camera_info.info.name.clone();
    let mut camera = match lifecycle::take_for_capture(&state, CameraRole::Main, &camera_name).await {
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
        .set_camera_token(CameraRole::Main, camera.cancel_token())
        .await;

    // New session: force a full CaptureConfig reapply on the very next
    // capture() regardless of any out-of-band mutation (cooler/target-temp)
    // that happened while this handle was idle between sessions.
    camera.invalidate_config_cache();

    // Capture a probe frame to determine dimensions and channel capacities
    let settings = state.settings.read().await.clone();
    let mut capture_config = settings.to_capture_config();
    apply_best_raw_format(&mut capture_config, &camera_info.info, &camera_name);
    apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera_name);
    apply_sensor_mode_support_override(&mut capture_config, &camera_info.info, &camera_name);
    // Bounded like every other capture. Unbounded, this call is where a dead
    // handle hides: the field log shows seventy seconds between "Starting
    // capture session" and the SDK finally admitting the device was gone, with
    // nothing on screen for the whole of it.
    let probe_timeout = Duration::from_micros(capture_config.exposure_us)
        + capture_watchdog_margin(capture_config.exposure_us, capture_config.timeout);
    let probe_state = Arc::clone(&state);
    let probe_config = capture_config.clone();
    let (camera, probe_result) = match tokio::task::spawn_blocking(move || {
        capture_frame_bounded(camera, probe_config, 1, probe_timeout, &probe_state)
    })
    .await
    {
        Ok(CaptureOutcome::Completed(cam, result)) => (Some(cam), Some(result)),
        Ok(CaptureOutcome::TimedOut) => (None, None),
        Err(e) => {
            error!(error = %e, "Probe capture task failed to run");
            (None, None)
        }
    };

    let (camera, probe_raw) = match (camera, probe_result) {
        (Some(cam), Some(Ok(frame))) => (cam, frame),
        (camera, probe_result) => {
            let reason = match &probe_result {
                Some(Err(e)) => e.to_string(),
                _ => "camera did not return the first frame in time".to_string(),
            };
            error!(reason = %reason, "Failed to capture probe frame for pipeline setup");
            state.send_error(format!("Failed to capture initial frame: {}", reason));
            state.clear_camera_token(CameraRole::Main).await;

            // A lost device invalidates the handle; a timeout already abandoned
            // it. Either way the session ends as a fault, so the reconnect
            // supervisor gets a chance at it.
            let handle_is_usable =
                matches!(&probe_result, Some(Err(e)) if !e.is_sdk_disconnected());
            match (camera, handle_is_usable) {
                (Some(cam), true) => {
                    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, Some(cam)).await
                }
                (Some(mut cam), false) => {
                    let _ = cam.close();
                    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, None).await;
                }
                (None, _) => lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, None).await,
            }
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };
    // Sized through the same raw-CFA stage the pipeline will use: with
    // superpixel debayering on, a frame is a quarter of the size a full-
    // resolution demosaic would suggest, and the channel budget follows it.
    let probe_frame = match pipeline::convert_captured_frame(
        &probe_raw,
        &camera_info.info,
        &pipeline::build_cfa_pipeline(&settings),
        pipeline::debayer_algorithm(&settings),
    ) {
        Ok(f) => f,
        Err(e) => {
            error!(error = %e, "Failed to decode probe frame");
            state.send_error(format!("Failed to decode initial frame: {}", e));
            state.clear_camera_token(CameraRole::Main).await;
            lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, Some(camera)).await;
            state.set_capture_state(CaptureState::Idle).await;
            return;
        }
    };

    // Each channel is sized from the payload it carries, not from one frame size for
    // all three: the two capture channels move `Arc<RawFrame>` — sensor bytes, a
    // quarter to a sixth of the debayered frame — while only the render channel moves
    // the f32 `Frame`. The stacking channel is bounded by the lag it would introduce as
    // well, which is why the exposure comes into it.
    //
    // Resolved once, from the settings this session started with. A `SyncSender` cannot
    // be resized anyway, and the probe frame the depth is derived from is equally a
    // snapshot — so an exposure changed mid-session leaves the channels as they are, and
    // the figure actually used is logged below rather than left to be inferred.
    let raw_memory = probe_raw.data_slice().len();
    let frame_memory = probe_frame.memory_size();
    let capacities = pipeline_capacities(raw_memory, frame_memory, settings.exposure_us);
    info!(
        raw_memory_bytes = raw_memory,
        frame_memory_bytes = frame_memory,
        exposure_us = settings.exposure_us,
        stacking_channel_capacity = capacities.stacking,
        storage_channel_capacity = capacities.storage,
        render_channel_capacity = capacities.render,
        queue_budget_bytes = crate::server::capture::channel::frame_queue_budget_bytes(),
        width = probe_frame.width(),
        height = probe_frame.height(),
        channels = probe_frame.channels(),
        "Pipeline channel capacity calculated"
    );

    // Create bounded channels
    let (stacking_tx, stacking_rx) = mpsc::sync_channel::<CapturedFrame>(capacities.stacking);
    let (storage_tx, storage_rx) = mpsc::sync_channel::<CapturedFrame>(capacities.storage);
    let (render_tx, render_rx) = mpsc::sync_channel::<StackedFrame>(capacities.render);
    // Shared between the two tasks that own the ends of the render channel, so the
    // stacking task can tell whether the copy it is about to build has anywhere to go.
    let render_depth = QueueDepth::default();
    // The other two are reported, not read: a depth that sits at the ceiling says the
    // stage behind it is slow, while one that spikes and drains says it stalled once.
    // `SyncSender` exposes no length, so this is the only way to tell those apart.
    let stacking_queue_depth = QueueDepth::default();
    let storage_queue_depth = QueueDepth::default();

    // Send the probe frame as the first frame through the pipeline
    let first_raw = Arc::new(probe_raw);
    let first_msg = CapturedFrame {
        frame: Arc::clone(&first_raw),
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let first_msg_storage = CapturedFrame {
        frame: first_raw,
        frame_number: 1,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    // The probe frame counts toward the drop-rate denominator like any other.
    state.frame_delivered();
    stacking_queue_depth.sent();
    if stacking_tx.send(first_msg).is_err() {
        stacking_queue_depth.taken();
    }
    storage_queue_depth.sent();
    if storage_tx.send(first_msg_storage).is_err() {
        storage_queue_depth.taken();
    }

    // Spawn worker threads — each gets a clone of the tokio Handle
    let state_capture = Arc::clone(&state);
    let state_stacking = Arc::clone(&state);
    let state_render = Arc::clone(&state);
    let state_storage = Arc::clone(&state);

    let depth_stacking = render_depth.clone();
    let depth_render = render_depth;

    let stacking_depth_capture = stacking_queue_depth.clone();
    let storage_depth_capture = storage_queue_depth.clone();

    let rt_capture = rt_handle.clone();
    let rt_stacking = rt_handle.clone();
    let rt_render = rt_handle.clone();
    let rt_storage = rt_handle.clone();

    let capture_handle = std::thread::Builder::new()
        .name("capture-task".into())
        .spawn(move || {
            run_capture_task(
                state_capture,
                camera,
                CaptureChannels {
                    stacking_tx,
                    storage_tx,
                    stacking_depth: stacking_depth_capture,
                    storage_depth: storage_depth_capture,
                    capacities,
                },
                rt_capture,
            )
        })
        .expect("Failed to spawn capture thread");

    // On a resume, hand the parked accumulators to the new stacking task; on a
    // fresh start `CaptureService` has already cleared them.
    let carryover = if resume.is_some() {
        state
            .stacking_carryover
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    } else {
        None
    };

    let stacking_handle = std::thread::Builder::new()
        .name("stacking-task".into())
        .spawn(move || {
            run_stacking_task(
                state_stacking,
                StackingChannels {
                    stacking_rx,
                    stacking_depth: stacking_queue_depth,
                    render_tx,
                    render_depth: depth_stacking,
                    render_capacity: capacities.render,
                },
                rt_stacking,
                carryover,
            );
        })
        .expect("Failed to spawn stacking thread");

    let render_handle = std::thread::Builder::new()
        .name("render-task".into())
        .spawn(move || {
            run_render_task(state_render, render_rx, depth_render, rt_render);
        })
        .expect("Failed to spawn render thread");

    let storage_handle = std::thread::Builder::new()
        .name("storage-task".into())
        .spawn(move || {
            storage::run_storage_task(state_storage, storage_rx, storage_queue_depth, rt_storage);
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

    // End capture session. Off the runtime: `end_session` waits for the writer to be
    // told, because a SER container it never hears about is left without the frame count
    // in its header and is unreadable. The producing threads are joined by now, so the
    // queue is only draining and the wait is bounded.
    {
        let disk_writer = state.disk_writer.clone();
        let _ = tokio::task::spawn_blocking(move || disk_writer.end_session()).await;
    }

    // A plate solve runs on a detached task and can outlive the frame it was given
    // by minutes. Left alone it keeps the solve latch raised, so the *next* capture
    // session cannot solve either.
    super::solving::abandon_solve_on_shutdown(&state).await;

    info!(camera_id = %camera_id, "Capture pipeline ended");

    // Return the camera handle to the session (or finalize disconnect if lost).
    state.clear_camera_token(CameraRole::Main).await;
    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, returned_camera).await;
    state.set_capture_state(CaptureState::Idle).await;
}

// =============================================================================
// CaptureTask
// =============================================================================

/// The sending ends of the two capture channels, with the depth counters that shadow
/// them.
///
/// Grouped rather than passed as four more arguments: a sender and its counter are only
/// correct together — the counter has to be incremented before the send and given back
/// when the send did not happen — so keeping them apart invites exactly the desync
/// `QueueDepth` documents.
pub(crate) struct CaptureChannels {
    pub stacking_tx: mpsc::SyncSender<CapturedFrame>,
    pub storage_tx: mpsc::SyncSender<CapturedFrame>,
    pub stacking_depth: QueueDepth,
    pub storage_depth: QueueDepth,
    pub capacities: channel::PipelineCapacities,
}

/// Camera capture loop running on a dedicated OS thread.
///
/// Acquires frames from the camera and sends them (as `Arc<Frame>`) to the
/// stacking and storage channels. Uses `try_send` on the stacking channel
/// to avoid blocking when the pipeline can't keep up — frames are dropped
/// and counted. The storage channel uses `try_send` independently.
pub(crate) fn run_capture_task(
    state: Arc<AppState>,
    mut camera: Box<dyn crate::camera::Camera>,
    channels: CaptureChannels,
    rt: tokio::runtime::Handle,
) -> Option<Box<dyn crate::camera::Camera>> {
    let CaptureChannels {
        stacking_tx,
        storage_tx,
        stacking_depth,
        storage_depth,
        capacities,
    } = channels;
    debug!("Capture task started");

    // Frame numbering continues from 1 (probe frame was #1)
    let mut frame_number: u64 = 1;
    let mut last_status_at = Instant::now()
        .checked_sub(STATUS_POLL_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut camera_ok = true;
    let mut storage_drops = DropLog::default();

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
        apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera.info().name);
        apply_sensor_mode_support_override(
            &mut capture_config,
            &camera_info.info,
            &camera.info().name,
        );

        // Capture a frame (blocking FFI call, bounded so a stuck SDK call
        // can't freeze the pipeline indefinitely — see capture_frame_bounded).
        let watchdog_timeout = Duration::from_micros(capture_config.exposure_us)
            + capture_watchdog_margin(capture_config.exposure_us, capture_config.timeout);
        let (new_camera, capture_result) = match capture_frame_bounded(
            camera,
            capture_config,
            frame_number + 1,
            watchdog_timeout,
            &state,
        ) {
            CaptureOutcome::Completed(cam, result) => (cam, result),
            // The handle is gone — moved into a detached thread that didn't
            // return in time. Nothing left to close() or return;
            // `stacking_tx`/`storage_tx` still drop normally on the way out.
            CaptureOutcome::TimedOut => return None,
        };
        camera = new_camera;

        let raw_frame = match capture_result {
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
                if e.is_sdk_disconnected() {
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
            camera = match poll_camera_status_bounded(camera, &state, settings.target_temp_c, &rt) {
                StatusPollOutcome::Completed(camera) => {
                    last_status_at = Instant::now();
                    camera
                }
                StatusPollOutcome::TimedOut => return None,
            };
        }

        frame_number += 1;
        // Counted before either send: the denominator of the drop rate is what the
        // camera produced, not what the pipeline managed to accept.
        state.frame_delivered();
        let arc_frame = Arc::new(raw_frame);

        // Send to stacking channel (non-blocking — drop frame if full)
        let stacking_msg = CapturedFrame {
            frame: Arc::clone(&arc_frame),
            frame_number,
            settings: settings.clone(),
            camera_info: camera_info.clone(),
        };
        // Counted before the send and given back on failure — see `QueueDepth`.
        stacking_depth.sent();
        if stacking_tx.try_send(stacking_msg).is_err() {
            stacking_depth.taken();
            state.frame_dropped();
            debug!(frame_number, "Frame dropped: stacking pipeline busy");
        }
        telemetry_metrics::record_pipeline_queue_depth(
            "capture_to_stacking",
            stacking_depth.pending() as u64,
            capacities.stacking as u64,
        );

        // Send to storage channel (non-blocking — independent dropping)
        if settings.saves_raw_frames() && state.disk_writer.is_enabled() {
            let storage_msg = CapturedFrame {
                frame: arc_frame,
                frame_number,
                settings,
                camera_info,
            };
            storage_depth.sent();
            if storage_tx.try_send(storage_msg).is_err() {
                storage_depth.taken();
                if let Some(dropped) = storage_drops.record() {
                    warn!(frame_number, dropped, "Raw frames dropped: storage pipeline busy");
                }
            }
            telemetry_metrics::record_pipeline_queue_depth(
                "capture_to_storage",
                storage_depth.pending() as u64,
                capacities.storage as u64,
            );
        }
    }

    if let Some(dropped) = storage_drops.flush() {
        warn!(
            dropped,
            "Raw frames dropped since the last report: storage pipeline busy"
        );
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
