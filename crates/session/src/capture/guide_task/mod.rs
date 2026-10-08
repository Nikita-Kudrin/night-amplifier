//! The guide camera's free-running loop: one thread, not the four-thread imaging
//! pipeline, since nothing here is stacked or queued. Starts on connect, not Start
//! Capture, so plate solving and a look through the guide scope are available *while*
//! framing, before any imaging session begins.
//! **Render gate**: post-processing matches the main camera's but runs only while
//! somebody watches ([`FrameStream::has_viewers`]), skipping extraction/stretch/encode
//! (two thirds of a frame's cost) so a guide camera doesn't double a main-only
//! session's CPU bill. Solving and raw saving aren't gated — they're why the loop runs.

mod output;
mod sensor;

use output::{render_and_publish, GuideDiskSession};
use sensor::{GuideCooler, SensorReadout};

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use super::analysis::PreviewAnalysis;
use super::solving::{self, SolveSource};
use super::stage_config;
use super::stall::{handle_stall, StallSite, StallTracker, StallVerdict};
use super::stream_encoding::{ConversionCache, FailureReports};
use super::watchdog::{capture_frame_bounded, capture_watchdog_timeout, CaptureOutcome};
use night_amplifier_core::camera::Camera;
use crate::error::ApiError;
use crate::state::{
    AppState, CameraOp, CameraRole, ConnectedCameraInfo, GuideLoopTicket, RawSessionResume,
};

/// How long the loop waits before retrying after a recoverable capture error, so a
/// camera erroring instantly cannot spin a core.
const ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Longer wait after the camera rejected the capture config. Retrying that fast is
/// pointless — nothing changes until the user edits a setting — but the loop keeps
/// polling so it recovers the moment they do.
const REJECTED_CONFIG_BACKOFF: Duration = Duration::from_secs(2);

/// Start the guide loop for a connected guide camera. Returns `false`, starting nothing,
/// while a loop is already registered — starting, running, or winding down on its own: a
/// second one would compete with it for the one handle.
///
/// Non-blocking: the handle is checked out on a tokio task, because `take_for_capture`
/// may have to wait for the monitor to hand it back and `connect` must not block on that.
pub fn start(state: &Arc<AppState>, camera: &ConnectedCameraInfo) -> bool {
    // Registered before the spawn, not inside it: a disconnect arriving while the task is
    // still queued would otherwise find no loop, decide none was running, and close the
    // handle out from under it.
    let Some(ticket) = state.guide_loops.register() else {
        return false;
    };

    let state = Arc::clone(state);
    let camera = camera.clone();
    tokio::spawn(async move {
        let id = ticket.id;
        let Err(failure) = spawn_loop(&state, &camera, ticket).await else {
            return;
        };
        state.guide_loops.finish(id);
        match failure {
            StartFailure::Recovering => info!(
                camera = %camera.info.name,
                "Guide loop not started: the camera is being reopened, which restarts it"
            ),
            StartFailure::Lost => info!(
                camera = %camera.info.name,
                "Guide loop not started: its handle was lost and the camera disconnected"
            ),
            StartFailure::Failed(e) => {
                error!(camera = %camera.info.name, error = %e, "Could not start the guide loop");
                state.send_error(format!(
                    "Guide camera '{}' connected but its loop could not start: {}",
                    camera.info.name, e
                ));
            }
        }
    });
    true
}

/// Why a guide loop did not start.
enum StartFailure {
    /// The handle went to recovery instead; the reopen starts the loop again.
    Recovering,
    /// The handle was lost and, with reconnecting off, the camera torn down — which the
    /// reconnect supervisor reports.
    Lost,
    Failed(String),
}

