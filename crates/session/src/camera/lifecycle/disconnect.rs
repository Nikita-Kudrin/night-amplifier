//! Disconnect: stop what holds the camera, warm a cooled sensor up, then close it.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use super::{send_monitor_cmd, sync_solver_rig, take_camera, with_camera};
use crate::camera::WARMUP_TIMEOUT;
use crate::error::{ApiError, ApiResult};
use crate::events::ServerEvent;
use crate::state::{AppState, CameraPhase, CameraRole, CaptureState, MonitorCmd};

/// A warm-up's hard deadline, kept here rather than by the monitor: a monitor that is
/// wedged, paused, or polling a camera that never answers cannot end a warm-up, and on
/// 2026-09-20 nothing else could either. Past the monitor's own `WARMUP_TIMEOUT`, which
/// normally ends it first.
const WARMUP_DEADLINE: Duration = WARMUP_TIMEOUT.saturating_add(Duration::from_secs(30));

/// How often the warm-up deadline watchdog looks.
#[cfg(not(test))]
const WARMUP_WATCH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const WARMUP_WATCH_INTERVAL: Duration = Duration::from_millis(50);

/// How long a Disconnect waits for the capture it stopped to wind down and hand the
/// handle back. The final stack is written first: up to 7.5 s on the Pi (2026-09-20).
#[cfg(not(test))]
const CAPTURE_STOP_WAIT: Duration = Duration::from_secs(15);
#[cfg(test)]
pub(in crate::camera) const CAPTURE_STOP_WAIT: Duration = Duration::from_secs(3);

/// Budget for switching a camera's cooler off and closing it without a warm-up.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a Disconnect of a cooled camera warms it up first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmupPolicy {
    /// Warm up first while the camera can be — cooled, answering, and holding a handle.
    WhenPossible,
    /// Switch the cooler off and close now. The observer's call: thermal shock to the
    /// sensor is theirs to accept.
    Skip,
}

/// Where a Disconnect left the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectOutcome {
    Disconnected,
    /// Warming up; the handle closes on its own once warm, and by the deadline at the
    /// latest. `remaining` is the time left to that deadline.
    WarmingUp { remaining: Option<Duration> },
}

/// Why a camera session is ending. Decides whether the reconnect supervisor
/// treats the loss as something to recover from.
///
/// This used to be a bare `unexpected: bool` at fourteen call sites, where
/// `true` and `false` said nothing about which situation they meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectCause {
    /// The user asked, or the session ended normally. Do not reconnect —
    /// reconnecting would fight the request that got us here.
    Requested,
    /// The camera stopped answering or reported its device gone. Recoverable
    /// in principle: suspended for the reconnect supervisor (see `recovery`).
    DeviceFault,
    /// The supervisor gave up. Ends the session like `Requested`, but keeps the guide
    /// camera's raw-frame folder, since the observer did not end the observation.
    RecoveryFailed,
}

impl DisconnectCause {
    fn should_attempt_reconnect(self) -> bool {
        matches!(self, DisconnectCause::DeviceFault)
    }
}

