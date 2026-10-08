//! Settings API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use super::super::dto::{ApiResponse, SettingsResponse, UpdateSettingsRequest};
use crate::session::services::SettingsService;
use crate::session::state::{AppState, StackingType};

/// GET /api/settings
///
/// Get current capture settings
pub async fn get_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let settings = state.settings.snapshot();
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
    if let Err(refused) = SettingsService::update(&state, request).await {
        return (refused.status_code(), ApiResponse::err(refused.to_string()));
    }
    let settings = state.settings.snapshot();
    (StatusCode::OK, ApiResponse::ok(SettingsResponse::from(&*settings)))
}
