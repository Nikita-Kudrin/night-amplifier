//! Quiet recovery: a camera that drops out is suspended, not disconnected.
//!
//! Every dropout on 2026-09-07 was back within seconds, yet each one tore the session
//! down — camera removed, selection cleared, guide stream blanked, solver re-pointed,
//! "Disconnected" on screen — and then waited a fixed 5 s. Twice the observer clicked
//! Connect in that window, which cancelled the recovery outright.
//!
//! Suspending keeps everything the UI and the solver are looking at, drops only the
//! dead handle, and lets the supervisor (`reconnect`) reopen the device under the same
//! entry. The ladder from there: nothing reaches the user unless the reconnect takes
//! longer than `reconnect::NOTICE_AFTER`, and a full teardown happens only when it
//! gives up. The slot's [`Recovery`] state is the single record of where this stands.

use std::sync::Arc;

use tracing::{info, warn};

use super::install;
use super::lifecycle::{self, DisconnectCause};
use super::reconnect::OPEN_TIMEOUT;
use crate::camera::identity::{self, DeviceIdentity};
use crate::camera::{DeviceCatalog, OpenedCamera};
use crate::server::capture::watchdog::release_faulted_handle;
use crate::server::error::{ApiError, ApiResult};
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CameraPhase, CameraRole, CaptureState, ConnectedCameraInfo, InstallOutcome,
    MonitorCmd, Recovery, SuspendVerdict,
};

/// Suspend `role`'s camera after a device fault and make sure a supervisor is reopening
/// it. Returns `false` when the fault should instead end the session: automatic
/// reconnect is off, or the camera is no longer the one in that role.
pub(super) async fn suspend(state: &Arc<AppState>, role: CameraRole, camera_name: &str) -> bool {
    if !state.settings.read().await.auto_reconnect {
        return false;
    }
    let Some(recorded) = state
        .camera_in_role(role)
        .await
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
    slot.set_monitor_tx(None);
    // The guide stream keeps its last frame on screen.
    if role == CameraRole::Guide {
        crate::server::capture::guide_task::stop(state).await;
    }
    let stale = slot.handle.lock().expect("camera handle mutex poisoned").take();
    if let Some(camera) = stale {
        release_faulted_handle(camera, state, role);
    }
    state.clear_camera_token(role).await;
    slot.drain_ops();

    state.set_camera_phase(role, camera_name, CameraPhase::Recovering).await;
    if role == CameraRole::Main {
        pause_capture_for_recovery(state).await;
    }
    slot.notify_handle_returned();
}

/// Mark a running capture as paused rather than stopped, so the UI keeps showing the
/// session and the supervisor resumes it. A capture the observer was already stopping
/// has no resume plan and is left to end.
async fn pause_capture_for_recovery(state: &Arc<AppState>) {
    let resumable = state.session_resume_plan.read().await.is_some();
    {
        let mut session = state.session.write().await;
        if !resumable || !matches!(session.state, CaptureState::Capturing | CaptureState::Starting)
        {
            return;
        }
        session.state = CaptureState::Recovering;
    }
    let _ = state
        .events
        .send(ServerEvent::state_changed(CaptureState::Recovering));
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
        .await
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
    if is_recovering(state, recorded).await {
        lifecycle::finalize_disconnect(
            state,
            recorded.role,
            &recorded.info.name,
            DisconnectCause::RecoveryFailed,
        )
        .await;
    }
    let _ = state.events.send(ServerEvent::camera_reconnect_failed(
        recorded.info.name.clone(),
        attempts,
        reason.to_string(),
    ));
    state.send_error(format!(
        "Could not bring camera '{}' back ({}). Check the cable and reconnect.",
        recorded.info.name, reason
    ));
}

/// Reopen the camera `recorded` describes, for the reconnect supervisor.
///
/// Never by position alone: the device list is re-enumerated, only devices that can be
/// this camera are tried (see `identity::recovery_candidates`), and the opened handle
/// must match before it replaces the suspended entry. An index reopen is how, on
/// 2026-09-07, the imaging camera came back as the guide camera.
///
/// Holds the connect lock from open to install, and re-checks the slot once the open
/// returns: `disconnect` waits on that lock, so an observer's Disconnect can no longer
/// land mid-open and be undone by the install.
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
        .await
        .filter(|other| other.provider.eq_ignore_ascii_case(&recorded.provider))
        .map(|other| DeviceIdentity::of(&other.info).with_device_id(other.info.id));
    let use_simulated = state.settings.read().await.use_simulated_camera;
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
