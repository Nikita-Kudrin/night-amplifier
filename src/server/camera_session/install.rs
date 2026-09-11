//! Opening a camera and installing it into a role, for `lifecycle::connect` and the
//! reconnect supervisor's `recovery::reopen_for_recovery`.
//!
//! Both callers hold the connect lock from the first vendor call to the last, so every
//! vendor stage — listing, opening and probing, seeding the cooler and dew heater — runs
//! through the slot's `pending_opens` under `OPEN_TIMEOUT`. Seeding used to run inline
//! on the runtime: a camera that answered the probe and then hung held every Connect,
//! and the recovery budget never ran out.

use std::sync::Arc;
use tracing::{debug, error, info, warn};

use super::lifecycle::{
    apply_camera_profile_on_connect, camera_profile_key, send_monitor_cmd, sync_solver_rig,
};
use super::monitor;
use super::reconnect::OPEN_TIMEOUT;
use crate::camera::identity;
use crate::camera::{Camera, CameraInfo, CameraLocator, DeviceIdentity, OpenedCamera};
use crate::server::error::{ApiError, ApiResult};
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, BoundedCallError, CameraPhase, CameraRole, ConnectedCameraInfo, MonitorCmd,
};
use crate::telemetry::metrics as telemetry_metrics;

/// A vendor stage that did not come back, as the error the caller reports.
pub(super) fn stage_failed(stage: &str, error: BoundedCallError) -> ApiError {
    match error {
        BoundedCallError::TimedOut => {
            ApiError::CameraOpenFailed(format!("{stage} did not return within {OPEN_TIMEOUT:?}"))
        }
        BoundedCallError::Panicked => ApiError::Internal(format!("{stage} panicked")),
    }
}

/// Where `locator` is listed now, and what is listed there. Opens nothing.
pub(super) async fn locate(
    state: &Arc<AppState>,
    role: CameraRole,
    provider: &str,
    locator: &CameraLocator,
    use_simulated: bool,
) -> ApiResult<(usize, DeviceIdentity)> {
    let catalog = Arc::clone(&state.device_catalog);
    let listing_provider = provider.to_string();
    let listed = state
        .slot(role)
        .pending_opens
        .run_bounded(OPEN_TIMEOUT, move || catalog.identities(&listing_provider, use_simulated))
        .await
        .map_err(|e| stage_failed("listing cameras", e))?
        .map_err(|e| ApiError::CameraOpenFailed(e.to_string()))?;
    let Some(index) = identity::resolve_index(&listed, locator) else {
        return Err(ApiError::CameraOpenFailed(match locator {
            CameraLocator::Serial(serial) => {
                format!("no {provider} camera with serial {serial} is connected")
            }
            CameraLocator::Index(index) => {
                format!("no {provider} camera is listed at position {index}")
            }
        }));
    };
    Ok((index, listed[index].clone()))
}

/// Refuse the device the other role's camera is using.
///
/// Before the open, never after: vendor closes go by device id, so opening that device
/// and closing the result again would already have closed the other role's camera. A
/// recovered camera keeps an index id its device no longer sits at, so the id says
/// nothing here — the SDK's serial or device id does, or failing both, where the other
/// camera was last opened.
pub(super) async fn refuse_device_of_other_role(
    state: &Arc<AppState>,
    role: CameraRole,
    provider: &str,
    index: usize,
    listed: &DeviceIdentity,
) -> ApiResult<()> {
    let Some(other) = state.camera_in_role(role.other()).await else {
        return Ok(());
    };
    if !other.provider.eq_ignore_ascii_case(provider) {
        return Ok(());
    }
    let held = DeviceIdentity::of(&other.info).with_device_id(other.info.id);
    let same = listed
        .is_same_device(&held)
        .unwrap_or(other.index == index && other.info.name == listed.name);
    if !same {
        return Ok(());
    }
    Err(ApiError::CameraRoleMismatch {
        camera: other.info.name,
        held: other.role.label(),
        requested: role.label(),
    })
}

