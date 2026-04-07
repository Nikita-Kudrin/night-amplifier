//! Push-To navigation API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use super::super::dto::{
    ApiResponse, MessageResponse, PushToConfigRequest, PushToStatusResponse, SearchCatalogRequest,
    SetTargetRequest,
};
use super::super::services::PushToService;
use super::super::state::AppState;
// Removed CatalogType

/// GET /api/push-to/status
///
/// Get the current Push-To navigation status
pub async fn get_push_to_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let status = PushToService::get_status(&state).await;
    (StatusCode::OK, ApiResponse::ok(status))
}

/// POST /api/push-to/target
///
/// Set the current target (by name or coordinates)
pub async fn set_push_to_target(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SetTargetRequest>,
) -> impl IntoResponse {
    // Try by name first
    if let Some(ref name) = request.name {
        match PushToService::set_target_by_name(&state, name).await {
            Ok(entry) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "data": entry
                    })),
                )
            }
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "success": false,
                        "error": e
                    })),
                )
            }
        }
    }

    // Try by coordinates
    if let (Some(ra), Some(dec)) = (request.ra_degrees, request.dec_degrees) {
        match PushToService::set_target_by_coords(&state, ra, dec).await {
            Ok(coord) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "success": true,
                        "data": coord
                    })),
                )
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "success": false,
                        "error": e
                    })),
                )
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "success": false,
            "error": "Must provide either 'name' or both 'ra_degrees' and 'dec_degrees'"
        })),
    )
}

/// DELETE /api/push-to/target
///
/// Clear the current target
pub async fn clear_push_to_target(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match PushToService::clear_target(&state).await {
        Ok(()) => (
            StatusCode::OK,
            ApiResponse::ok(MessageResponse {
                message: "Target cleared".to_string(),
                camera_id: None,
            }),
        ),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, ApiResponse::err(e)),
    }
}

/// GET /api/push-to/direction
///
/// Get the current push direction to target
pub async fn get_push_to_direction(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match PushToService::get_direction(&state).await {
        Some(direction) => (StatusCode::OK, ApiResponse::ok(direction)),
        None => (
            StatusCode::NOT_FOUND,
            ApiResponse::err("No position or target set"),
        ),
    }
}

/// GET /api/push-to/catalog/search
///
/// Search the target catalog
pub async fn search_catalog(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<SearchCatalogRequest>,
) -> impl IntoResponse {
    let results = PushToService::search_catalog(&state, &params.query, params.limit).await;
    (StatusCode::OK, ApiResponse::ok(results))
}

/// GET /api/push-to/catalog/messier
///
/// Get all Messier objects
pub async fn get_messier_catalog(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let results = PushToService::get_catalog_by_type(&state, "Messier").await;
    (StatusCode::OK, ApiResponse::ok(results))
}

/// GET /api/push-to/catalog/ngc
///
/// Get NGC objects
pub async fn get_ngc_catalog(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let results = PushToService::get_catalog_by_type(&state, "NGC").await;
    (StatusCode::OK, ApiResponse::ok(results))
}

/// GET /api/push-to/catalog/ic
///
/// Get IC objects
pub async fn get_ic_catalog(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let results = PushToService::get_catalog_by_type(&state, "IC").await;
    (StatusCode::OK, ApiResponse::ok(results))
}

/// POST /api/push-to/config
///
/// Update Push-To configuration (FOV, database path)
pub async fn update_push_to_config(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PushToConfigRequest>,
) -> impl IntoResponse {
    // Update FOV if provided
    if let Some(fov) = request.fov_degrees {
        if let Err(e) = PushToService::set_fov(&state, fov).await {
            return (StatusCode::INTERNAL_SERVER_ERROR, ApiResponse::err(e));
        }
    }

    // Load database if path provided
    if let Some(ref path) = request.database_path {
        if let Err(e) = PushToService::load_database(&state, path).await {
            return (StatusCode::BAD_REQUEST, ApiResponse::err(e));
        }
    }

    let status = PushToService::get_status(&state).await;
    (StatusCode::OK, ApiResponse::ok(status))
}