/// Disconnect a camera, ending its session in bounded time whatever it is doing.
///
/// The observer's Disconnect is a must (2026-09-20: a camera unplugged mid-session could be
/// neither disconnected nor started until the board was power-cycled). The imaging
/// camera's capture is stopped first, the stack saved as on any Stop. A cooled camera
/// warms up first while that can work — answering, with a handle to command — and closes
/// at once when it cannot or `warmup` says to skip it. A warm-up always ends by
/// [`WARMUP_DEADLINE`].
pub async fn disconnect(
    state: &Arc<AppState>,
    camera_id: &str,
    warmup: WarmupPolicy,
) -> ApiResult<DisconnectOutcome> {
    let Some(connected) = state.roster.get(camera_id) else {
        warn!(camera_id = %camera_id, "Attempted to disconnect non-connected camera");
        return Err(ApiError::CameraNotConnected(camera_id.to_string()));
    };
    let role = connected.role;
    let camera_name = connected.info.name;

    // The guide camera has no such tie: its loop is its own and stopping it costs the
    // session nothing but plate solving.
    let capture_winding_down = match role {
        CameraRole::Main => end_capture_for_disconnect(state, &camera_name).await,
        CameraRole::Guide => false,
    };

    // Nothing to warm up or stop: the handle is already gone, and ending the session is
    // what stops the supervisor. Behind the connect lock, which a reopen holds from
    // opening the device to installing it — tearing down past it was undone the moment
    // the open returned.
    if state.slot(role).is_recovering() {
        let _reopen_done = state.camera_connect_lock.lock().await;
        if state.slot(role).is_recovering() {
            finalize_disconnect(state, role, &camera_name, DisconnectCause::Requested).await;
            return Ok(DisconnectOutcome::Disconnected);
        }
        // It came back while we waited: disconnect it the ordinary way.
    }

    let fast = state.settings.snapshot().profile_for(role).cooler_fast_mode;

    if state.camera_phase(role) == CameraPhase::WarmingUp {
        if warmup == WarmupPolicy::Skip {
            info!(camera_id = %camera_id, "Warm-up skipped on request; disconnecting now");
            disconnect_without_warmup(state, role, &camera_name).await;
            return Ok(DisconnectOutcome::Disconnected);
        }
        // A warm-up nothing is timing would have no end.
        if state.slot(role).warmup().is_none() {
            begin_warmup(state, role, &camera_name, fast).await;
        }
        info!(camera_id = %camera_id, "Disconnect requested but already warming up");
        return Ok(warming_up(state, role));
    }

    // Stop the free-running loop before anything touches the handle, so the loop is not
    // mid-exposure when the warmup or the close arrives.
    if role == CameraRole::Guide {
        crate::capture::guide_task::stop(state).await;
    }

    // Decide whether to warm up: if the user had cooling enabled in settings
    // (current intent) OR the last status sample reported cooler_on, ramp
    // the TEC down before closing the handle. Relying on settings alone is
    // important because the monitor may not have polled yet on fresh connects.
    let cooler_enabled_in_settings = state.settings.snapshot().profile_for(role).cooler_enabled;
    let cooler_reported_on = state
        .get_camera_status(&camera_name)
        .map(|s| s.cooler_on)
        .unwrap_or(false);
    if !(cooler_enabled_in_settings || cooler_reported_on) {
        finalize_disconnect(state, role, &camera_name, DisconnectCause::Requested).await;
        return Ok(DisconnectOutcome::Disconnected);
    }

    if let Some(why) = warmup_impossible(state, role, &camera_name, warmup, capture_winding_down).await
    {
        info!(camera_id = %camera_id, why, "Disconnecting without a warm-up");
        disconnect_without_warmup(state, role, &camera_name).await;
        return Ok(DisconnectOutcome::Disconnected);
    }

    // Still saving its stack: the handle comes back through `return_from_capture`, which
    // starts the warm-up then. The deadline holds from now, in case it never does.
    if capture_winding_down {
        track_warmup(state, role, &camera_name).await;
        info!(camera_id = %camera_id, "Warm-up begins once the capture has handed the camera back");
        return Ok(warming_up(state, role));
    }

    // The monitor closes the handle and emits `CameraDisconnected` once the sensor
    // reaches WARMUP_THRESHOLD_C; the watchdog makes sure that happens by the deadline.
    begin_warmup(state, role, &camera_name, fast).await;
    info!(camera_id = %camera_id, fast, "Warmup initiated; disconnect will complete asynchronously");
    Ok(warming_up(state, role))
}

fn warming_up(state: &AppState, role: CameraRole) -> DisconnectOutcome {
    DisconnectOutcome::WarmingUp {
        remaining: state.slot(role).warmup_remaining(),
    }
}

/// Why a cooled camera cannot be warmed up, or `None` when it can. A capture still winding
/// down holds the handle and will give it back, so its empty slot is not "no handle".
async fn warmup_impossible(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
    warmup: WarmupPolicy,
    capture_winding_down: bool,
) -> Option<&'static str> {
    if warmup == WarmupPolicy::Skip {
        return Some("skipped on request");
    }
    // Unplugged, or stalling: every ramp step would fail, for minutes.
    if crate::camera::health::has_recent_fault(state, role, camera_name) {
        return Some("the camera's last calls failed");
    }
    if capture_winding_down {
        return None;
    }
    // Waits out a monitor poll in progress. Empty after that, nothing can command the TEC.
    if !handle_reachable(state, role) || with_camera(state, role, |_| ()).await.is_none() {
        return Some("there is no camera handle to command");
    }
    None
}

