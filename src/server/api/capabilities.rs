//! Capabilities API handler
//!
//! Provides information about available features and plugins.

use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::background::BACKGROUND_PLUGIN;
use crate::push_to::PUSH_TO_PLUGIN;
use crate::render::{AI_DENOISE_PLUGIN, DENOISE_PLUGIN, SATURATION_PLUGIN};
use crate::server::dto::{
    ApiResponse, CapabilitiesResponse, CometCapabilities, DeepSkyCapabilities,
    PlanetaryCapabilities, PushToCapabilities,
};
use crate::stacking::{COMET_PLUGIN, REJECTION_PLUGIN};

/// GET /api/capabilities
///
/// Returns the current server capabilities based on registered plugins.
pub async fn get_capabilities() -> impl IntoResponse {
    let active = crate::license::is_pro_active();
    let has_rejection = active && REJECTION_PLUGIN.get().is_some();
    let has_background = active && BACKGROUND_PLUGIN.get().is_some();
    let has_push_to = active && PUSH_TO_PLUGIN.get().is_some();
    let has_saturation = active && SATURATION_PLUGIN.get().is_some();
    let has_comet = active && COMET_PLUGIN.get().is_some();
    let has_denoise = active && DENOISE_PLUGIN.get().is_some();
    let has_ai_denoise = active && AI_DENOISE_PLUGIN.get().is_some();

    let has_pro = has_rejection
        || has_background
        || has_push_to
        || has_saturation
        || has_comet
        || has_denoise
        || has_ai_denoise;

    let response = CapabilitiesResponse {
        has_pro,
        deep_sky: DeepSkyCapabilities {
            advanced_rejection: has_rejection,
            rbf_background: has_background,
            saturation_boost: has_saturation,
            denoise: has_denoise,
            ai_denoise: has_ai_denoise,
        },
        planetary: PlanetaryCapabilities {
            advanced_stacking: true,
        },
        push_to: PushToCapabilities {
            astap_solver: has_push_to,
        },
        comet: CometCapabilities {
            pro_stacking: has_comet,
        },
        debug_logging: tracing::enabled!(tracing::Level::DEBUG),
    };

    (StatusCode::OK, ApiResponse::ok(response))
}