/// Open `provider`'s device at `index` and prove it answers. A serial locator is checked
/// again on the opened handle, since the list can reorder between listing and opening.
pub(super) async fn open_verified(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_id: &str,
    provider: &str,
    index: usize,
    locator: &CameraLocator,
    use_simulated: bool,
) -> ApiResult<OpenedCamera> {
    let catalog = Arc::clone(&state.device_catalog);
    let (camera_id, provider, locator) =
        (camera_id.to_string(), provider.to_string(), locator.clone());
    state
        .slot(role)
        .pending_opens
        .run_bounded(OPEN_TIMEOUT, move || {
            let mut opened = catalog
                .open(&provider, index, use_simulated)
                .map_err(|e| ApiError::CameraOpenFailed(e.to_string()))?;
            if let CameraLocator::Serial(serial) = &locator {
                let found = opened.camera.info();
                if found.serial.as_deref() != Some(serial.as_str()) {
                    return Err(ApiError::CameraIdentityMismatch {
                        expected: serial.clone(),
                        found: found.serial.clone().unwrap_or_else(|| found.name.clone()),
                    });
                }
            }
            verify_responsive(&mut opened.camera, &camera_id)?;
            Ok(opened)
        })
        .await
        .map_err(|e| stage_failed("opening the camera", e))?
}

/// Prove the handle works before reporting success.
///
/// `open()` returning is not evidence: the field failure this guards against opened
/// cleanly, seeded the cooler without complaint, and only started answering
/// `POA_ERROR_NOT_OPENED` a minute later, once the previous abandoned handle's
/// destructor had closed the device underneath it. A status read touches the same
/// config path a capture will, so a handle that is already dead fails here instead of
/// at the first frame.
pub(super) fn verify_responsive(camera: &mut Box<dyn Camera>, camera_id: &str) -> ApiResult<()> {
    let Err(e) = camera.status() else {
        return Ok(());
    };
    if !e.is_sdk_disconnected() {
        debug!(camera_id = %camera_id, error = %e, "Probe read returned a non-fatal error");
        return Ok(());
    }
    error!(
        camera_id = %camera_id,
        camera_name = %camera.info().name,
        error = %e,
        "Camera opened but is not responding; discarding the handle"
    );
    let _ = camera.close();
    Err(ApiError::CameraOpenFailed(format!(
        "camera opened but did not respond: {}",
        e
    )))
}

/// What a connect seeds the hardware with, read from the role's profile beforehand so
/// the seeding itself needs no lock and can run off the runtime.
struct HardwareSetup {
    /// The final cooler target, when the camera has a cooler the profile switches on.
    cooler_target: Option<f64>,
    cooler_fast_mode: bool,
    /// Enabled and power, when the camera has a dew heater.
    dew_heater: Option<(bool, i32)>,
}

impl HardwareSetup {
    /// Seed the cooler and the dew heater. Returns whether the cooler took its setpoint.
    ///
    /// In normal (ramped) mode the TEC setpoint is held at the current sensor temperature
    /// and the monitor ramps it toward the target at `RAMP_RATE_C_PER_MIN`; fast mode
    /// pushes the final target directly.
    fn apply(&self, camera: &mut Box<dyn Camera>) -> bool {
        let mut cooler_applied = false;
        if let Some(final_target) = self.cooler_target {
            let initial_setpoint = if self.cooler_fast_mode {
                final_target
            } else {
                camera.status().map_or(final_target, |s| s.temperature_c)
            };
            match camera
                .set_target_temperature(initial_setpoint)
                .and_then(|()| camera.set_cooler(true))
            {
                Ok(()) => cooler_applied = true,
                Err(e) => {
                    warn!(error = %e, "Failed to enable cooler on connect — falling back to Idle")
                }
            }
        }
        if let Some((enabled, power)) = self.dew_heater {
            let _ = camera.set_dew_heater(enabled, power);
        }
        cooler_applied
    }
}