async fn spawn_loop(
    state: &Arc<AppState>,
    camera_info: &ConnectedCameraInfo,
    ticket: GuideLoopTicket,
) -> Result<(), StartFailure> {
    let camera_name = camera_info.info.name.clone();
    let GuideLoopTicket { id, cancel } = ticket;

    // A disconnect that landed between `start` and here has already set the switch; taking
    // the handle now would leave it checked out of a slot nobody is going to reclaim.
    if cancel.load(Ordering::SeqCst) {
        return Ok(());
    }

    let camera = match crate::camera::lifecycle::take_for_capture(
        state,
        CameraRole::Guide,
        &camera_name,
    )
    .await
    {
        Ok(camera) => camera,
        Err(ApiError::CameraRecovering { .. }) => return Err(StartFailure::Recovering),
        Err(ApiError::CameraHandleLost { .. }) => {
            return Err(hand_back_lost_handle(state, id, &camera_name).await)
        }
        Err(e) => return Err(StartFailure::Failed(e.to_string())),
    };

    state
        .set_camera_token(CameraRole::Guide, camera.cancel_token())
        .await;

    // Rejoin the folder a dropout interrupted, so one guide session stays in one
    // directory across a reconnect — the same reason `SessionResumePlan` carries the
    // imaging camera's.
    let resume = state.slot(CameraRole::Guide).raw_session();

    let loop_state = Arc::clone(state);
    let loop_info = camera_info.clone();
    let rt = tokio::runtime::Handle::current();
    let spawned = std::thread::Builder::new()
        .name("guide-task".into())
        .spawn(move || {
            // Marked here rather than in `connect`: this is the first moment a loop
            // certainly exists, so a spawn that never got this far cannot leave the
            // imaging camera stood down for a solve source that is not there.
            loop_state.guide_loops.mark_running(id);
            // A panic would end the thread still registered, its slot `Guiding` with no
            // handle: Start answered "running" and no later loop could take the camera.
            // Recovered like the imaging pipeline's panicked capture thread.
            let camera = std::panic::catch_unwind(AssertUnwindSafe(|| {
                run(&loop_state, &loop_info, camera, &cancel, resume, &rt)
            }))
            .unwrap_or_else(|_| {
                error!(camera = %loop_info.info.name, "The guide loop panicked; handing its camera to recovery");
                None
            });

            // Unregistered *before* handing the handle back. On the device-loss path
            // `return_from_capture(None)` reaches `finalize_disconnect`, which asks this
            // loop to stop — and a loop asking itself to stop would sit out the whole
            // wait budget for a handle it has already lost. By id, so a loop that `stop`
            // gave up on and that ends after its successor started leaves that one be.
            loop_state.guide_loops.finish(id);

            rt.block_on(crate::camera::lifecycle::return_from_capture(
                &loop_state,
                CameraRole::Guide,
                &loop_info.info.name,
                camera,
            ));
        });
    if let Err(e) = spawned {
        // The handle went down with the closure. Recover it like any other lost handle.
        hand_back_lost_handle(state, id, &camera_name).await;
        return Err(StartFailure::Failed(format!("failed to spawn the guide thread: {e}")));
    }

    info!(camera = %camera_name, "Guide camera loop started");
    Ok(())
}

/// Recover a handle this starting loop never got, or lost on the way. Unregistered first:
/// the teardown stops "the guide loop", and a registered one is waited on for its handle
/// — this loop, which has none to give, for the whole of `stop`'s budget.
async fn hand_back_lost_handle(state: &Arc<AppState>, id: u64, camera_name: &str) -> StartFailure {
    state.guide_loops.finish(id);
    crate::camera::lifecycle::return_from_capture(
        state,
        CameraRole::Guide,
        camera_name,
        None,
    )
    .await;
    if state.slot(CameraRole::Guide).is_recovering() {
        StartFailure::Recovering
    } else {
        StartFailure::Lost
    }
}

