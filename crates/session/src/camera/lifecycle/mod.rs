//! Public API for camera connect/disconnect/handoff orchestration.
//!
//! This layer owns the camera handle in each role's `CameraSlot` and coordinates with
//! that slot's monitor thread. `CameraService`, the capture loop and
//! the guide loop delegate to these functions rather than opening/closing handles
//! directly. Every entry point names a [`CameraRole`]: with an imaging camera and a
//! guide camera connected at once, "the camera" is no longer an answer.
//!
//! Connect, handle access and settings live here; [`disconnect`] and the capture
//! hand-off ([`take_for_capture`], [`return_from_capture`]) in their own modules.

mod disconnect;
mod hand_off;

pub use disconnect::{
    disconnect, finalize_disconnect, DisconnectCause, DisconnectOutcome, WarmupPolicy,
};
pub use hand_off::{return_from_capture, take_for_capture};
#[cfg(test)]
pub(super) use disconnect::CAPTURE_STOP_WAIT;

use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use super::install;
use night_amplifier_core::camera::identity::{self, CameraIdError};
use night_amplifier_core::camera::{Camera, CameraInfo};
use crate::error::{ApiError, ApiResult};
use crate::state::{
    AppState, CameraCaptureProfile, CameraPhase, CameraRole, CaptureSettings, CaptureState,
    ConnectedCameraInfo, MonitorCmd,
};

/// How long a caller waits for the monitor to hand the camera handle back
/// before giving up. Slightly over the monitor's own `FFI_CALL_TIMEOUT`, so a
/// call that is merely slow is waited out and only a genuine stall gives up.
const HANDLE_WAIT_TIMEOUT: Duration = Duration::from_millis(3_500);

/// Take a slot's camera handle, waiting for the monitor to give it back if it
/// currently has it checked out for a bounded call.
///
/// A slot's handle being `None` is ambiguous on its own: it means either "no camera
/// connected" or "the monitor is mid-poll". Treating the second as the first is
/// what made a capture start fail, and a cooler slider move vanish, whenever
/// they landed inside a poll window.
pub(crate) async fn take_camera(
    state: &Arc<AppState>,
    role: CameraRole,
) -> Option<Box<dyn Camera>> {
    with_handle_slot(state, role, |slot| slot.take()).await
}

/// Run `f` against a slot's live handle, waiting for the monitor to return it if
/// necessary. `None` means no handle arrived within `HANDLE_WAIT_TIMEOUT`.
pub(crate) async fn with_camera<T>(
    state: &Arc<AppState>,
    role: CameraRole,
    f: impl FnOnce(&mut Box<dyn Camera>) -> T,
) -> Option<T> {
    let mut f = Some(f);
    with_handle_slot(state, role, |slot| {
        let cam = slot.as_mut()?;
        Some(f.take().expect("closure consumed once")(cam))
    })
    .await
}

/// Shared wait loop: register for the hand-back signal, try `f`, and sleep
/// until either the monitor signals or the budget runs out. Registering before
/// the check is what stops a hand-back that lands between them from being lost.
///
/// A recovering slot ends the wait at once: its handle is gone, and the one a reopen
/// installs is in the slot before recovery lets go of it, so `f` is tried first.
async fn with_handle_slot<T>(
    state: &Arc<AppState>,
    role: CameraRole,
    mut f: impl FnMut(&mut Option<Box<dyn Camera>>) -> Option<T>,
) -> Option<T> {
    let slot = state.slot(role);
    let deadline = tokio::time::Instant::now() + HANDLE_WAIT_TIMEOUT;
    loop {
        let notified = slot.handle_returned.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        {
            let mut guard = slot.handle.lock().expect("camera handle mutex poisoned");
            if let Some(value) = f(&mut guard) {
                return Some(value);
            }
        }
        if slot.is_recovering() {
            return None;
        }

        tokio::select! {
            _ = &mut notified => {}
            _ = tokio::time::sleep_until(deadline) => return None,
        }
    }
}

