//! Settings API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use super::super::camera_session::lifecycle::camera_profile_key;
use super::super::dto::{ApiResponse, SettingsResponse, UpdateSettingsRequest};
use super::super::events::ServerEvent;
use super::super::services::PushToService;
use super::super::state::{AppState, CameraRole, CaptureSettings, CaptureState, StackingType};

/// Returns the profile key (`"{provider}/{model}"`) for the camera in `role`, if any.
/// `None` when that position is empty — callers should treat it as "skip per-camera
/// work" rather than creating a phantom profile.
async fn camera_profile_key_for(state: &Arc<AppState>, role: CameraRole) -> Option<String> {
    state
        .camera_in_role(role)
        .await
        .map(|info| camera_profile_key(&info.provider, &info.info.name, role))
}

/// GET /api/settings
///
/// Get current capture settings
pub async fn get_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = state.settings.read().await;
    let response = SettingsResponse::from(&*settings);
    (StatusCode::OK, ApiResponse::ok(response))
}

/// GET /api/settings/stacking-types
///
/// Get list of available stacking types with their capabilities
pub async fn get_stacking_types() -> impl IntoResponse {
    let types: Vec<_> = StackingType::all().iter().map(|t| t.info()).collect();
    (StatusCode::OK, ApiResponse::ok(types))
}

/// Which Push-To-relevant inputs a settings update actually changes.
///
/// Deliberately about *change*, not presence. The frontend posts the whole telescope
/// block on every debounced save, so testing `is_some()` fired on saves that changed
/// nothing — and since these flags now abort and restart a plate solve, an ordinary
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

