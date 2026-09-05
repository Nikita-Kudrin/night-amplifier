//! Public API for camera connect/disconnect/handoff orchestration.
//!
//! This layer owns the camera handle in each `AppState.camera_slots` entry and
//! coordinates with that slot's monitor thread. `CameraService`, the capture loop and
//! the guide loop delegate to these functions rather than opening/closing handles
//! directly. Every entry point names a [`CameraRole`]: with an imaging camera and a
//! guide camera connected at once, "the camera" is no longer an answer.

use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::monitor;
use crate::camera::{Camera, CameraInfo, CameraRegistry};
use crate::server::error::{ApiError, ApiResult};
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CameraCaptureProfile, CameraPhase, CameraRole, CaptureSettings, CaptureState,
    ConnectedCameraInfo, MonitorCmd,
};
use crate::telemetry::metrics as telemetry_metrics;

/// How long a caller waits for the monitor to hand the camera handle back
/// before giving up. Slightly over the monitor's own `FFI_CALL_TIMEOUT`, so a
/// call that is merely slow is waited out and only a genuine stall gives up.
const HANDLE_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(3_500);

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
    /// in principle: hand it to the reconnect supervisor.
    DeviceFault,
}

impl DisconnectCause {
    fn should_attempt_reconnect(self) -> bool {
        matches!(self, DisconnectCause::DeviceFault)
    }
}

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
    // Serialize connects. The idempotency check below reads `cameras`, which
    // `finalize_disconnect` clears before the reconnect supervisor starts, so
    // without this an automatic reconnect and a user clicking Connect would
    // both pass it, both open the device, and the second would displace — and
    // therefore close — the first.
    let _connect_guard = state.camera_connect_lock.lock().await;

    // Already connected? Return the existing info — matches the prior
    // idempotent connect behavior. Connecting an already-connected camera into the
    // *other* role is a different request and is refused: one device cannot be both
    // the imaging camera and the guide camera.
    {
        let cameras = state.cameras.read().await;
        if let Some(info) = cameras.get(camera_id) {
            if info.role == role {
                return Ok(info.clone());
            }
            return Err(ApiError::CameraRoleMismatch {
                camera: info.info.name.clone(),
                held: info.role.label(),
                requested: role.label(),
            });
        }
    }

    vacate_role(state, role).await?;

    let (provider_name, index) = parse_camera_id(camera_id)?;
    let use_simulated = state.settings.read().await.use_simulated_camera;
    let provider_name = provider_name.to_string();

    // Open the camera on a blocking task so the FFI call doesn't occupy a
    // tokio worker. Returned: the handle plus the canonical provider name
    // (case-corrected) plus the CameraInfo.
    let open_result = tokio::task::spawn_blocking(
        move || -> Result<(Box<dyn Camera>, String), crate::camera::CameraError> {
            let mut registry = CameraRegistry::new();
            let _ = registry.register(crate::camera::PlayerOneProvider::new());
            let _ = registry.register(crate::camera::ZwoProvider::new());
            if use_simulated {
                let _ = registry.register(crate::camera::SimulatedProvider::new());
            }
            let provider_registry_name = registry
                .providers()
                .into_iter()
                .find(|name| name.to_lowercase() == provider_name.to_lowercase())
                .map(|s| s.to_string())
                .unwrap_or_else(|| provider_name.clone());

            let camera = registry.open_camera(&provider_registry_name, index)?;
            Ok((camera, provider_registry_name))
        },
    )
    .await;

    let (mut camera, provider_registry_name) = match open_result {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            error!(camera_id = %camera_id, error = %e, "Failed to open camera");
            return Err(ApiError::CameraOpenFailed(e.to_string()));
        }
        Err(e) => {
            error!(camera_id = %camera_id, error = %e, "Blocking task failed");
            return Err(ApiError::Internal(e.to_string()));
        }
    };

    let info = camera.info().clone();
    let camera_name = info.name.clone();

    // Prove the handle works before reporting success.
    //
    // `open()` returning is not evidence: the field failure this guards against
    // opened cleanly, seeded the cooler without complaint, and only started
    // answering `POA_ERROR_NOT_OPENED` a minute later, once the previous
    // abandoned handle's destructor had closed the device underneath it. A
    // status read touches the same config path a capture will, so a handle that
    // is already dead fails here instead of at the first frame.
    if let Err(e) = camera.status() {
        if e.is_sdk_disconnected() {
            error!(
                camera_id = %camera_id,
                camera_name = %camera_name,
                error = %e,
                "Camera opened but is not responding; discarding the handle"
            );
            let _ = camera.close();
            return Err(ApiError::CameraOpenFailed(format!(
                "camera opened but did not respond: {}",
                e
            )));
        }
        debug!(camera_id = %camera_id, error = %e, "Probe read returned a non-fatal error");
    }

    info!(
        camera_id = %camera_id,
        camera_name = %camera_name,
        provider = %provider_registry_name,
        "Camera opened and verified"
    );
    debug!(
        camera_id = %camera_id,
        specifications = ?info,
        "Camera specifications"
    );

    // Swap the per-camera profile into this role's live fields before deciding precool
    // — otherwise a cooled-camera's `cooler_enabled` would leak into the
    // next-connected uncooled camera.
    let profile_key = camera_profile_key(&provider_registry_name, &camera_name, role);
    let (cooler_enabled, target_temp_c, cooler_fast_mode, dew_heater_enabled, dew_heater_power) = {
        let mut settings = state.settings.write().await;
        apply_camera_profile_on_connect(&mut settings, profile_key.clone(), role, &info);
        let profile = settings.profile_for(role);
        (
            profile.cooler_enabled,
            profile.target_temp_c,
            profile.cooler_fast_mode,
            profile.dew_heater_enabled,
            profile.dew_heater_power,
        )
    };

    // Decide initial phase: if the camera supports cooling and the user has
    // a target in settings, kick off precool right now. Otherwise Idle.
    //
    // In normal (ramped) mode we hold the TEC setpoint at the current sensor
    // temperature and let the monitor ramp it toward the user's target at
    // `RAMP_RATE_C_PER_MIN`. In fast mode we push the final target directly,
    // restoring the old "snap to setpoint" behavior.
    let (initial_phase, cooler_applied) = if info.has_cooler && cooler_enabled {
        if let Some(final_target) = target_temp_c {
            let initial_setpoint = if cooler_fast_mode {
                final_target
            } else {
                match camera.status() {
                    Ok(s) => s.temperature_c,
                    Err(_) => final_target,
                }
            };
            let seed = camera
                .set_target_temperature(initial_setpoint)
                .and_then(|()| camera.set_cooler(true));
            match seed {
                Ok(()) => (CameraPhase::Precooling, true),
                Err(e) => {
                    warn!(error = %e, "Failed to enable cooler on connect — falling back to Idle");
                    (CameraPhase::Idle, false)
                }
            }
        } else {
            (CameraPhase::Idle, false)
        }
    } else {
        (CameraPhase::Idle, false)
    };

    // Apply initial dew heater state if supported.
    if info.has_dew_heater {
        let _ = camera.set_dew_heater(dew_heater_enabled, dew_heater_power);
    }

    let connected_info = ConnectedCameraInfo {
        id: camera_id.to_string(),
        provider: provider_registry_name,
        index,
        role,
        info,
    };

    // Store handle, metadata, selected, phase.
    {
        let mut cameras = state.cameras.write().await;
        cameras.insert(camera_id.to_string(), connected_info.clone());
        telemetry_metrics::record_cameras_count(cameras.len() as u64);
    }
    if role == CameraRole::Main {
        // The selection is what the settings panel is editing, and a freshly connected
        // imaging camera is what the user is about to configure. A guide camera does not
        // steal that focus.
        *state.selected_camera.write().await = Some(camera_id.to_string());
    }
    {
        let slot = state.slot(role);
        let mut guard = slot.handle.lock().expect("camera handle mutex poisoned");
        debug_assert!(
            guard.is_none(),
            "connect installed a handle over an occupied {} slot — vacate_role should have cleared it",
            role.label()
        );
        if let Some(mut displaced) = guard.replace(camera) {
            warn!(camera_name = %camera_name, role = role.label(), "Closing a camera handle displaced by this connect");
            let _ = displaced.close();
        }
    }
    state.slot(role).notify_handle_returned();

    state.set_camera_phase(&camera_name, initial_phase).await;

    // Spawn the monitor thread. It will drive Precooling→Idle transition
    // and emit `CameraStatusUpdated` every 2s for any cooled camera.
    let tx = monitor::spawn(
        Arc::clone(state),
        role,
        camera_name.clone(),
        tokio::runtime::Handle::current(),
    );
    if let Some(orphan) = state.slot(role).set_monitor_tx(Some(tx)) {
        let _ = orphan.send(MonitorCmd::Shutdown);
    }

    // If we started precooling, hand the ramp targets off to the monitor so
    // it can begin rate-limited tracking (or snap to target when in fast mode).
    if cooler_applied {
        send_monitor_cmd(
            state,
            role,
            MonitorCmd::UpdateCoolerTarget {
                enabled: true,
                target: target_temp_c,
                fast: cooler_fast_mode,
            },
        );
    }

    let _ = state
        .events
        .send(ServerEvent::camera_connected(&camera_name));

    // Name the solving camera and its optics before any frame can reach the solver, so
    // the first solve of the session is already judged against the right rig. With a
    // guide camera present that rig is the *guide* scope, which is usually a different
    // focal length — an ASTAP hint from the main scope sends it searching at the wrong
    // scale.
    sync_solver_rig(state).await;

    // Persist the (possibly new / clamped) camera profile to disk.
    state.save_settings().await;

    // The guide camera free-runs from the moment it connects: solving and its preview
    // must work while the user is still framing, before any capture has started.
    if role == CameraRole::Guide {
        crate::server::capture::guide_task::start(state, &connected_info);
    }

    debug!(
        camera_id = %camera_id,
        role = role.label(),
        phase = ?initial_phase,
        cooler_applied,
        "Camera session started"
    );

    Ok(connected_info)
}