/// Whether the handle is in the slot or on its way back from a monitor call. Disconnect
/// asks after stopping the capture and the guide loop, so nothing else can bring it back.
fn handle_reachable(state: &AppState, role: CameraRole) -> bool {
    let slot = state.slot(role);
    slot.holds_handle() || slot.monitor_call_age().is_some()
}

/// Stop the imaging capture so its camera can be disconnected, and wait — bounded — for
/// the pipeline to wind down and hand the handle back. Returns `true` when it is still
/// winding down at the end of the wait.
///
/// The sub in flight is cut short: Stop alone lets it finish, and a 300 s deep-sky sub
/// outlasted the wait, so the cooled camera was closed with no warm-up.
async fn end_capture_for_disconnect(state: &Arc<AppState>, camera_name: &str) -> bool {
    // A capture paused for recovery ends with the same compare-and-set a resume makes:
    // checked only, a reopen finishing meanwhile resumed the capture on the camera this
    // disconnect was warming up, and cancelled the warm-up.
    if state.end_paused_capture() {
        return false;
    }
    let running = |capture: CaptureState| {
        matches!(
            capture,
            CaptureState::Starting | CaptureState::Capturing | CaptureState::Stopping
        )
    };
    if !running(state.capture_state()) {
        return false;
    }
    info!(camera_name, "Disconnect requested during a capture; stopping the capture first");
    crate::services::CaptureService::stop_capture(state).await;
    state.cancel_active_exposure(CameraRole::Main).await;

    let deadline = tokio::time::Instant::now() + CAPTURE_STOP_WAIT;
    while running(state.capture_state()) {
        if tokio::time::Instant::now() >= deadline {
            warn!(camera_name, "The capture is still stopping; the camera disconnects once it has");
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Put the camera into `WarmingUp`, start the watchdog, and have the monitor drive it.
pub(super) async fn begin_warmup(state: &Arc<AppState>, role: CameraRole, camera_name: &str, fast: bool) {
    track_warmup(state, role, camera_name).await;
    send_monitor_cmd(state, role, MonitorCmd::StartWarmup { fast });
}

/// Put the camera into `WarmingUp` and start the watchdog that ends the warm-up by
/// [`WARMUP_DEADLINE`] whatever the monitor manages. Nothing drives it yet: callers whose
/// monitor can command the camera now use [`begin_warmup`].
async fn track_warmup(state: &Arc<AppState>, role: CameraRole, camera_name: &str) {
    let warmup = state.slot(role).begin_warmup(Instant::now() + WARMUP_DEADLINE);
    state.set_camera_phase(role, camera_name, CameraPhase::WarmingUp);
    // The guide loop has stopped, and the close is minutes away: the viewers would sit on
    // a frozen frame. After the phase moves, for the reason `finalize_disconnect` removes
    // the entry first (see `ViewedCamera::select`).
    if role == CameraRole::Guide {
        state.fall_back_to_main_view();
    }

    let state = Arc::clone(state);
    let camera_name = camera_name.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(WARMUP_WATCH_INTERVAL).await;
            // Ended — complete, cancelled by a Start, skipped — or replaced by a later one.
            let Some(current) = state.slot(role).warmup().filter(|w| w.epoch == warmup.epoch)
            else {
                return;
            };
            if Instant::now() < current.deadline {
                continue;
            }
            let ours = state
                .camera_in_role(role)
                .is_some_and(|camera| camera.info.name == camera_name);
            if !ours || state.camera_phase(role) != CameraPhase::WarmingUp {
                return;
            }
            warn!(
                camera_name,
                role = role.label(),
                "The warm-up overran its deadline; disconnecting without finishing it"
            );
            disconnect_without_warmup(&state, role, &camera_name).await;
            return;
        }
    });
}