/// Open a camera into `role`, store the handle long-term, and (optionally) begin
/// pre-cooling. Replaces the old `CameraService::connect_camera` behavior
/// that dropped the handle immediately after probing `CameraInfo`.
///
/// The rig holds at most one camera per role. A role already taken by a *different*
/// camera is resolved by [`vacate_role`]: swapped while the incumbent is idle, refused
/// while it is capturing or warming up.
pub async fn connect(
    state: &Arc<AppState>,
    camera_id: &str,
    role: CameraRole,
) -> ApiResult<ConnectedCameraInfo> {
    // Serialize connects, and with them the supervisor's `reopen_for_recovery`: two
    // concurrent opens of one device would both pass the checks below, and the second
    // would displace — and therefore close — the first.
    let _connect_guard = state.camera_connect_lock.lock().await;

    // Already connected? Return the existing info — matches the prior
    // idempotent connect behavior. Connecting an already-connected camera into the
    // *other* role is a different request and is refused: one device cannot be both
    // the imaging camera and the guide camera.
    if let Some(info) = state.roster.get(camera_id) {
        // A camera being recovered is still this camera: the answer is the entry, and the
        // supervisor carries on reopening it.
        if info.role == role {
            return Ok(info);
        }
        return Err(ApiError::CameraRoleMismatch {
            camera: info.info.name,
            held: info.role.label(),
            requested: role.label(),
        });
    }

    let (provider, locator) = identity::parse_camera_id(camera_id).map_err(|e| match e {
        CameraIdError::Format => ApiError::InvalidCameraIdFormat,
        CameraIdError::Locator => ApiError::InvalidCameraIndex,
    })?;
    // A late open takes the device lease when it returns, superseding whatever was
    // opened in between — the same reason recovery waits on these.
    if state.slot(role).pending_opens.in_flight() > 0 {
        return Err(ApiError::CameraOpenFailed(format!(
            "an earlier open of the {} camera is still inside the vendor SDK",
            role.label()
        )));
    }
    let use_simulated = state.settings.snapshot().use_simulated_camera;

    let (index, listed) = install::locate(state, role, provider, &locator, use_simulated).await?;
    install::refuse_device_of_other_role(state, role, provider, index, &listed).await?;
    vacate_role(state, role).await?;

    let opened = install::open_verified(state, role, camera_id, provider, index, &locator, use_simulated)
        .await
        .inspect_err(|e| error!(camera_id = %camera_id, error = %e, "Failed to open camera"))?;
    crate::camera::health::forget_restart_history(state, role, &opened.camera.info().name);
    install::install_camera(state, camera_id, role, opened.provider, index, opened.camera, None).await
}

/// Make `role` free for a new camera, or explain why it cannot be.
///
/// A camera mid-capture or mid-warmup is doing something the user asked for and that
/// cannot be interrupted safely — a warmup cut short closes a handle with the sensor
/// still cold. An idle one is just occupying the position, so it is disconnected and the
/// new camera takes its place.
pub(crate) async fn vacate_role(state: &Arc<AppState>, role: CameraRole) -> ApiResult<()> {
    let Some(incumbent) = state.camera_in_role(role) else {
        return Ok(());
    };

    let phase = state.camera_phase(role);
    let capture_state = state.capture_state();
    let busy_capturing = role == CameraRole::Main
        && matches!(
            capture_state,
            CaptureState::Capturing | CaptureState::Starting | CaptureState::Recovering
        );

    // `Guiding` is deliberately absent: a guide loop never ends on its own, so refusing
    // it would mean a guide camera could never be replaced at all. `finalize_disconnect`
    // stops the loop before it touches the handle.
    if phase == CameraPhase::Capturing || phase == CameraPhase::WarmingUp || busy_capturing {
        return Err(ApiError::CameraRoleBusy {
            role: role.label(),
            camera: incumbent.info.name.clone(),
        });
    }

    info!(
        role = role.label(),
        replacing = %incumbent.info.name,
        "Role already taken by an idle camera — disconnecting it first"
    );
    finalize_disconnect(
        state,
        role,
        &incumbent.info.name,
        DisconnectCause::Requested,
    )
    .await;
    Ok(())
}

/// Point the plate solver at whichever camera is currently the solve source, along with
/// that camera's optics.
///
/// One place decides both, because they have to agree: naming the guide camera while
/// still handing over the main scope's focal length is worse than naming neither.
pub(crate) async fn sync_solver_rig(state: &Arc<AppState>) {
    let solve_camera = match state.camera_in_role(CameraRole::Guide) {
        Some(guide) => Some(guide),
        None => state.camera_in_role(CameraRole::Main),
    };
    let camera_name = solve_camera.map(|info| info.info.name);

    let telescope = {
        let settings = state.settings.snapshot();
        settings.solver_telescope(camera_name.as_deref())
    };

    // One call, not two: the camera and the optics are a single fact about the rig, and
    // the solver's remembered FOV is keyed on both. Applying them separately made the
    // solver resolve once against a pair that never existed — the new camera behind the
    // old camera's focal length — and that resolve *discards* a remembered FOV whose
    // camera disagrees, so the intermediate state could delete the outgoing rig's
    // measurement on the way past. See `PushToSolverPlugin::set_rig`.
    crate::services::PushToService::set_rig(state, camera_name, telescope).await;
}

