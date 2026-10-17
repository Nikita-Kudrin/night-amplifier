//! Capabilities API handler
//!
//! Provides information about available features and plugins.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::server::dto::{
    ApiResponse, CapabilitiesResponse, CometCapabilities, DeepSkyCapabilities,
    PlanetaryCapabilities, PushToCapabilities,
};
use crate::session::state::AppState;

/// GET /api/capabilities
///
/// What the server's plugins offer while the licence is active.
pub async fn get_capabilities(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let plugins = &state.plugins;
    let has_rejection = plugins.rejection().is_some();
    let has_background = plugins.background().is_some();
    let has_push_to = plugins.push_to_solver().is_some();
    let has_saturation = plugins.saturation().is_some();
    let has_comet = plugins.comet().is_some();
    let has_denoise = plugins.denoise().is_some();
    let has_ai_denoise = plugins.ai_denoise().is_some();

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