/// Make `role` free for a new camera, or explain why it cannot be.
///
/// A camera mid-capture or mid-warmup is doing something the user asked for and that
/// cannot be interrupted safely — a warmup cut short closes a handle with the sensor
/// still cold. An idle one is just occupying the position, so it is disconnected and the
/// new camera takes its place.
pub(crate) async fn vacate_role(state: &Arc<AppState>, role: CameraRole) -> ApiResult<()> {
    let Some(incumbent) = state.camera_in_role(role).await else {
        return Ok(());
    };

    let phase = state.camera_phase(&incumbent.info.name).await;
    let capture_state = state.capture_state().await;
    let busy_capturing = role == CameraRole::Main
        && matches!(
            capture_state,
            CaptureState::Capturing | CaptureState::Starting
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
    let solve_camera = match state.camera_in_role(CameraRole::Guide).await {
        Some(guide) => Some(guide),
        None => state.camera_in_role(CameraRole::Main).await,
    };
    let camera_name = solve_camera.map(|info| info.info.name);

    let telescope = {
        let settings = state.settings.read().await;
        settings.solver_telescope(camera_name.as_deref())
    };

    // One call, not two: the camera and the optics are a single fact about the rig, and
    // the solver's remembered FOV is keyed on both. Applying them separately made the
    // solver resolve once against a pair that never existed — the new camera behind the
    // old camera's focal length — and that resolve *discards* a remembered FOV whose
    // camera disagrees, so the intermediate state could delete the outgoing rig's
    // measurement on the way past. See `PushToSolverPlugin::set_rig`.
    crate::server::services::PushToService::set_rig(camera_name, telescope).await;
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

/// Swap the per-camera profile for `key` into `role`'s live hardware fields, seeding a
/// fresh one from those fields if the camera has no profile yet.
///
/// Either way the profile is clamped to what this camera can actually do, and the
/// clamped copy is written back to the map. Clamping the *stored* path too is what
/// repairs a profile that was persisted out of range — settings files written before
/// `CameraCaptureProfile` had a real `Default` hold `exposure_us: 0, bin: 0`, which
/// `CaptureConfig::validate` rejects on every frame.
///
/// "Live fields" means the flat `CaptureSettings` fields for the main camera and
/// `CaptureSettings::guide_camera` for the guide — the two cameras are connected at once
/// and cannot share one set of values.
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
/// Two kinds of clamp, and they answer different questions. Capability fields (cooler,
/// sensor mode, dew heater) are zeroed when the hardware has none, so a previous
/// camera's settings cannot bleed into a profile that could never use them. Range
/// fields (exposure, gain, binning) are the three `CaptureConfig::validate` rejects
/// outright — an out-of-range one is not a cosmetic problem, it stops the camera
/// capturing at all. A zero there means "never configured", so it takes the default
/// rather than the camera's minimum: a 32 µs sub is a valid exposure and a useless one.
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

/// Disconnect (or begin warmup prior to disconnect) for a camera.
pub async fn disconnect(state: &Arc<AppState>, camera_id: &str) -> ApiResult<String> {
    let connected = {
        let cameras = state.cameras.read().await;
        cameras.get(camera_id).cloned()
    };
    let Some(connected) = connected else {
        warn!(camera_id = %camera_id, "Attempted to disconnect non-connected camera");
        return Err(ApiError::CameraNotConnected(camera_id.to_string()));
    };
    let role = connected.role;
    let camera_name = connected.info.name;

    // Can't disconnect the imaging camera mid-capture — user must stop capture first.
    // The guide camera has no such tie: its loop is its own and stopping it costs the
    // session nothing but plate solving.
    if role == CameraRole::Main {
        let current_capture_state = state.capture_state().await;
        if current_capture_state == CaptureState::Capturing
            || current_capture_state == CaptureState::Starting
        {
            return Err(ApiError::CameraInUse);
        }
    }

    let phase = state.camera_phase(&camera_name).await;

    // Already warming up — idempotent no-op.
    if phase == CameraPhase::WarmingUp {
        info!(camera_id = %camera_id, "Disconnect requested but already warming up");
        return Ok(camera_name);
    }

    // Stop the free-running loop before anything touches the handle, so the loop is not
    // mid-exposure when the warmup or the close arrives.
    if role == CameraRole::Guide {
        crate::server::capture::guide_task::stop(state).await;
    }

    // Decide whether to warm up: if the user had cooling enabled in settings
    // (current intent) OR the last status sample reported cooler_on, ramp
    // the TEC down before closing the handle. Relying on settings alone is
    // important because the monitor may not have polled yet on fresh connects.
    let (cooler_enabled_in_settings, fast) = {
        let settings = state.settings.read().await;
        let profile = settings.profile_for(role);
        (profile.cooler_enabled, profile.cooler_fast_mode)
    };
    let cooler_reported_on = state
        .get_camera_status(&camera_name)
        .await
        .map(|s| s.cooler_on)
        .unwrap_or(false);
    let needs_warmup = cooler_enabled_in_settings || cooler_reported_on;

    if needs_warmup {
        // Start warmup; monitor thread will close handle + emit
        // CameraDisconnected when the sensor reaches WARMUP_THRESHOLD_C.
        state
            .set_camera_phase(&camera_name, CameraPhase::WarmingUp)
            .await;
        send_monitor_cmd(state, role, MonitorCmd::StartWarmup { fast });
        info!(camera_id = %camera_id, fast, "Warmup initiated; disconnect will complete asynchronously");
        Ok(camera_name)
    } else {
        // No cooler active — close immediately.
        finalize_disconnect(state, role, &camera_name, DisconnectCause::Requested).await;
        Ok(camera_name)
    }
}

/// Take a slot's camera handle for a capture session. Cancels any in-progress
/// warmup and transitions the phase to `Capturing`.
pub async fn take_for_capture(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
) -> Result<Box<dyn Camera>, ApiError> {
    let phase = state.camera_phase(camera_name).await;

    if phase == CameraPhase::WarmingUp {
        // User started capture mid-warmup — cancel, re-enable cooler per
        // current settings before handoff. Capture's per-frame
        // `apply_cooler_config` pushes the final target, so the ramp the
        // monitor would have installed is overridden anyway.
        debug!(camera_name, "Cancelling warmup: capture requested");
        send_monitor_cmd(state, role, MonitorCmd::CancelWarmup);
        let profile = state.settings.read().await.profile_for(role);
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

    let camera = take_camera(state, role).await.ok_or_else(|| {
        ApiError::Internal(format!(
            "Camera '{}' did not become available for capture — the monitor is stuck in a camera call",
            camera_name
        ))
    })?;

    // A guide camera's loop runs for the length of the connection, not the length of a
    // session, and the gates below it need to be able to tell those apart.
    let phase = match role {
        CameraRole::Main => CameraPhase::Capturing,
        CameraRole::Guide => CameraPhase::Guiding,
    };
    state.set_camera_phase(camera_name, phase).await;

    Ok(camera)
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
            let superseded = state.camera_in_role(role).await.map(|c| c.info.name).as_deref()
                != Some(camera_name)
                || state.slot(role).holds_handle();
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

            // Decide phase: if cooling is enabled and we're not yet near target,
            // precooling; otherwise idle. We use the last cached status as a
            // cheap proxy — the monitor will correct it on the next poll.
            let profile = state.settings.read().await.profile_for(role);
            let target = profile.target_temp_c;
            let cooler_enabled = profile.cooler_enabled;
            let fast = profile.cooler_fast_mode;

            let next_phase = if let Some(t) = target {
                if cooler_enabled {
                    match state.get_camera_status(camera_name).await {
                        Some(status)
                            if (status.temperature_c - t).abs() <= super::PRECOOL_TOLERANCE_C =>
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

            state.set_camera_phase(camera_name, next_phase).await;
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
            finalize_disconnect(state, role, camera_name, DisconnectCause::DeviceFault).await;
        }
    }
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
    // Shut down the monitor thread first.
    send_monitor_cmd(state, role, MonitorCmd::Shutdown);
    state.slot(role).set_monitor_tx(None);

    // The guide loop must let go of the handle before we close it. On the requested
    // path `disconnect` already stopped it; on a device fault this is where it stops.
    if role == CameraRole::Guide {
        crate::server::capture::guide_task::stop(state).await;
        state.guide_stream.clear().await;
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

    // Drop metadata and status, clear selected.
    let removed_id = {
        let mut cameras = state.cameras.write().await;
        let id = cameras
            .iter()
            .find(|(_, v)| v.info.name == camera_name && v.role == role)
            .map(|(k, _)| k.clone());
        if let Some(ref id) = id {
            cameras.remove(id);
            telemetry_metrics::record_cameras_count(cameras.len() as u64);
        }
        id
    };
    if let Some(ref id) = removed_id {
        let mut selected = state.selected_camera.write().await;
        if selected.as_ref() == Some(id) {
            *selected = None;
        }
    }
    {
        let mut statuses = state.latest_camera_status.write().await;
        statuses.remove(camera_name);
    }

    state.slot(role).notify_handle_returned();
    // Re-point the solver: losing the guide camera hands solving back to the main one,
    // along with the main scope's optics.
    sync_solver_rig(state).await;
    state
        .set_camera_phase(camera_name, CameraPhase::Disconnected)
        .await;
    let _ = state
        .events
        .send(ServerEvent::camera_disconnected(camera_name));

    info!(camera_name, role = role.label(), "Camera disconnected");

    if !cause.should_attempt_reconnect() {
        // A deliberate disconnect ends the observation, so the folder it was filling is
        // not something a later session should rejoin.
        *state.slot(role).raw_session.write().await = None;
        return;
    }
    let Some(camera_id) = removed_id else {
        return;
    };
    super::reconnect::spawn(state, role, &camera_id, camera_name);
}

/// Push `role`'s current `cooler_enabled`/`target_temp_c` settings to that slot's camera
/// handle. Called by the settings API when those fields change while connected but
/// not capturing — otherwise slider moves are only persisted, never reaching the
/// TEC. Skips if: no camera in the role; camera has no cooling; the handle is held by
/// the capture thread (its per-frame `apply_cooler_config` will pick up the change
/// next frame); or the camera is `WarmingUp` (the monitor intentionally disabled the
/// cooler, waiting for the sensor to thaw).
pub async fn apply_cooler_settings(state: &Arc<AppState>, role: CameraRole) {
    let Some(connected) = state.camera_in_role(role).await else {
        return;
    };
    let camera_name = connected.info.name;
    if !connected.info.has_cooler {
        return;
    }

    let phase = state.camera_phase(&camera_name).await;
    if matches!(
        phase,
        CameraPhase::Capturing | CameraPhase::Guiding | CameraPhase::WarmingUp
    ) {
        debug!(
            camera_name = %camera_name,
            ?phase,
            "Skipping live cooler apply — phase owns the cooler"
        );
        return;
    }

    let (enabled, target, fast) = {
        let profile = state.settings.read().await.profile_for(role);
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
    // drop back to Precooling so the monitor re-drives the settle logic.
    if enabled && target.is_some() && phase == CameraPhase::Idle {
        state
            .set_camera_phase(&camera_name, CameraPhase::Precooling)
            .await;
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
    let Some(connected) = state.camera_in_role(role).await else {
        return;
    };
    let camera_name = connected.info.name;
    if !connected.info.has_dew_heater {
        return;
    }

    let phase = state.camera_phase(&camera_name).await;
    let (enabled, power) = {
        let profile = state.settings.read().await.profile_for(role);
        (profile.dew_heater_enabled, profile.dew_heater_power)
    };

    // A capture session ends, and its next `initialize_capture_session` reapplies
    // everything; a guide loop does not, so dropping the change there means the switch
    // never reaches the device. Hand it to whoever holds the handle instead.
    if phase == CameraPhase::Guiding {
        state
            .slot(role)
            .queue_op(crate::server::state::CameraOp::SetDewHeater { enabled, power });
        debug!(
            camera_name = %camera_name,
            enabled, power, "Dew heater change queued for the guide loop"
        );
        return;
    }
    if phase == CameraPhase::Capturing {
        debug!(
            camera_name = %camera_name,
            "Skipping live dew heater apply — capture owns the handle"
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

/// Parse camera ID into provider name and index (e.g. "playerone_0" → ("playerone", 0)).
pub(super) fn parse_camera_id(camera_id: &str) -> ApiResult<(&str, usize)> {
    let parts: Vec<&str> = camera_id.splitn(2, '_').collect();
    if parts.len() != 2 {
        return Err(ApiError::InvalidCameraIdFormat);
    }
    let index: usize = parts[1].parse().map_err(|_| ApiError::InvalidCameraIndex)?;
    Ok((parts[0], index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_camera_id_valid() {
        let (provider, index) = parse_camera_id("playerone_0").unwrap();
        assert_eq!(provider, "playerone");
        assert_eq!(index, 0);
    }

    #[test]
    fn parse_camera_id_invalid_format() {
        assert!(matches!(
            parse_camera_id("invalidformat"),
            Err(ApiError::InvalidCameraIdFormat)
        ));
    }

    #[test]
    fn parse_camera_id_invalid_index() {
        assert!(matches!(
            parse_camera_id("provider_notanumber"),
            Err(ApiError::InvalidCameraIndex)
        ));
    }
}
