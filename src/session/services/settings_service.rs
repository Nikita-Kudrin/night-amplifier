//! Applying a settings update: what it may not do, what it changes, and what has to
//! follow from the change.
//!
//! [`check`] and [`apply`] are pure, so the rules are tested without a server; they run
//! inside one `SettingsStore::update`, and the reactions after it. Everything a reaction
//! needs from another lock is read *before* the update, which takes no lock of its own.

use std::sync::Arc;

use tracing::info;

use super::PushToService;
use crate::plugins::Plugins;
use crate::session::camera::lifecycle::{
    self, apply_cooler_settings, apply_dew_heater_settings, camera_profile_key,
};
use crate::session::capture::storage;
use super::UpdateSettingsRequest;
use crate::session::error::{ApiError, ApiResult};
use crate::session::events::ServerEvent;
use crate::session::state::{
    focus_mode, AppState, CameraRole, CaptureMode, CaptureSettings, CaptureState, Resolution,
};

/// Which Push-To-relevant inputs a settings update actually changes.
///
/// Deliberately about *change*, not presence. The frontend posts the whole telescope
/// block on every debounced save, so testing `is_some()` fired on saves that changed
/// nothing — and since these flags abort and restart a plate solve, an ordinary
/// settings write could kill a solve the user was waiting on. The field log for
/// 2026-08-22 has settings updates arriving ten to a minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpticsChange {
    /// Telescope optics (focal length, pixel size, sensor dimensions, barlow).
    pub telescope: bool,
    /// Framing: binning or sensor mode. Both change the effective field of view
    /// without touching the telescope block.
    pub framing: bool,
}

impl OpticsChange {
    /// Whether anything that invalidates a plate solve changed.
    pub fn any(&self) -> bool {
        self.telescope || self.framing
    }
}

/// Compare a settings request against the settings currently in force.
///
/// `role` is the camera the request's hardware fields are for: binning and sensor mode
/// live in that camera's own profile, so comparing them against the flat fields would
/// report a guide-camera change as a change to the imaging camera's framing.
pub fn optics_change(
    request: &UpdateSettingsRequest,
    current: &CaptureSettings,
    role: CameraRole,
) -> OpticsChange {
    let profile = current.profile_for(role);
    let bin_changed = request.bin.is_some_and(|b| b != profile.bin);
    let sensor_mode_changed = request
        .sensor_mode_override
        .as_ref()
        .is_some_and(|m| Some(m) != profile.sensor_mode_override.as_ref());

    // A per-camera optics profile is what the solver actually reads, so a request that
    // only rewrites the map still moves the FOV hint. The flat block alone would miss it.
    let profiles_changed = request
        .camera_telescope_profiles
        .as_ref()
        .is_some_and(|p| *p != current.camera_telescope_profiles);

    OpticsChange {
        telescope: profiles_changed
            || request
                .telescope
                .as_ref()
                .is_some_and(|t| *t != current.telescope),
        framing: bin_changed || sensor_mode_changed,
    }
}

/// The Pro features a request may switch on, read once from the licence and plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProFeatures {
    pub saturation_boost: bool,
    pub multi_point_planetary: bool,
}

impl ProFeatures {
    fn of(plugins: &Plugins) -> Self {
        Self {
            saturation_boost: plugins.saturation().is_some(),
            multi_point_planetary: plugins.planetary().is_some(),
        }
    }
}

/// Where an update lands: the camera its hardware fields are for, and what that
/// camera's capture is doing.
#[derive(Debug, Clone)]
pub(crate) struct UpdateTarget {
    pub role: CameraRole,
    /// The `"{provider}/{model}"` profile of the camera in `role`; `None` when that
    /// position is empty, so no phantom profile is created for it.
    pub profile_key: Option<String>,
    pub capture_state: CaptureState,
}

/// What an applied update changed: the input to every reaction that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SettingsDelta {
    pub optics: OpticsChange,
    pub cooler: bool,
    pub dew_heater: bool,
    /// Exposure, gain, offset or binning: the exposure in flight no longer matches them.
    pub exposure: bool,
    /// The update switched a running live view to stacking, which ends Focus/Finder mode.
    pub left_focus_mode: bool,
}