/// Build the `HashMap` key used to store per-camera capture profiles.
///
/// `provider` + `model` is what the user intuits as the camera's identity
/// ("PlayerOne/Neptune-C II"), but it is not enough on its own now that two cameras are
/// connected at once: two bodies of the same model, one imaging and one guiding, would
/// share one entry and overwrite each other's exposure every time either was edited.
/// The guide role therefore gets its own suffix — and the main role deliberately does
/// not, so every profile already on disk keeps its key and its values.
pub fn camera_profile_key(provider: &str, camera_name: &str, role: CameraRole) -> String {
    match role {
        CameraRole::Main => format!("{}/{}", provider, camera_name),
        CameraRole::Guide => format!("{}/{}#{}", provider, camera_name, role.label()),
    }
}

/// Swap the per-camera profile for `key` into `role`'s live hardware fields (flat
/// `CaptureSettings` for Main, `guide_camera` for Guide — both can be connected at once
/// and can't share one set of values), seeding a fresh profile if none exists yet.
///
/// Either way the profile is clamped to what this camera can do, and the clamped copy
/// written back to the map — clamping the *stored* path too repairs a profile persisted
/// out of range: files written before `CameraCaptureProfile` had a real `Default` hold
/// `exposure_us: 0, bin: 0`, which `CaptureConfig::validate` rejects on every frame.
pub fn apply_camera_profile_on_connect(
    settings: &mut CaptureSettings,
    key: String,
    role: CameraRole,
    info: &CameraInfo,
) {
    let mut profile = match settings.camera_profiles.get(&key) {
        Some(stored) => stored.clone(),
        None => settings.profile_for(role),
    };
    clamp_profile_to_camera(&mut profile, info);
    apply_profile_to_role(settings, role, &profile);
    settings.camera_profiles.insert(key, profile);
}

/// Bring a profile inside what `info` supports.
///
/// Two kinds of clamp. Capability fields (cooler, sensor mode, dew heater) are
/// zeroed when the hardware has none, so a previous camera's settings can't bleed
/// into a profile that could never use them. Range fields (exposure, gain, binning)
/// are the three `CaptureConfig::validate` rejects outright, stopping capture — a
/// zero there means "never configured" and takes the default rather than the
/// camera's minimum: a 32 µs sub is valid but useless.
pub(crate) fn clamp_profile_to_camera(profile: &mut CameraCaptureProfile, info: &CameraInfo) {
    let defaults = CameraCaptureProfile::default();

    if !info.has_cooler {
        profile.cooler_enabled = false;
        profile.target_temp_c = None;
    }
    if info.sensor_modes.is_empty() {
        profile.sensor_mode_override = None;
    }
    if !info.has_dew_heater {
        profile.dew_heater_enabled = false;
        profile.dew_heater_power = defaults.dew_heater_power;
    }
    profile.dew_heater_power = profile.dew_heater_power.clamp(0, 100);

    if profile.exposure_us == 0 {
        profile.exposure_us = defaults.exposure_us;
    }
    if info.min_exposure_us <= info.max_exposure_us {
        profile.exposure_us = profile
            .exposure_us
            .clamp(info.min_exposure_us, info.max_exposure_us);
    }
    if info.min_gain <= info.max_gain {
        profile.gain = profile.gain.clamp(info.min_gain, info.max_gain);
    }
    if !info.supported_bins.contains(&profile.bin) {
        profile.bin = info
            .supported_bins
            .first()
            .copied()
            .unwrap_or(defaults.bin);
    }
}

/// Write a profile into whichever live fields belong to `role`.
fn apply_profile_to_role(
    settings: &mut CaptureSettings,
    role: CameraRole,
    profile: &CameraCaptureProfile,
) {
    match role {
        CameraRole::Main => profile.apply_to(settings),
        CameraRole::Guide => settings.guide_camera = profile.clone(),
    }
}