/// Ask the guide loop to stop and wait for it to hand the handle back.
///
/// Called before anything closes the handle. Returns once the slot holds a handle again
/// or the wait budget expires — a loop stuck inside a vendor call has already abandoned
/// its handle to the capture watchdog, and waiting longer would not produce one.
pub async fn stop(state: &Arc<AppState>) {
    // Unregistered first: once this function has been called the loop is not running,
    // and solving goes back to the imaging camera immediately — a cooled guide camera
    // then warms up for minutes, and leaving it marked running through all of it means
    // neither camera may offer the solver a frame.
    let Some(cancel) = state.guide_loops.take() else {
        return;
    };
    cancel.store(true, Ordering::SeqCst);
    // Cut short the exposure in flight, or the stop waits out a full guide sub.
    state.slot(CameraRole::Guide).cancel_exposure().await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let slot = state.slot(CameraRole::Guide);
    while tokio::time::Instant::now() < deadline {
        // The loop's thread holds the other reference to its switch until it has finished
        // handing back — which a recovering slot refuses, closing the handle instead, so
        // "holds a handle" alone would wait out the whole budget there.
        let loop_gone = Arc::strong_count(&cancel) == 1;
        if slot.holds_handle() || loop_gone {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    debug!("Guide loop did not return its handle within the stop budget");
}

/// The loop body. Returns the handle unless a watchdog abandoned it.
pub(super) fn run(
    state: &Arc<AppState>,
    camera_info: &ConnectedCameraInfo,
    mut camera: Box<dyn Camera>,
    cancel: &AtomicBool,
    resume: Option<RawSessionResume>,
    rt: &tokio::runtime::Handle,
) -> Option<Box<dyn Camera>> {
    debug!(camera = %camera_info.info.name, "Guide task started");

    let mut conversions = ConversionCache::default();
    let mut failures = FailureReports::default();
    // Outlives the loop for the same reason the render task's does: the background
    // model and image statistics describe the sky, not the frame.
    let mut analysis = PreviewAnalysis::new();
    let mut cfa_key = None;
    let mut cfa_pipeline = None;
    let mut disk = GuideDiskSession::new(resume);
    let mut frame_number: u64 = disk.first_frame_number() - 1;
    // The config error already reported, so the same one is not reported again.
    let mut rejected_config: Option<String> = None;
    let mut cooler = GuideCooler::default();
    let mut sensor = SensorReadout::default();
    let mut stalls = StallTracker::for_camera(state, CameraRole::Guide, &camera_info.info.name);

    while !cancel.load(Ordering::SeqCst) {
        let settings = state.settings.snapshot();
        let profile = settings.guide_camera.clone();
        let mut config = settings.to_capture_config_with(&profile, CameraRole::Guide);

        // Everything below runs between exposures, which is the only moment anything can
        // reach this camera: the loop owns its handle for the whole connection, so the
        // monitor thread — which drives these for the imaging camera — can never check
        // it out.
        for op in state.slot(CameraRole::Guide).drain_ops() {
            apply_op(camera.as_mut(), op, &camera_info.info.name);
        }
        sensor.sample(camera.as_ref(), state, camera_info, &profile);
        if let Some(setpoint) =
            cooler.setpoint(&profile, sensor.temperature_c, std::time::Instant::now())
        {
            config.target_temp_c = Some(setpoint);
        }
        super::config_overrides::apply_best_raw_format(
            &mut config,
            &camera_info.info,
            &camera_info.info.name,
        );
        super::config_overrides::apply_cooler_support_override(
            &mut config,
            &camera_info.info,
            &camera_info.info.name,
        );
        super::config_overrides::apply_sensor_mode_support_override(&mut config, &camera_info.info);
        super::config_overrides::apply_guide_acquisition_override(&mut config);

        frame_number += 1;
        let watchdog_timeout = capture_watchdog_timeout(&config, &camera_info.info);
        let (returned, result) = match capture_frame_bounded(
            camera,
            config,
            frame_number,
            watchdog_timeout,
            state,
            CameraRole::Guide,
        ) {
            CaptureOutcome::Completed(cam, result) => (cam, result),
            // The handle went with a detached thread that never returned. Nothing
            // left to hand back; the fault detector has already recorded it.
            CaptureOutcome::TimedOut => return None,
        };
        camera = returned;

        let raw_frame = match result {
            Ok(frame) => Arc::new(frame),
            Err(e) => {
                if matches!(e, night_amplifier_core::camera::CameraError::Cancelled) {
                    camera.cancel_token().store(false, Ordering::SeqCst);
                    continue;
                }
                if e.is_sdk_disconnected() {
                    error!(error = %e, "Guide camera disconnected during capture");
                    return None;
                }
                if let night_amplifier_core::camera::CameraError::ExposureTimeout(budget) = e {
                    let name = &camera_info.info.name;
                    if let StallVerdict::Escalate(_) =
                        handle_stall(&mut stalls, StallSite::Guide, state, name, budget)
                    {
                        return None;
                    }
                    continue;
                }
                if let night_amplifier_core::camera::CameraError::InvalidParameter { .. } = e {
                    // The camera rejected the config, so it will reject the identical
                    // one next frame too. Report it once and idle instead of filling
                    // the log at 2 Hz — the loop stays up because a settings edit is
                    // exactly how the user fixes this.
                    let message = e.to_string();
                    if rejected_config.as_deref() != Some(&message) {
                        error!(error = %e, "Guide camera rejected its capture settings");
                        state.send_error(format!(
                            "Guide camera '{}' rejected its settings: {}",
                            camera_info.info.name, e
                        ));
                        rejected_config = Some(message);
                    }
                    std::thread::sleep(REJECTED_CONFIG_BACKOFF);
                    continue;
                }
                warn!(error = %e, "Guide frame capture failed");
                std::thread::sleep(ERROR_BACKOFF);
                continue;
            }
        };
        rejected_config = None;
        stalls.frame_delivered();

        // Above both gates below: an unwatched guide camera with no solve target is
        // still saving subs if the user asked it to.
        disk.write(state, &settings, &raw_frame, frame_number, camera_info);

        let watched = state.guide_stream.has_viewers();
        let solving_wanted = solving::plate_solve_available(state, SolveSource::Guide);
        if !watched && !solving_wanted {
            continue;
        }

        // Rebuilt only when the settings behind it change, exactly as the stacking task
        // does — building a `CfaPipeline` per frame is pure waste.
        let key = (settings.sensor_correction.clone(), settings.stacking_type);
        if cfa_key.as_ref() != Some(&key) {
            cfa_pipeline = Some(stage_config::build_cfa_pipeline(&settings));
            cfa_key = Some(key);
        }
        let algorithm = stage_config::debayer_algorithm(&settings);
        let frame = match stage_config::convert_captured_frame(
            &raw_frame,
            &camera_info.info,
            cfa_pipeline.as_ref().expect("cfa pipeline built above"),
            algorithm,
        ) {
            Ok(frame) => Arc::new(frame),
            Err(e) => {
                warn!(error = %e, "Guide frame conversion failed");
                continue;
            }
        };

        if solving_wanted {
            solving::offer_plate_solve(state, rt, Arc::clone(&frame), SolveSource::Guide);
        }

        if !watched {
            continue;
        }

        render_and_publish(
            state,
            frame,
            &settings,
            &mut conversions,
            &mut failures,
            &mut analysis,
        );
    }

    debug!(camera = %camera_info.info.name, "Guide task ended");
    Some(camera)
}

/// Run one queued hardware call against the handle.
fn apply_op(camera: &mut dyn Camera, op: CameraOp, camera_name: &str) {
    match op {
        CameraOp::SetDewHeater { enabled, power } => match camera.set_dew_heater(enabled, power) {
            Ok(()) => info!(camera = %camera_name, enabled, power, "Guide dew heater applied"),
            Err(e) => warn!(error = %e, "Guide dew heater change failed"),
        },
    }
}

#[cfg(test)]
mod tests;
