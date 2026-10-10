//! Lending the camera handle to a capture loop and taking it back.

use std::sync::Arc;
use tracing::{debug, error, warn};

use super::disconnect::begin_warmup;
use super::{finalize_disconnect, send_monitor_cmd, take_camera, with_camera, DisconnectCause};
use crate::error::ApiError;
use crate::state::{AppState, CameraPhase, CameraRole, MonitorCmd};
use night_amplifier_core::camera::Camera;

/// Take a slot's camera handle for a capture session. Cancels any in-progress
/// warmup and transitions the phase to `Capturing`.
pub async fn take_for_capture(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
) -> Result<Box<dyn Camera>, ApiError> {
    let phase = state.camera_phase(role);

    if phase == CameraPhase::WarmingUp {
        // User started capture mid-warmup — cancel, re-enable cooler per
        // current settings before handoff. Capture's per-frame
        // `apply_cooler_config` pushes the final target, so the ramp the
        // monitor would have installed is overridden anyway.
        debug!(camera_name, "Cancelling warmup: capture requested");
        send_monitor_cmd(state, role, MonitorCmd::CancelWarmup);
        state.slot(role).end_warmup();
        let profile = state.settings.snapshot().profile_for(role);
        if profile.cooler_enabled {
            let _ = with_camera(state, role, |cam| cam.set_cooler(true)).await;
            // Re-seed the cooldown ramp so that if capture exits quickly the
            // monitor picks up a gentle ramp rather than snapping to target.
            // Fast mode preserves the old "snap to target" behavior.
            send_monitor_cmd(
                state,
                role,
                MonitorCmd::UpdateCoolerTarget {
                    enabled: true,
                    target: profile.target_temp_c,
                    fast: profile.cooler_fast_mode,
                },
            );
        }
    }

    // Tell the monitor to pause; it will observe `Capturing` phase and skip
    // its polling loop. This avoids contention with capture's own calls.
    send_monitor_cmd(state, role, MonitorCmd::HandOffToCapture);

    let Some(camera) = take_camera(state, role).await else {
        return Err(abandon_hand_off(state, role, camera_name, phase).await);
    };

    // A guide camera's loop runs for the length of the connection, not the length of a
    // session, and the gates below it need to be able to tell those apart.
    let phase = match role {
        CameraRole::Main => CameraPhase::Capturing,
        CameraRole::Guide => CameraPhase::Guiding,
    };
    state.set_camera_phase(role, camera_name, phase);

    Ok(camera)
}

/// Undo a hand-off whose take came back empty, and say what is holding the handle.
///
/// The monitor is resumed unless another owner holds the handle: left paused it never
/// polls again, and a warm-up the Start cancelled left the phase at `WarmingUp` with
/// nothing driving it. On 2026-09-20 that made every later Disconnect a no-op until the
/// board was power-cycled. A lost handle is [`ApiError::CameraHandleLost`].
async fn abandon_hand_off(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
    phase: CameraPhase,
) -> ApiError {
    let slot = state.slot(role);
    if slot.is_recovering() {
        return ApiError::CameraRecovering {
            camera: camera_name.to_string(),
        };
    }

    // Another owner still has it — a stopping capture, or a guide loop on its way out —
    // and hands it back when done. The monitor stays paused for that owner.
    let owner = match phase {
        CameraPhase::Capturing => Some("a capture that is still stopping"),
        CameraPhase::Guiding => Some("the guide loop"),
        _ => None,
    };
    if let Some(owner) = owner {
        warn!(camera_name, role = role.label(), owner, "The camera handle is still held elsewhere");
        return ApiError::Internal(format!(
            "Camera '{camera_name}' is still held by {owner}; try again in a moment"
        ));
    }

    send_monitor_cmd(state, role, MonitorCmd::ResumeAfterCapture);
    if let Some(busy_for) = slot.monitor_call_age() {
        if phase == CameraPhase::WarmingUp {
            let fast = state.settings.snapshot().profile_for(role).cooler_fast_mode;
            begin_warmup(state, role, camera_name, fast).await;
        }
        warn!(camera_name, role = role.label(), ?busy_for, "The camera's status poll is still inside the driver");
        return ApiError::Internal(format!(
            "Camera '{}' is busy: its status poll has been inside the camera driver for {:.1} s",
            camera_name,
            busy_for.as_secs_f64()
        ));
    }

    // Nobody holds it, so it is lost. The caller hands back `None`, which reopens it the way
    // a device fault would — not this function: tearing down from inside the take made the
    // guide loop's teardown wait out `guide_task::stop`'s budget on the loop asking.
    error!(
        camera_name,
        role = role.label(),
        "The camera handle is missing and nothing holds it; handing the camera to recovery"
    );
    ApiError::CameraHandleLost {
        camera: camera_name.to_string(),
    }
}

