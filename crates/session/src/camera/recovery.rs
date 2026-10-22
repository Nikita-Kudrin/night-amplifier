//! Quiet recovery: a camera that drops out is suspended, not disconnected.
//!
//! Every dropout on 2026-09-07 was back within seconds, yet each tore the session down
//! (handle removed, guide blanked, solver re-pointed) after a fixed 5 s wait — twice
//! the observer's Connect click landed in that window and cancelled recovery outright.
//! Suspending keeps the UI/solver's view intact and lets `reconnect` reopen the device
//! under the same entry; nothing reaches the user unless that exceeds
//! `reconnect::NOTICE_AFTER`. The slot's [`Recovery`] state records where this stands.

use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use super::install;
use super::lifecycle::{self, DisconnectCause};
use super::reconnect::OPEN_TIMEOUT;
use night_amplifier_core::camera::identity::{self, DeviceIdentity};
use night_amplifier_core::camera::{DeviceCatalog, OpenedCamera};
use crate::capture::watchdog::{capture_watchdog_timeout, release_faulted_handle};
use crate::error::{ApiError, ApiResult};
use crate::state::{
    AppState, CameraPhase, CameraRole, CaptureState, ConnectedCameraInfo, InstallOutcome,
    MonitorCmd, Recovery, RecoveryStatus, SuspendVerdict,
};

/// Suspend `role`'s camera after a device fault and make sure a supervisor is reopening
/// it. Returns `false` when the fault should instead end the session: automatic
/// reconnect is off, or the camera is no longer the one in that role.
pub(super) async fn suspend(state: &Arc<AppState>, role: CameraRole, camera_name: &str) -> bool {
    if !state.settings.snapshot().auto_reconnect {
        return false;
    }
    let Some(recorded) = state
        .camera_in_role(role)
        .filter(|camera| camera.info.name == camera_name)
    else {
        return false;
    };
    match state.slot(role).begin_suspend() {
        SuspendVerdict::Suspended => {}
        // The capture loop and the monitor can both see one loss.
        SuspendVerdict::AlreadySuspended => return true,
        SuspendVerdict::Deferred => {
            info!(camera_name, role = role.label(), "Reopened camera failed while being installed; recovery goes round again");
            return true;
        }
    }

    warn!(camera_name, role = role.label(), "Camera lost; reopening it without disconnecting");
    detach(state, role, camera_name).await;
    super::reconnect::spawn(state, recorded);
    true
}

/// Drop everything tied to a suspended camera's lost handle, keeping its entry.
///
/// Also run by the supervisor for a handle that failed during its own install, which is
/// why this is not inlined into [`suspend`].
pub(super) async fn detach(state: &Arc<AppState>, role: CameraRole, camera_name: &str) {
    let slot = state.slot(role);
    lifecycle::send_monitor_cmd(state, role, MonitorCmd::Shutdown);
    slot.retire_monitor();
    // The guide stream keeps its last frame on screen.
    if role == CameraRole::Guide {
        crate::capture::guide_task::stop(state).await;
    }
    let stale = slot.handle.lock().expect("camera handle mutex poisoned").take();
    if let Some(camera) = stale {
        release_faulted_handle(camera, state, role);
    }
    state.clear_camera_token(role).await;
    slot.drain_ops();

    state.set_camera_phase(role, camera_name, CameraPhase::Recovering);
    if role == CameraRole::Main {
        pause_capture_for_recovery(state).await;
    }
    slot.notify_handle_returned();
}

/// Mark a running capture as paused rather than stopped, so the UI keeps showing the
/// session and the supervisor resumes it. A capture the observer was already stopping
/// has no resume plan and is left to end.
async fn pause_capture_for_recovery(state: &Arc<AppState>) {
    if !state.resume.has_plan() {
        return;
    }
    let _ = state.transition_capture_state(|current| {
        matches!(current, CaptureState::Capturing | CaptureState::Starting)
            .then_some(CaptureState::Recovering)
    });
}

