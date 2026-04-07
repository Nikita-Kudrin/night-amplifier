//! Settings API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use super::super::dto::{ApiResponse, SettingsResponse, UpdateSettingsRequest};
use super::super::events::ServerEvent;
use super::super::state::{AppState, CaptureState, StackingType};

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

    {
        let mut settings = state.settings.write().await;

        if let Some(exposure_us) = request.exposure_us {
            settings.exposure_us = exposure_us;
        }
        if let Some(gain) = request.gain {
            settings.gain = gain;
        }
        if let Some(offset) = request.offset {
            settings.offset = offset;
        }
        if let Some(bin) = request.bin {
            settings.bin = bin;
        }
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
        if let Some(save_raw_frames) = request.save_raw_frames {
            settings.save_raw_frames = save_raw_frames;
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
        if let Some(saturation_boost) = request.saturation_boost {
            if saturation_boost && crate::render::SATURATION_PLUGIN.get().is_none() {
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
        if let Some(comet_roi) = request.comet_roi {
            settings.comet_roi = Some(comet_roi);
        }

        if let Some(wanderer_mode) = request.wanderer_mode {
            settings.wanderer_mode = wanderer_mode;
        }
        if let Some(eyepiece) = request.eyepiece {
            settings.eyepiece = eyepiece;
        }

        // Enable disk writer only in stacking mode (not live view or wanderer)
        // This must be done after all mode settings are updated
        let is_stacking_mode = settings.stacking && !settings.wanderer_mode;
        let disk_enabled =
            is_stacking_mode && (settings.save_raw_frames || settings.save_stacked_image);
        state.disk_writer.set_enabled(disk_enabled);
    }

    let _ = state.events.send(ServerEvent::SettingsUpdated);

    // Persist settings to disk
    state.save_settings().await;

    let settings = state.settings.read().await;
    let response = SettingsResponse::from(&*settings);
    (StatusCode::OK, ApiResponse::ok(response))
}