/// Refuses a request before any of it is applied, so a refused request changes nothing.
pub(crate) fn check(
    request: &UpdateSettingsRequest,
    current: &CaptureSettings,
    capture_state: CaptureState,
    pro: ProFeatures,
) -> ApiResult<()> {
    if request.stacking_type.is_some() && capture_state != CaptureState::Idle {
        return Err(ApiError::StackingTypeChangeNotAllowed);
    }

    // Entering Focus/Finder mode drops the pre-demosaic banding correction, and the frame
    // it produces is the frame the accumulator integrates — so switching it on mid-stack
    // mixes banding into a master that can never be cleaned again. Judged on the mode this
    // request *leaves* the capture in: `focus_mode` and `stacking` in one request slipped
    // past a check of the current mode. Leaving the mode is always allowed.
    if request.focus_mode == Some(true) {
        let resulting_mode = CaptureMode::from_flags(
            request.stacking.unwrap_or(current.stacking),
            request.wanderer_mode.unwrap_or(current.wanderer_mode),
        );
        let stacking_type = request.stacking_type.unwrap_or(current.stacking_type);
        if focus_mode::conflicts_with_capture(resulting_mode, stacking_type, capture_state) {
            return Err(ApiError::FocusModeWhileStacking);
        }
    }

    if request.saturation_boost == Some(true) && !pro.saturation_boost {
        return Err(ApiError::ProFeatureRequired("Shadow Saturation Boost"));
    }
    if request.planetary_multi_point_alignment == Some(true) && !pro.multi_point_planetary {
        return Err(ApiError::ProFeatureRequired("Multi-Point Planetary Alignment"));
    }
    Ok(())
}

/// Copies each field the request carries; ranges are `CaptureSettings::sanitized`'s job.
macro_rules! copy_present {
    ($request:ident => $target:expr; $($field:ident),+ $(,)?) => {
        $(if let Some(value) = $request.$field {
            $target.$field = value;
        })+
    };
}

/// [`copy_present`] for an optional setting the request can set but not clear.
macro_rules! set_present {
    ($request:ident => $target:expr; $($field:ident),+ $(,)?) => {
        $(if let Some(value) = $request.$field {
            $target.$field = Some(value);
        })+
    };
}

/// Applies a request [`check`] accepted onto `settings`. `plugins` decide what leaving
/// Focus/Finder mode may restore.
pub(crate) fn apply(
    request: UpdateSettingsRequest,
    settings: &mut CaptureSettings,
    target: &UpdateTarget,
    plugins: &Plugins,
) -> SettingsDelta {
    let delta = SettingsDelta {
        optics: optics_change(&request, settings, target.role),
        cooler: request.cooler_enabled.is_some()
            || request.target_temp_c.is_some()
            || request.cooler_fast_mode.is_some(),
        dew_heater: request.dew_heater_enabled.is_some() || request.dew_heater_power.is_some(),
        exposure: request.exposure_us.is_some()
            || request.gain.is_some()
            || request.offset.is_some()
            || request.bin.is_some(),
        left_focus_mode: false,
    };
    let focus_request = request.focus_mode;

    if let Some(resolution) = request.streaming_resolution {
        log_stream_resolution_change(
            "Streaming resolution changed",
            settings.streaming_resolution,
            resolution,
        );
    }
    if let Some(eyepiece) = &request.eyepiece {
        log_stream_resolution_change(
            "Eyepiece streaming resolution changed",
            settings.eyepiece.stream_resolution.resolution(),
            eyepiece.stream_resolution.resolution(),
        );
    }

    // The hardware fields belong to one camera, not to the session: with a guide camera
    // connected there are two live sets, and `role` says which one this request edits.
    // Applied as a unit so a request can never leave a camera half-updated.
    let mut profile = settings.profile_for(target.role);
    copy_present!(request => profile;
        exposure_us, gain, offset, bin, cooler_enabled, cooler_fast_mode,
        dew_heater_enabled, dew_heater_power,
    );
    set_present!(request => profile; target_temp_c, sensor_mode_override);

    copy_present!(request => settings;
        auto_stretch, stacking, rejection_sigma, rejection_method, background_subtraction,
        background_extraction_algorithm, raw_frame_saving, save_stacked_image, stacking_type,
        weighting_preset, stretch_aggressiveness, auto_stretch_intensity, saturation_boost,
        saturation_boost_strength, use_simulated_camera, simulated_preload_images,
        show_focus_image, force_focus_image_now, planetary_auto_tracking,
        planetary_multi_point_alignment, auto_reconnect, auto_resume_capture, wanderer_mode,
        denoise, preview_resolution, streaming_resolution, sensor_correction, eyepiece,
        telescope, camera_telescope_profiles, camera_profiles, eula_accepted,
        indi_server_host, indi_server_port,
    );
    set_present!(request => settings; comet_roi, planetary_roi, last_camera_name);

    let profile = profile.sanitized();
    match target.role {
        CameraRole::Main => profile.apply_to(settings),
        CameraRole::Guide => settings.guide_camera = profile.clone(),
    }
    // Remembered against the camera itself, so reconnecting restores what the user set.
    if let Some(key) = &target.profile_key {
        settings.camera_profiles.insert(key.clone(), profile);
    }

    // Last, so the mode wins over any managed setting in the same request: it is a
    // statement about the whole group. With no toggle in the request, `reconcile` absorbs
    // anything a stale client wrote behind its back rather than letting the next toggle
    // silently revert it.
    match focus_request {
        Some(on) => focus_mode::set(settings, on, plugins),
        None => focus_mode::reconcile(settings),
    }
    // A running live view switched to stacking: no start path sees that, and `check`
    // only guards *entering*. Inside the write guard, so the stacking task never reads
    // `stacking` on with the corrections still held off.
    let left_focus_mode = focus_mode::leave_if_conflicting(settings, target.capture_state, plugins);

    *settings = std::mem::take(settings).sanitized();
    SettingsDelta {
        left_focus_mode,
        ..delta
    }
}

