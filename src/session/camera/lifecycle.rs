//! Public API for camera connect/disconnect/handoff orchestration.
//!
//! This layer owns the camera handle in each role's `CameraSlot` and coordinates with
//! that slot's monitor thread. `CameraService`, the capture loop and
//! the guide loop delegate to these functions rather than opening/closing handles
//! directly. Every entry point names a [`CameraRole`]: with an imaging camera and a
//! guide camera connected at once, "the camera" is no longer an answer.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::{install, WARMUP_TIMEOUT};
use crate::camera::identity::{self, CameraIdError};
use crate::camera::{Camera, CameraInfo};
use crate::session::error::{ApiError, ApiResult};
use crate::session::events::ServerEvent;
use crate::session::state::{
    AppState, CameraCaptureProfile, CameraPhase, CameraRole, CaptureSettings, CaptureState,
    ConnectedCameraInfo, MonitorCmd,
};

/// How long a caller waits for the monitor to hand the camera handle back
/// before giving up. Slightly over the monitor's own `FFI_CALL_TIMEOUT`, so a
/// call that is merely slow is waited out and only a genuine stall gives up.
const HANDLE_WAIT_TIMEOUT: Duration = Duration::from_millis(3_500);

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
pub(super) const CAPTURE_STOP_WAIT: Duration = Duration::from_secs(3);

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
    crate::session::camera::health::forget_restart_history(state, role, &opened.camera.info().name);
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
    crate::session::services::PushToService::set_rig(state, camera_name, telescope).await;
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
        crate::session::capture::guide_task::stop(state).await;
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
    if crate::session::camera::health::has_recent_fault(state, role, camera_name) {
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
    crate::session::services::CaptureService::stop_capture(state).await;
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
async fn begin_warmup(state: &Arc<AppState>, role: CameraRole, camera_name: &str, fast: bool) {
    track_warmup(state, role, camera_name).await;
    send_monitor_cmd(state, role, MonitorCmd::StartWarmup { fast });
}

/// Put the camera into `WarmingUp` and start the watchdog that ends the warm-up by
/// [`WARMUP_DEADLINE`] whatever the monitor manages. Nothing drives it yet: callers whose
/// monitor can command the camera now use [`begin_warmup`].
async fn track_warmup(state: &Arc<AppState>, role: CameraRole, camera_name: &str) {
    let warmup = state.slot(role).begin_warmup(Instant::now() + WARMUP_DEADLINE);
    state.set_camera_phase(role, camera_name, CameraPhase::WarmingUp);

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
        crate::session::capture::guide_task::stop(state).await;
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
        && super::recovery::suspend(state, role, camera_name).await
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
        crate::session::capture::guide_task::stop(state).await;
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
    super::reconnect::spawn(state, removed);
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
            .queue_op(crate::session::state::CameraOp::SetDewHeater { enabled, power });
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