/// Close the other role's camera when it runs on the same SDK, so `recorded` has that SDK
/// to itself until its first frame; the sibling's supervisor is held until then
/// ([`end_isolation`]), bounded by [`isolation_deadline`].
///
/// 2026-10-12: the Ares-C PRO stayed frameless through 20 reopens and a reconnect while
/// the Neptune-C II was open on the same PlayerOne SDK, and streamed — beside the Neptune,
/// once that was reopened — the moment the Neptune was closed. Released on the open alone,
/// the sibling reopened (and rescanned the bus) during the first exposure. Never an
/// imaging camera mid-capture, nor one warming up after the observer's Disconnect: that
/// one would be reopened and cooled again.
pub(super) async fn isolate_from_sibling(state: &Arc<AppState>, recorded: &ConnectedCameraInfo) {
    let role = recorded.role.other();
    let Some(sibling) = state
        .camera_in_role(role)
        .filter(|sibling| sibling.provider.eq_ignore_ascii_case(&recorded.provider))
    else {
        return;
    };
    if state.slot(role).is_recovering() {
        return;
    }
    if state.camera_phase(role) == CameraPhase::WarmingUp || state.slot(role).warmup().is_some() {
        info!(camera = %recorded.info.name, sibling = %sibling.info.name, "Not closing the other camera: it is warming up to disconnect");
        return;
    }
    if role == CameraRole::Main && !matches!(state.capture_state(), CaptureState::Idle | CaptureState::Error) {
        info!(camera = %recorded.info.name, sibling = %sibling.info.name, "Not closing the imaging camera mid-capture to reopen this one alone");
        return;
    }
    warn!(
        camera = %recorded.info.name,
        sibling = %sibling.info.name,
        provider = %recorded.provider,
        "The camera has delivered no frame since it was last opened; reopening it with the other camera on its SDK closed"
    );
    // Before the suspension, which spawns the supervisor this holds.
    state.slot(role).reopen_hold.hold_until(isolation_deadline(state, recorded));
    if !suspend(state, role, &sibling.info.name).await {
        state.slot(role).reopen_hold.release();
        return;
    }
    if !state.slot(role).sdk_calls.wait_drained(super::reconnect::ABANDONED_CALL_WAIT).await {
        info!(sibling = %sibling.info.name, "The other camera's close is still running; reopening past it");
    }
}

/// The latest a sibling stays closed for `recorded`: its reopen, its install, and one
/// whole first frame, watchdog included.
fn isolation_deadline(state: &AppState, recorded: &ConnectedCameraInfo) -> Instant {
    let settings = state.settings.snapshot();
    let config = settings.to_capture_config_with(&settings.profile_for(recorded.role), recorded.role);
    Instant::now() + OPEN_TIMEOUT * 2 + capture_watchdog_timeout(&config, &recorded.info)
}

/// Let the camera that isolation closed for `role`'s camera reopen: that camera framed,
/// or its recovery is over without one.
pub(super) fn end_isolation(state: &AppState, role: CameraRole) {
    state.slot(role.other()).reopen_hold.release();
}

/// Whether `recorded` is still the camera suspended in its role — the supervisor's
/// check before each attempt, since the observer can disconnect it or connect another
/// camera in its place at any time.
pub(super) async fn is_recovering(state: &Arc<AppState>, recorded: &ConnectedCameraInfo) -> bool {
    if state.slot(recorded.role).recovery() != Recovery::Suspended {
        return false;
    }
    state
        .camera_in_role(recorded.role)
        .is_some_and(|camera| camera.id == recorded.id && camera.info.name == recorded.info.name)
}

/// The supervisor could not bring the camera back: tear the session down and tell the
/// observer, which is the first they hear of it.
pub(super) async fn give_up(
    state: &Arc<AppState>,
    recorded: &ConnectedCameraInfo,
    attempts: u32,
    reason: &str,
) {
    warn!(camera = %recorded.info.name, %reason, "Giving up on reconnecting the camera");
    end_isolation(state, recorded.role);
    // Before the role empties: recorded after, a sync in between showed the camera gone
    // with no reason, and a test that saw the empty role read the status before it landed.
    state.recovery_status.set(
        recorded.role,
        RecoveryStatus::Failed {
            name: recorded.info.name.clone(),
            attempts,
            reason: reason.to_string(),
        },
    );
    if is_recovering(state, recorded).await {
        lifecycle::finalize_disconnect(
            state,
            recorded.role,
            &recorded.info.name,
            DisconnectCause::RecoveryFailed,
        )
        .await;
    }
    state.send_error(format!(
        "Could not bring camera '{}' back ({}). Check the cable and reconnect.",
        recorded.info.name, reason
    ));
}