/// Return the handle after a capture session ends. If the capture thread
/// lost the handle (e.g., panicked), we transition straight to Disconnected.
pub async fn return_from_capture(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
    camera: Option<Box<dyn Camera>>,
) {
    match camera {
        Some(mut cam) => {
            // The disconnect may already have finished without this handle: both
            // `guide_task::stop` and the capture watchdog give up after a budget and let
            // it proceed. Parking a live handle in a slot whose camera is gone leaves an
            // open device nothing will ever close, and re-stamping the phase resurrects a
            // camera the UI has retired. The same budget lets a *reconnect* land first,
            // which is why the occupied slot is checked too — same reasoning as
            // `monitor::with_camera_bounded`, and `DeviceLease` makes closing the
            // superseded handle a no-op against the live device.
            let superseded = state.camera_in_role(role).map(|c| c.info.name).as_deref()
                != Some(camera_name)
                || state.slot(role).holds_handle()
                || state.slot(role).is_recovering();
            if superseded {
                warn!(
                    camera_name,
                    role = role.label(),
                    "Handle returned after its camera was replaced or disconnected; closing it"
                );
                if let Err(e) = cam.close() {
                    warn!(error = %e, "camera.close() failed — dropping anyway");
                }
                state.slot(role).notify_handle_returned();
                return;
            }

            *state
                .slot(role)
                .handle
                .lock()
                .expect("camera handle mutex poisoned") = Some(cam);
            state.slot(role).notify_handle_returned();

            // A Disconnect that outlasted this capture's wind-down left its warm-up waiting
            // for the handle. Only now can the monitor command the cooler.
            if state.slot(role).warmup().is_some() {
                let fast = state.settings.snapshot().profile_for(role).cooler_fast_mode;
                send_monitor_cmd(state, role, MonitorCmd::ResumeAfterCapture);
                send_monitor_cmd(state, role, MonitorCmd::StartWarmup { fast });
                return;
            }

            // Decide phase: if cooling is enabled and we're not yet near target,
            // precooling; otherwise idle. We use the last cached status as a
            // cheap proxy — the monitor will correct it on the next poll.
            let profile = state.settings.snapshot().profile_for(role);
            let target = profile.target_temp_c;
            let cooler_enabled = profile.cooler_enabled;
            let fast = profile.cooler_fast_mode;

            let next_phase = if let Some(t) = target {
                if cooler_enabled {
                    match state.get_camera_status(camera_name) {
                        Some(status)
                            if (status.temperature_c - t).abs() <= crate::camera::PRECOOL_TOLERANCE_C =>
                        {
                            CameraPhase::Idle
                        }
                        _ => CameraPhase::Precooling,
                    }
                } else {
                    CameraPhase::Idle
                }
            } else {
                CameraPhase::Idle
            };

            state.set_camera_phase(role, camera_name, next_phase);
            send_monitor_cmd(state, role, MonitorCmd::ResumeAfterCapture);

            // If we're back in Precooling after capture, the capture thread's
            // per-frame apply pushed the final target to hardware — which
            // means the monitor needs to re-seed its cooldown ramp from the
            // current sensor temp if the sensor is still settling.
            if next_phase == CameraPhase::Precooling {
                send_monitor_cmd(
                    state,
                    role,
                    MonitorCmd::UpdateCoolerTarget {
                        enabled: cooler_enabled,
                        target,
                        fast,
                    },
                );
            }
        }
        None => {
            // Capture thread crashed or returned without the handle.
            warn!(
                camera_name,
                role = role.label(),
                "Capture ended without returning handle; cleaning up"
            );
            // A camera the observer is disconnecting is not one to reopen.
            let cause = if state.slot(role).warmup().is_some() {
                DisconnectCause::Requested
            } else {
                DisconnectCause::DeviceFault
            };
            finalize_disconnect(state, role, camera_name, cause).await;
        }
    }
}