/// Push `role`'s current `cooler_enabled`/`target_temp_c` settings to that slot's camera
/// handle. Called by the settings API when those fields change while connected but
/// not capturing — otherwise slider moves are only persisted, never reaching the
/// TEC. Skips if: no camera in the role; camera has no cooling; the handle is held by
/// the capture thread (its per-frame `apply_cooler_config` will pick up the change
/// next frame); or the camera is `WarmingUp` (the monitor intentionally disabled the
/// cooler, waiting for the sensor to thaw).
pub async fn apply_cooler_settings(state: &Arc<AppState>, role: CameraRole) {
    let Some(connected) = state.camera_in_role(role) else {
        return;
    };
    let camera_name = connected.info.name;
    if !connected.info.has_cooler {
        return;
    }

    let phase = state.camera_phase(role);
    if matches!(
        phase,
        CameraPhase::Capturing
            | CameraPhase::Guiding
            | CameraPhase::WarmingUp
            | CameraPhase::Recovering
    ) {
        debug!(
            camera_name = %camera_name,
            ?phase,
            "Skipping live cooler apply — phase owns the cooler"
        );
        return;
    }

    let (enabled, target, fast) = {
        let profile = state.settings.snapshot().profile_for(role);
        (
            profile.cooler_enabled,
            profile.target_temp_c,
            profile.cooler_fast_mode,
        )
    };

    // Only the cooler enable/disable switch is pushed to hardware here; the
    // target temperature is handed to the monitor so the setpoint ramps at
    // RAMP_RATE_C_PER_MIN instead of snapping to the final value.
    let applied = match with_camera(state, role, |cam| cam.set_cooler(enabled)).await {
        Some(Ok(())) => true,
        Some(Err(e)) => {
            warn!(error = %e, "Failed to apply live cooler switch");
            false
        }
        None => {
            warn!(
                camera_name = %camera_name,
                "Cooler change not applied — no camera handle became available"
            );
            false
        }
    };
    if !applied {
        return;
    }

    // If the user changed the target while we were already settled (Idle),
    // drop back to Precooling so the monitor re-drives the settle logic — unless a
    // capture took the camera while the switch waited for its handle.
    if enabled && target.is_some() && phase == CameraPhase::Idle {
        let (from, to) = (CameraPhase::Idle, CameraPhase::Precooling);
        state.transition_camera_phase(role, &camera_name, from, to);
    }

    // Hand the new target to the monitor: it will re-seed the cooldown ramp
    // from the current sensor temperature (or snap to target when in fast
    // mode) and advance toward `target`.
    send_monitor_cmd(
        state,
        role,
        MonitorCmd::UpdateCoolerTarget {
            enabled,
            target,
            fast,
        },
    );

    info!(
        camera_name = %camera_name,
        enabled,
        target_temp_c = ?target,
        fast,
        "Live cooler settings applied"
    );
}

/// Push `role`'s current `dew_heater_enabled` / `dew_heater_power` settings to that
/// slot's camera handle.
pub async fn apply_dew_heater_settings(state: &Arc<AppState>, role: CameraRole) {
    let Some(connected) = state.camera_in_role(role) else {
        return;
    };
    let camera_name = connected.info.name;
    if !connected.info.has_dew_heater {
        return;
    }

    let phase = state.camera_phase(role);
    let (enabled, power) = {
        let profile = state.settings.snapshot().profile_for(role);
        (profile.dew_heater_enabled, profile.dew_heater_power)
    };

    // A capture session ends, and its next `initialize_capture_session` reapplies
    // everything; a guide loop does not, so dropping the change there means the switch
    // never reaches the device. Hand it to whoever holds the handle instead.
    if phase == CameraPhase::Guiding {
        state
            .slot(role)
            .queue_op(crate::state::CameraOp::SetDewHeater { enabled, power });
        debug!(
            camera_name = %camera_name,
            enabled, power, "Dew heater change queued for the guide loop"
        );
        return;
    }
    // A recovering camera has no handle; the reopen applies the stored profile.
    if matches!(phase, CameraPhase::Capturing | CameraPhase::Recovering) {
        debug!(
            camera_name = %camera_name,
            ?phase,
            "Skipping live dew heater apply — no handle to apply it to"
        );
        return;
    }

    match with_camera(state, role, |cam| cam.set_dew_heater(enabled, power)).await {
        Some(Ok(())) => info!(
            camera_name = %camera_name,
            enabled,
            power,
            "Live dew heater settings applied"
        ),
        Some(Err(e)) => warn!(error = %e, "Failed to apply live dew heater settings"),
        None => warn!(
            camera_name = %camera_name,
            "Dew heater change not applied — no camera handle became available"
        ),
    }
}

/// Send a command to one slot's monitor, swallowing failures if it has exited.
pub(crate) fn send_monitor_cmd(state: &Arc<AppState>, role: CameraRole, cmd: MonitorCmd) {
    state.slot(role).send_monitor_cmd(cmd);
}