/// Reopen the camera `recorded` describes, for the reconnect supervisor.
///
/// Never by position alone: the device list is re-enumerated, only devices that can be
/// this camera are tried (`identity::recovery_candidates`), and the opened handle must
/// match before it replaces the suspended entry — an index reopen is how the imaging camera came back as the guide camera on 2026-09-07.
///
/// Holds the connect lock from open to install and re-checks the slot after open
/// returns, so an observer's Disconnect can't land mid-open and be undone by install.
pub(super) async fn reopen_for_recovery(
    state: &Arc<AppState>,
    recorded: &ConnectedCameraInfo,
) -> ApiResult<ConnectedCameraInfo> {
    let connect_guard = state.camera_connect_lock.lock().await;
    if !is_recovering(state, recorded).await {
        return Err(ApiError::CameraNotConnected(recorded.id.clone()));
    }
    let slot = state.slot(recorded.role);
    if slot.pending_opens.in_flight() > 0 {
        return Err(ApiError::CameraOpenFailed(
            "an earlier reopen is still inside the vendor SDK".to_string(),
        ));
    }

    let other_role = state
        .camera_in_role(recorded.role.other())
        .filter(|other| other.provider.eq_ignore_ascii_case(&recorded.provider))
        .map(|other| DeviceIdentity::of(&other.info).with_device_id(other.info.id));
    let use_simulated = state.settings.snapshot().use_simulated_camera;
    let catalog = Arc::clone(&state.device_catalog);
    let expected = recorded.clone();

    // Listed, opened and probed on one bounded call. One past the timeout keeps running;
    // `pending_opens` holds further reopens back until it returns, and its handle is
    // closed before it leaves the count.
    let (opened, index) = slot
        .pending_opens
        .run_bounded(OPEN_TIMEOUT, move || {
            let (mut opened, index) =
                open_matching(catalog.as_ref(), &expected, other_role.as_ref(), use_simulated)?;
            install::verify_responsive(&mut opened.camera, &expected.id)?;
            Ok::<_, ApiError>((opened, index))
        })
        .await
        .map_err(|e| install::stage_failed("reopening", e))??;

    if !is_recovering(state, recorded).await || !slot.begin_install() {
        release_faulted_handle(opened.camera, state, recorded.role);
        return Err(ApiError::CameraNotConnected(recorded.id.clone()));
    }
    // Under the id it was recovered for, even if its position moved: without a serial
    // the id *is* a position, and the other role's entry may be keyed by the new one.
    let installed = install::install_camera(
        state,
        &recorded.id,
        recorded.role,
        opened.provider,
        index,
        opened.camera,
        Some(recorded),
    )
    .await
    .inspect_err(|_| {
        // Nothing was registered, and the handle went with the setup that did not return.
        // Suspended again, so the next attempt may install — unless recovery was ended.
        slot.abandon_install();
    })?;

    match slot.finish_install() {
        InstallOutcome::Healthy => Ok(installed),
        InstallOutcome::Refaulted => {
            drop(connect_guard);
            detach(state, recorded.role, &recorded.info.name).await;
            Err(ApiError::CameraOpenFailed(
                "the camera failed again while it was being reinstalled".to_string(),
            ))
        }
        InstallOutcome::Ended => {
            drop(connect_guard);
            warn!(camera = %recorded.info.name, "Recovery ended during the install; disconnecting what it installed");
            lifecycle::finalize_disconnect(
                state,
                recorded.role,
                &recorded.info.name,
                DisconnectCause::Requested,
            )
            .await;
            Err(ApiError::CameraNotConnected(recorded.id.clone()))
        }
    }
}

/// Try each device that can be `recorded`'s camera, keeping the first that turns out
/// to be it. A device that opens as some other camera is dropped, which closes it.
fn open_matching(
    catalog: &dyn DeviceCatalog,
    recorded: &ConnectedCameraInfo,
    other_role: Option<&DeviceIdentity>,
    use_simulated: bool,
) -> ApiResult<(OpenedCamera, usize)> {
    let listed = catalog
        .identities(&recorded.provider, use_simulated)
        .map_err(|e| ApiError::CameraOpenFailed(e.to_string()))?;
    let expected = DeviceIdentity::of(&recorded.info);
    let candidates =
        identity::recovery_candidates(&listed, &expected, recorded.index, other_role);

    let mut failure = ApiError::CameraOpenFailed(format!("'{}' is not connected", expected.name));
    for index in candidates {
        let opened = match catalog.open(&recorded.provider, index, use_simulated) {
            Ok(opened) => opened,
            Err(e) => {
                failure = ApiError::CameraOpenFailed(e.to_string());
                continue;
            }
        };
        let found = DeviceIdentity::of(opened.camera.info());
        if expected.matches(&found) {
            return Ok((opened, index));
        }
        warn!(
            expected = %expected.name,
            found = %found.name,
            index,
            "Reopened a different camera than the one being recovered; closing it"
        );
        drop(opened);
        failure = ApiError::CameraIdentityMismatch {
            expected: expected.name.clone(),
            found: found.name,
        };
    }
    Err(failure)
}