/// End the session now: cooler off on the way out, then close — bounded, on a thread of
/// its own, because the cameras this is for are the ones that may not answer.
async fn disconnect_without_warmup(state: &Arc<AppState>, role: CameraRole, camera_name: &str) {
    // The monitor first, so it neither polls the handle away nor reads the slot emptied
    // below as a lost handle.
    send_monitor_cmd(state, role, MonitorCmd::Shutdown);
    state.slot(role).set_monitor_tx(None);
    if role == CameraRole::Guide {
        crate::capture::guide_task::stop(state).await;
    }

    let camera = if handle_reachable(state, role) {
        take_camera(state, role).await
    } else {
        None
    };
    if let Some(camera) = camera {
        let released = state
            .slot(role)
            .sdk_calls
            .run_bounded(RELEASE_TIMEOUT, move || {
                let mut camera = camera;
                if camera.info().has_cooler {
                    if let Err(e) = camera.set_cooler(false) {
                        warn!(error = %e, "Could not switch the cooler off before closing");
                    }
                }
                if let Err(e) = camera.close() {
                    warn!(error = %e, "camera.close() failed — dropping anyway");
                }
            })
            .await;
        if released.is_err() {
            warn!(
                camera_name,
                "The camera did not answer while being switched off; its handle closes when the call returns"
            );
        }
    }
    finalize_disconnect(state, role, camera_name, DisconnectCause::Requested).await;
}

/// Close the handle, drop state, broadcast `CameraDisconnected`, and
/// transition phase to `Disconnected`. Used by both immediate-disconnect
/// (no warmup) and warmup-completion paths.
pub async fn finalize_disconnect(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
    cause: DisconnectCause,
) {
    // A report about a camera the role no longer holds — a loop that outlived its
    // camera's replacement — must not tear down the camera that replaced it.
    if let Some(current) = state.camera_in_role(role) {
        if current.info.name != camera_name {
            warn!(camera_name, current = %current.info.name, role = role.label(), ?cause, "Ignoring a disconnect for a camera this role no longer holds");
            return;
        }
    }
    if cause == DisconnectCause::DeviceFault
        && crate::camera::recovery::suspend(state, role, camera_name).await
    {
        return;
    }
    let was_recovering = state.slot(role).end_recovery();
    state.slot(role).end_warmup();

    // Shut down the monitor thread first.
    send_monitor_cmd(state, role, MonitorCmd::Shutdown);
    state.slot(role).set_monitor_tx(None);

    // The guide loop must let go of the handle before we close it. On the requested
    // path `disconnect` already stopped it; on a device fault this is where it stops.
    if role == CameraRole::Guide {
        crate::capture::guide_task::stop(state).await;
        state.guide_stream.clear();
    }

    // Close and drop the handle.
    if let Some(mut cam) = state
        .slot(role)
        .handle
        .lock()
        .expect("camera handle mutex poisoned")
        .take()
    {
        if let Err(e) = cam.close() {
            warn!(error = %e, "camera.close() failed — dropping anyway");
        }
    }
    state.clear_camera_token(role).await;
    // A hardware call queued for this slot's owner — the dew heater switch, so far —
    // and never drained because the camera disconnected before its next exposure must
    // not be replayed against whatever connects into this role next.
    state.slot(role).drain_ops();

    // Entry, selection, status and phase leave in one transaction. The phase used to
    // follow only after `sync_solver_rig`, and a monitor call returning in that window
    // parked its handle in the emptied slot instead of closing it.
    let removed = state.roster.remove(role, camera_name);
    // After the removal, never before: see `ViewedCamera::select`. A fault that recovery
    // suspended returned above, so a USB hiccup keeps the viewers on the guide camera.
    if role == CameraRole::Guide {
        state.fall_back_to_main_view();
    }

    state.slot(role).notify_handle_returned();
    // Re-point the solver: losing the guide camera hands solving back to the main one,
    // along with the main scope's optics.
    sync_solver_rig(state).await;
    state.announce_camera_phase(role, camera_name, CameraPhase::Disconnected);
    let _ = state
        .events
        .send(ServerEvent::camera_disconnected(camera_name));

    info!(camera_name, role = role.label(), "Camera disconnected");

    if was_recovering && role == CameraRole::Main {
        state.end_paused_capture();
    }

    if !cause.should_attempt_reconnect() {
        // A deliberate disconnect ends the observation, so the folder it was filling is
        // not something a later session should rejoin.
        if cause == DisconnectCause::Requested {
            state.slot(role).set_raw_session(None);
        }
        return;
    }
    // Only reached when recovery declined to suspend — automatic reconnect is off — so
    // the supervisor's job is to say so.
    let Some(removed) = removed else {
        return;
    };
    crate::camera::reconnect::spawn(state, removed);
}