/// Swap `role`'s per-camera profile into the live settings and read back what the
/// hardware is to be seeded with.
async fn apply_profile(
    state: &Arc<AppState>,
    provider: &str,
    info: &CameraInfo,
    role: CameraRole,
) -> HardwareSetup {
    // Before deciding precool — otherwise a cooled camera's `cooler_enabled` would leak
    // into the next-connected uncooled camera.
    let profile_key = camera_profile_key(provider, &info.name, role);
    let mut settings = state.settings.write().await;
    apply_camera_profile_on_connect(&mut settings, profile_key, role, info);
    // Give the solver a rig key that came from this sensor rather than from the flat
    // block, which describes whichever camera was configured last. Two unprofiled cameras
    // otherwise share one key and one remembered FOV — see
    // `CaptureSettings::ensure_camera_telescope_profile`.
    if settings.ensure_camera_telescope_profile(&info.name, info) {
        info!(
            camera_name = %info.name,
            role = role.label(),
            pixel_size_um = info.pixel_size_y_um,
            sensor = format!("{}x{}", info.max_width, info.max_height),
            "Seeded a telescope profile from the camera's own sensor"
        );
    }
    let profile = settings.profile_for(role);
    HardwareSetup {
        cooler_target: profile
            .target_temp_c
            .filter(|_| info.has_cooler && profile.cooler_enabled),
        cooler_fast_mode: profile.cooler_fast_mode,
        dew_heater: info
            .has_dew_heater
            .then_some((profile.dew_heater_enabled, profile.dew_heater_power)),
    }
}

/// Install an opened, verified camera in `role` and start its session: profile,
/// pre-cool, monitor, solver rig, and the guide loop for a guide camera.
///
/// `replaces` is the suspended entry a recovery reopened this camera for, kept under
/// the same id so the selection and the capture resume plan still name it. An error
/// means the hardware setup did not return: nothing was registered, and the handle
/// stays with the call until it does.
pub(super) async fn install_camera(
    state: &Arc<AppState>,
    camera_id: &str,
    role: CameraRole,
    provider_registry_name: String,
    index: usize,
    camera: Box<dyn Camera>,
    replaces: Option<&ConnectedCameraInfo>,
) -> ApiResult<ConnectedCameraInfo> {
    let info = camera.info().clone();
    let camera_name = info.name.clone();

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

    let setup = apply_profile(state, &provider_registry_name, &info, role).await;
    let (cooler_target, cooler_fast_mode) = (setup.cooler_target, setup.cooler_fast_mode);
    let (camera, cooler_applied) = state
        .slot(role)
        .pending_opens
        .run_bounded(OPEN_TIMEOUT, move || {
            let mut camera = camera;
            let applied = setup.apply(&mut camera);
            (camera, applied)
        })
        .await
        .map_err(|e| stage_failed("setting up the cooler and dew heater", e))?;
    let initial_phase = if cooler_applied {
        CameraPhase::Precooling
    } else {
        CameraPhase::Idle
    };

    let connected_info = ConnectedCameraInfo {
        id: camera_id.to_string(),
        provider: provider_registry_name,
        index,
        role,
        info,
    };

    {
        let mut cameras = state.cameras.write().await;
        cameras.insert(camera_id.to_string(), connected_info.clone());
        telemetry_metrics::record_cameras_count(cameras.len() as u64);
    }
    if role == CameraRole::Main && replaces.is_none() {
        // The selection is what the settings panel is editing, and a freshly connected
        // imaging camera is what the user is about to configure. A guide camera does not
        // steal that focus, and a recovered one never lost it.
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

    state.set_camera_phase(role, &camera_name, initial_phase).await;

    // The monitor drives Precooling→Idle and emits `CameraStatusUpdated` every 2s for
    // any cooled camera.
    let tx = monitor::spawn(
        Arc::clone(state),
        role,
        camera_name.clone(),
        tokio::runtime::Handle::current(),
    );
    if let Some(orphan) = state.slot(role).set_monitor_tx(Some(tx)) {
        let _ = orphan.send(MonitorCmd::Shutdown);
    }

    // Hand the ramp targets to the monitor for rate-limited tracking (or a snap to
    // target in fast mode).
    if cooler_applied {
        send_monitor_cmd(
            state,
            role,
            MonitorCmd::UpdateCoolerTarget {
                enabled: true,
                target: cooler_target,
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
