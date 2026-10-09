use super::capture_thread::{capture_probe_frame, run_capture_task, CaptureChannels, FrameNumbers};
use super::config_overrides::*;
use super::watchdog::*;
use night_amplifier_core::camera::CameraError;
use crate::capture::channel::{pipeline_capacities, QueueDepth};
use crate::capture::channel::{CapturedFrame, StackedFrame};
use crate::error::ApiError;
use crate::state::{AppState, CameraRole, CaptureState, SessionResumePlan};
use std::sync::mpsc;
use std::sync::Arc;
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
    use crate::camera::lifecycle;

    debug!(camera_id = %camera_id, resumed = resume.is_some(), "Capture pipeline starting");

    // The stacking thread offers frames to Push-To's async tasks through this.
    let rt_handle = tokio::runtime::Handle::current();

    // Get camera info for opening
    let camera_info = match storage::get_camera_info(&state, &camera_id).await {
        Some(info) => info,
        None => {
            error!(camera_id = %camera_id, "Camera not found in capture loop");
            state.send_error("Camera not found".to_string());
            state.end_capture_state();
            return;
        }
    };

    // Initialize capture session
    let resume_dir = resume.as_ref().and_then(|p| p.disk_session_dir.clone());
    if let Err(e) = storage::initialize_capture_session(&state, resume_dir).await {
        error!(error = %e, "Failed to initialize capture session");
        state.send_error(e);
        state.end_capture_state();
        return;
    }

    // Only now, with the session directory in place. `sync_disk_session` opens no
    // directory unless a capture is active, so flipping this earlier let a settings
    // update land on a capture whose directory did not exist yet. Compare-and-set: a
    // Stop that landed during startup left `Stopping`, which a plain write overwrote.
    let started = |current| (current == CaptureState::Starting).then_some(CaptureState::Capturing);
    if let Err(current) = state.transition_capture_state(started) {
        debug!(camera_id = %camera_id, state = ?current, "Capture stopped during startup");
        // Nothing was queued into it, so there is nothing for `end_session` to wait out.
        state.disk_writer.abandon_session();
        state.end_capture_state();
        return;
    }

    // Snapshot what a resume would need, now that the disk session exists and
    // the settings for this run are fixed. Recorded for every capture, because
    // a dropout can happen in any of them.
    let numbers = FrameNumbers::starting_at(resume.as_ref().map_or(1, |plan| plan.next_frame));
    let settings = {
        let settings = crate::state::CaptureSettings::clone(&state.settings.snapshot());
        state.resume.record(SessionResumePlan {
            camera_id: camera_id.clone(),
            settings: settings.clone(),
            disk_session_dir: state.disk_writer.session_dir(),
            next_frame: numbers.peek(),
        });
        settings
    };

    // Startup is not instantaneous, and a settings update that arrived during it saw an
    // inactive capture and left the writer alone. Reconcile once against the settings
    // this run is actually starting with.
    storage::sync_disk_session(&state, &settings, true).await;

    // Take the handle from AppState (held by `session::camera` since connect).
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
        // Failed again after the reopen and before this resume could take the handle:
        // the capture stays paused for the recovery under way, which resumes it.
        Err(e @ ApiError::CameraRecovering { .. }) => {
            warn!(camera_id = %camera_id, error = %e, "Camera is recovering; the capture stays paused");
            state.end_capture_state();
            return;
        }
        // Recovered like a handle a running capture lost: the capture pauses for the reopen,
        // which resumes it, and recovery decides what the observer hears.
        Err(e @ ApiError::CameraHandleLost { .. }) => {
            warn!(camera_id = %camera_id, error = %e, "No camera handle to start the capture with");
            lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, None).await;
            state.end_capture_state();
            return;
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Failed to take camera handle for capture");
            state.send_error(format!("Failed to take camera handle: {}", e));
            state.end_capture_state();
            return;
        }
    };

    // Register active camera cancel token in state
    state
        .set_camera_token(CameraRole::Main, camera.cancel_token())
        .await;

    // A Stop or Disconnect that landed during startup found no token to cut the probe
    // exposure with; without this the first frame would run its whole length.
    if state.is_cancelled() {
        debug!(camera_id = %camera_id, "Capture stopped before its first frame");
        state.clear_camera_token(CameraRole::Main).await;
        lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, Some(camera)).await;
        state.end_capture_state();
        return;
    }

    // New session: force a full CaptureConfig reapply on the very next
    // capture() regardless of any out-of-band mutation (cooler/target-temp)
    // that happened while this handle was idle between sessions.
    camera.invalidate_config_cache();

    // Capture a probe frame to determine dimensions and channel capacities
    let settings = state.settings_for_new_frame();
    let mut capture_config = settings.to_capture_config();
    apply_best_raw_format(&mut capture_config, &camera_info.info, &camera_name);
    apply_cooler_support_override(&mut capture_config, &camera_info.info, &camera_name);
    apply_sensor_mode_support_override(&mut capture_config, &camera_info.info);
    // Bounded like every other capture. Unbounded, this call is where a dead
    // handle hides: the field log shows seventy seconds between "Starting
    // capture session" and the SDK finally admitting the device was gone, with
    // nothing on screen for the whole of it.
    let probe_timeout = capture_watchdog_timeout(&capture_config, &camera_info.info);
    let probe_state = Arc::clone(&state);
    let probe_config = capture_config.clone();
    let probe_number = numbers.peek();
    let (camera, probe_result) = match tokio::task::spawn_blocking(move || {
        capture_probe_frame(camera, probe_config, probe_number, probe_timeout, &probe_state)
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
            state.clear_camera_token(CameraRole::Main).await;
            let reason = match &probe_result {
                Some(Err(e)) => e.to_string(),
                _ => "camera did not return the first frame in time".to_string(),
            };
            // A lost device, a stall restarting did not cure, and a timeout that already
            // abandoned the handle all end as a fault, and recovery decides what the
            // observer hears. Anything else is this capture's own failure to report.
            let faulted = !matches!(
                &probe_result,
                Some(Err(e)) if !e.is_sdk_disconnected() && !matches!(e, CameraError::ExposureTimeout(_))
            );
            match camera {
                // Cut short by a Stop or Disconnect: what was asked for, not a failure.
                Some(cam) if !faulted && state.is_cancelled() => {
                    debug!(reason = %reason, "Probe frame cancelled by a stop");
                    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, Some(cam)).await
                }
                Some(cam) if !faulted => {
                    error!(reason = %reason, "Failed to capture probe frame for pipeline setup");
                    state.send_error(format!("Failed to capture initial frame: {}", reason));
                    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, Some(cam)).await
                }
                camera => {
                    warn!(reason = %reason, "The camera faulted on the first frame; handing it to recovery");
                    if let Some(cam) = camera {
                        release_faulted_handle(cam, &state, CameraRole::Main);
                    }
                    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, None).await;
                }
            }
            state.end_capture_state();
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
            state.end_capture_state();
            return;
        }
    };

    // Each channel is sized from the payload it carries, not one frame size for all
    // three: the two capture channels move `Arc<RawFrame>` — sensor bytes, a quarter
    // to a sixth of the debayered frame — while only the render channel moves the f32
    // `Frame`; the stacking channel is also bounded by the lag it would introduce,
    // which is why the exposure factors in. Resolved once, from the settings this
    // session started with — a `SyncSender` cannot be resized anyway, and the probe
    // frame the depth is derived from is equally a snapshot — so an exposure changed
    // mid-session leaves the channels as they are; the figure used is logged below.
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
        queue_budget_bytes = crate::capture::channel::frame_queue_budget_bytes(),
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
    let first_number = numbers.claim();
    let first_raw = Arc::new(probe_raw);
    let first_msg = CapturedFrame {
        frame: Arc::clone(&first_raw),
        frame_number: first_number,
        settings: settings.clone(),
        camera_info: camera_info.clone(),
    };
    let first_msg_storage = CapturedFrame {
        frame: first_raw,
        frame_number: first_number,
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

    let rt_stacking = rt_handle.clone();
    let numbers_capture = numbers.clone();

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
                numbers_capture,
            )
        })
        .expect("Failed to spawn capture thread");

    // On a resume, hand the parked accumulators to the new stacking task; on a
    // fresh start `CaptureService` has already cleared them.
    let carryover = if resume.is_some() { state.resume.take_stack() } else { None };

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
            run_render_task(state_render, render_rx, depth_render);
        })
        .expect("Failed to spawn render thread");

    let storage_handle = std::thread::Builder::new()
        .name("storage-task".into())
        .spawn(move || {
            storage::run_storage_task(state_storage, storage_rx, storage_queue_depth);
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

    // A plate solve runs on the `push-to-solve` thread and can outlive this session
    // by minutes. Left alone it keeps the solve latch raised, so the *next* capture
    // session cannot solve either.
    super::solving::abandon_solve_on_shutdown(&state).await;

    info!(camera_id = %camera_id, "Capture pipeline ended");

    // Before the handle goes back: a fault hands it to recovery, which may resume at once.
    state.resume.edit_plan(|plan| plan.next_frame = numbers.peek());

    // Return the camera handle to the session (or finalize disconnect if lost).
    state.clear_camera_token(CameraRole::Main).await;
    lifecycle::return_from_capture(&state, CameraRole::Main, &camera_name, returned_camera).await;
    state.end_capture_state();
}

// =============================================================================
// CaptureTask
// =============================================================================