/// POST /api/settings
///
/// Update capture settings
pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateSettingsRequest>,
) -> impl IntoResponse {
    // Check if trying to change stacking_type during capture
    if request.stacking_type.is_some() {
        let current_state = state.capture_state().await;
        if current_state != CaptureState::Idle {
            return (
                StatusCode::CONFLICT,
                ApiResponse::err("Cannot change stacking type while capturing"),
            );
        }
    }

    // Which camera the hardware fields in this request are for. Absent means the
    // imaging camera, so every existing client keeps working untouched.
    let role = request.camera_role.unwrap_or(CameraRole::Main);

    let optics = optics_change(&request, &*state.settings.read().await, role);
    let cooler_fields_changed = request.cooler_enabled.is_some()
        || request.target_temp_c.is_some()
        || request.cooler_fast_mode.is_some();
    let dew_heater_fields_changed =
        request.dew_heater_enabled.is_some() || request.dew_heater_power.is_some();

    // Resolve the active camera's profile key before taking `settings.write()`
    // so we never hold `settings.write()` while awaiting `cameras.read()` —
    // elsewhere the lock order is cameras-first (e.g. disconnect), and the
    // reversed order here could deadlock under contention.
    let active_key = camera_profile_key_for(&state, role).await;

    // Same reason as `active_key`: `sync_disk_session` needs this but must not read the
    // session lock while `settings.write()` is held — `frame_processed` takes those two
    // in the opposite order.
    let capture_active = state.capture_state().await == CaptureState::Capturing;

    let applied_settings;
    {
        let mut settings = state.settings.write().await;

        if let Some(auto_stretch) = request.auto_stretch {
            settings.auto_stretch = auto_stretch;
        }

        if let Some(stacking) = request.stacking {
            settings.stacking = stacking;
        }
        if let Some(rejection_sigma) = request.rejection_sigma {
            settings.rejection_sigma = rejection_sigma.clamp(0.5, 10.0);
        }
        if let Some(rejection_method) = request.rejection_method {
            settings.rejection_method = rejection_method;
        }
        if let Some(background_subtraction) = request.background_subtraction {
            settings.background_subtraction = background_subtraction;
        }
        if let Some(algorithm) = request.background_extraction_algorithm {
            settings.background_extraction_algorithm = algorithm;
        }
        if let Some(raw_frame_saving) = request.raw_frame_saving {
            settings.raw_frame_saving = raw_frame_saving;
        }
        if let Some(save_stacked_image) = request.save_stacked_image {
            settings.save_stacked_image = save_stacked_image;
        }
        if let Some(stacking_type) = request.stacking_type {
            settings.stacking_type = stacking_type;
        }

        if let Some(weighting_preset) = request.weighting_preset {
            settings.weighting_preset = weighting_preset;
        }
        if let Some(stretch_aggressiveness) = request.stretch_aggressiveness {
            settings.stretch_aggressiveness = stretch_aggressiveness;
        }
        if let Some(auto_stretch_intensity) = request.auto_stretch_intensity {
            settings.auto_stretch_intensity = auto_stretch_intensity.clamp(0.0, 1.0);
        }
        if let Some(saturation_boost) = request.saturation_boost {
            if saturation_boost
                && crate::license::pro_plugin(&crate::render::SATURATION_PLUGIN).is_none()
            {
                return (
                    StatusCode::FORBIDDEN,
                    ApiResponse::err("Shadow Saturation Boost is a Pro feature"),
                );
            }
            settings.saturation_boost = saturation_boost;
        }
        if let Some(saturation_boost_strength) = request.saturation_boost_strength {
            settings.saturation_boost_strength = saturation_boost_strength.clamp(0.0, 1.0);
        }
        if let Some(use_simulated_camera) = request.use_simulated_camera {
            settings.use_simulated_camera = use_simulated_camera;
        }
        if let Some(simulated_preload_images) = request.simulated_preload_images {
            settings.simulated_preload_images = simulated_preload_images.max(1);
        }
        if let Some(show_focus_image) = request.show_focus_image {
            settings.show_focus_image = show_focus_image;
        }
        if let Some(force_focus_image_now) = request.force_focus_image_now {
            settings.force_focus_image_now = force_focus_image_now;
        }
        if let Some(comet_roi) = request.comet_roi {
            settings.comet_roi = Some(comet_roi);
        }
        if let Some(planetary_roi) = request.planetary_roi {
            settings.planetary_roi = Some(planetary_roi);
        }
        if let Some(auto_tracking) = request.planetary_auto_tracking {
            settings.planetary_auto_tracking = auto_tracking;
        }
        if let Some(multi_point) = request.planetary_multi_point_alignment {
            if multi_point
                && crate::license::pro_plugin(&crate::planetary::PLANETARY_PLUGIN).is_none()
            {
                return (
                    StatusCode::FORBIDDEN,
                    ApiResponse::err("Multi-Point Planetary Alignment is a Pro feature"),
                );
            }
            settings.planetary_multi_point_alignment = multi_point;
        }

        if let Some(auto_reconnect) = request.auto_reconnect {
            settings.auto_reconnect = auto_reconnect;
        }
        if let Some(auto_resume_capture) = request.auto_resume_capture {
            settings.auto_resume_capture = auto_resume_capture;
        }
        if let Some(wanderer_mode) = request.wanderer_mode {
            settings.wanderer_mode = wanderer_mode;
        }
        if let Some(denoise) = request.denoise {
            settings.denoise = denoise;
        }
        if let Some(preview_resolution) = request.preview_resolution {
            settings.preview_resolution = preview_resolution;
        }

        if let Some(sensor_correction) = request.sensor_correction {
            settings.sensor_correction = sensor_correction;
        }
        if let Some(eyepiece) = request.eyepiece {
            settings.eyepiece = eyepiece;
        }
        if let Some(telescope) = request.telescope {
            settings.telescope = telescope;
        }
        if let Some(profiles) = request.camera_telescope_profiles {
            settings.camera_telescope_profiles = profiles;
        }
        if let Some(profiles) = request.camera_profiles {
            settings.camera_profiles = profiles;
        }
        if let Some(name) = request.last_camera_name {
            settings.last_camera_name = Some(name);
        }
        if let Some(eula_accepted) = request.eula_accepted {
            settings.eula_accepted = eula_accepted;
        }
        if let Some(host) = request.indi_server_host {
            settings.indi_server_host = host;
        }
        if let Some(port) = request.indi_server_port {
            settings.indi_server_port = port;
        }

        // The ten hardware fields belong to one camera, not to the session: with a
        // guide camera connected there are two live sets, and `role` says which one this
        // request is editing. Applied as a unit so a request can never leave a camera
        // half-updated.
        {
            let mut profile = settings.profile_for(role);
            if let Some(exposure_us) = request.exposure_us {
                profile.exposure_us = exposure_us;
            }
            if let Some(gain) = request.gain {
                profile.gain = gain;
            }
            if let Some(offset) = request.offset {
                profile.offset = offset;
            }
            if let Some(bin) = request.bin {
                profile.bin = bin;
            }
            if let Some(cooler_enabled) = request.cooler_enabled {
                profile.cooler_enabled = cooler_enabled;
            }
            if let Some(target_temp_c) = request.target_temp_c {
                profile.target_temp_c = Some(target_temp_c.clamp(-60.0, 30.0));
            }
            if let Some(fast_mode) = request.cooler_fast_mode {
                profile.cooler_fast_mode = fast_mode;
            }
            if let Some(sensor_mode) = request.sensor_mode_override {
                profile.sensor_mode_override = Some(sensor_mode);
            }
            if let Some(dew_heater_enabled) = request.dew_heater_enabled {
                profile.dew_heater_enabled = dew_heater_enabled;
            }
            if let Some(dew_heater_power) = request.dew_heater_power {
                profile.dew_heater_power = dew_heater_power.clamp(0, 100);
            }

            match role {
                CameraRole::Main => profile.apply_to(&mut settings),
                CameraRole::Guide => settings.guide_camera = profile.clone(),
            }

            // Remember it against the camera itself, so reconnecting restores what the
            // user set. Skipped when the role is empty, so we don't create phantom
            // entries for a camera that is not there.
            if let Some(key) = active_key.clone() {
                settings.camera_profiles.insert(key, profile);
            }
        }

        // Snapshot for `sync_disk_session`, which runs once the write guard is gone.
        // Taken here rather than at the top of the block because which modes save is
        // part of the decision, so every mode field has to be applied first.
        applied_settings = settings.clone();

        // If exposure-impacting settings changed, cut short the exposure in flight so the
        // change takes effect now rather than at the end of the current sub.
        //
        // Scoped to `role`: the guide camera free-runs whether or not a capture is going,
        // and cancelling every slot would discard a running imaging sub because somebody
        // nudged the guide camera's gain.
        let exposure_impacting = request.exposure_us.is_some()
            || request.gain.is_some()
            || request.offset.is_some()
            || request.bin.is_some();
        let camera_is_exposing = match role {
            CameraRole::Main => state.capture_state().await == CaptureState::Capturing,
            // Always: its loop is running for as long as it is connected.
            CameraRole::Guide => state.guide_loop_running(),
        };
        if exposure_impacting && camera_is_exposing {
            tracing::info!(
                role = role.label(),
                "Exposure-impacting settings updated, cancelling the current exposure to apply them"
            );
            state.cancel_active_exposure(role).await;
        }
    }

    // Match the disk writer to the settings just applied. Deliberately outside the
    // block above: this takes `session_resume_plan`, and holding `settings.write()`
    // across another lock is an ordering constraint worth not having.
    crate::server::capture::storage::sync_disk_session(&state, &applied_settings, capture_active)
        .await;

    let _ = state.events.send(ServerEvent::SettingsUpdated);

    // Push live cooler changes to the active camera. Without this, slider
    // moves are persisted but never reach the TEC while the camera is idle —
    // the per-frame apply_cooler_config inside capture() only runs while
    // capturing.
    if cooler_fields_changed {
        crate::server::camera_session::lifecycle::apply_cooler_settings(&state, role).await;
    }

    if dew_heater_fields_changed {
        crate::server::camera_session::lifecycle::apply_dew_heater_settings(&state, role).await;
    }

    // Propagate telescope settings to plate solver for FOV calculation. Resolved
    // through `sync_solver_rig` rather than read flat, because with a guide camera
    // connected the solve runs on the *guide* scope and its focal length is what the
    // ASTAP hint has to describe.
    if optics.telescope {
        crate::server::camera_session::lifecycle::sync_solver_rig(&state).await;
    }

    // Anything that changes the field of view invalidates a solve in flight — it was
    // planned against the old optics — and equally invalidates the cached pointing.
    // Restart rather than cancel: a bare cancel leaves the star field unchanged, so
    // the movement detector reports `Idle` forever after and nothing re-solves.
    if optics.any() {
        let _ = PushToService::restart_solve(&state, "Equipment settings changed").await;
    }

    // Persist settings to disk
    state.save_settings().await;

    let settings = state.settings.read().await;
    let response = SettingsResponse::from(&*settings);
    (StatusCode::OK, ApiResponse::ok(response))
}