/// Applies to every connected client from the next rendered frame on, which is what makes
/// a size change in the field worth a line of its own.
fn log_stream_resolution_change(message: &str, from: Resolution, to: Resolution) {
    if from != to {
        info!(from = from.label(), to = to.label(), "{message}");
    }
}

/// Service for changing settings
pub struct SettingsService;

impl SettingsService {
    /// Checks, applies and persists `request`, then runs what the change calls for.
    /// Returns the settings it left in force.
    pub async fn update(
        state: &Arc<AppState>,
        request: UpdateSettingsRequest,
    ) -> ApiResult<CaptureSettings> {
        let role = request.camera_role.unwrap_or(CameraRole::Main);
        let target = UpdateTarget {
            role,
            profile_key: state
                .camera_in_role(role)
                .map(|camera| camera_profile_key(&camera.provider, &camera.info.name, role)),
            capture_state: state.capture_state(),
        };

        let (applied, delta) = state.settings.update(|settings| {
            check(&request, settings, target.capture_state, ProFeatures::of(&state.plugins))?;
            let delta = apply(request, settings, &target, &state.plugins);
            ApiResult::Ok((settings.clone(), delta))
        })?;

        Self::react(state, &target, delta, &applied).await;
        Ok(applied)
    }

    async fn react(
        state: &Arc<AppState>,
        target: &UpdateTarget,
        delta: SettingsDelta,
        applied: &CaptureSettings,
    ) {
        if delta.left_focus_mode {
            info!("Leaving Focus/Finder mode: the capture switched to stacking");
            let _ = state.events.send(ServerEvent::FocusModeLeft);
        }

        // Cut the exposure in flight short so the change lands now rather than at the end
        // of the current sub. Scoped to `role`: the guide camera free-runs whether or not
        // a capture is going, and cancelling every slot would discard a running imaging
        // sub because somebody nudged the guide camera's gain.
        let capturing = target.capture_state == CaptureState::Capturing;
        let camera_is_exposing = match target.role {
            CameraRole::Main => capturing,
            CameraRole::Guide => state.guide_loop_running(),
        };
        if delta.exposure && camera_is_exposing {
            info!(
                role = target.role.label(),
                "Exposure-impacting settings updated, cancelling the current exposure to apply them"
            );
            state.cancel_active_exposure(target.role).await;
        }

        storage::sync_disk_session(state, applied, capturing).await;
        // A resume restores the plan's settings. An edit made while the capture runs, or
        // while it is paused for a reconnect, is what the observer wants it to resume
        // with — the snapshot from capture start silently undid it.
        state.resume.edit_plan(|plan| plan.settings = applied.clone());

        let _ = state.events.send(ServerEvent::SettingsUpdated);

        // Without these, slider moves are persisted but never reach the TEC or the heater
        // while the camera is idle: the per-frame apply inside `capture()` only runs while
        // capturing.
        if delta.cooler {
            apply_cooler_settings(state, target.role).await;
        }
        if delta.dew_heater {
            apply_dew_heater_settings(state, target.role).await;
        }

        // Resolved through `sync_solver_rig` rather than read flat: with a guide camera
        // connected the solve runs on the *guide* scope, and its focal length is what the
        // ASTAP hint has to describe.
        if delta.optics.telescope {
            lifecycle::sync_solver_rig(state).await;
        }
        // Anything that changes the field of view invalidates a solve in flight — it was
        // planned against the old optics — and the cached pointing with it. Restart rather
        // than cancel: a bare cancel leaves the star field unchanged, so the movement
        // detector reports `Idle` forever after and nothing re-solves.
        if delta.optics.any() {
            let _ = PushToService::restart_solve(state, "Equipment settings changed").await;
        }

        state.save_settings();
    }
}

#[cfg(test)]
#[path = "settings_service_tests.rs"]
mod tests;
