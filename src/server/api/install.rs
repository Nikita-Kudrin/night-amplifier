//! ASTAP and Catalog installation API handlers
//!
//! In the Community version, these endpoints delegate to the PushToSystemPlugin if present,
//! otherwise they return an error indicating the feature requires the Pro version.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;
use tracing::info;

use super::super::dto::{
    ApiResponse, AstapInstallRequest, AstapStatusResponse, CatalogStatusResponse,
    DatabaseTypeResponse, MessageResponse,
};
use super::super::state::AppState;
use crate::push_to::PUSH_TO_PLUGIN;

/// GET /api/astap/status
///
/// Get ASTAP installation status
pub async fn get_astap_status() -> impl IntoResponse {
    if let Some(plugin) = PUSH_TO_PLUGIN.get() {
        let status = plugin.get_astap_status().await;
        (StatusCode::OK, ApiResponse::ok(status))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(AstapStatusResponse {
                binary_installed: false,
                binary_path: None,
                database_installed: false,
                database_path: None,
                database_type: None,
                ready: false,
            }),
        )
    }
}

/// GET /api/astap/databases
///
/// Get available database types for installation (in display order)
pub async fn get_astap_databases() -> impl IntoResponse {
    if let Some(plugin) = PUSH_TO_PLUGIN.get() {
        let databases = plugin.get_astap_databases().await;
        (StatusCode::OK, ApiResponse::ok(databases))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(Vec::<DatabaseTypeResponse>::new()),
        )
    }
}

/// POST /api/astap/install
///
/// Start ASTAP installation (binary and database)
pub async fn install_astap(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<AstapInstallRequest>,
) -> impl IntoResponse {
    if let Some(plugin) = PUSH_TO_PLUGIN.get() {
        match plugin
            .install_astap(&request.database_type, _state.events.clone())
            .await
        {
            Ok(_) => (
                StatusCode::ACCEPTED,
                ApiResponse::ok(MessageResponse {
                    message: "Installation started.".to_string(),
                    camera_id: None,
                }),
            ),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, ApiResponse::err(e)),
        }
    } else {
        (
            StatusCode::FORBIDDEN,
            ApiResponse::err("ASTAP installation requires Night Amplifier Pro."),
        )
    }
}

/// GET /api/catalog/status
///
/// Get OpenNGC catalog installation status
pub async fn get_catalog_status() -> impl IntoResponse {
    if let Some(plugin) = PUSH_TO_PLUGIN.get() {
        let status = plugin.get_catalog_status().await;
        (StatusCode::OK, ApiResponse::ok(status))
    } else {
        (
            StatusCode::OK,
            ApiResponse::ok(CatalogStatusResponse {
                installed: false,
                catalog_path: None,
                ngc_file_exists: false,
                addendum_file_exists: false,
                object_count: None,
            }),
        )
    }
}

/// POST /api/catalog/install
///
/// Start OpenNGC catalog installation (downloads NGC.csv and addendum.csv)
pub async fn install_catalog(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Some(plugin) = PUSH_TO_PLUGIN.get() {
        match plugin.install_catalog(_state.events.clone()).await {
            Ok(_) => (
                StatusCode::ACCEPTED,
                ApiResponse::ok(MessageResponse {
                    message: "Catalog installation started.".to_string(),
                    camera_id: None,
                }),
            ),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, ApiResponse::err(e)),
        }
    } else {
        (
            StatusCode::FORBIDDEN,
            ApiResponse::err("Catalog installation requires Night Amplifier Pro."),
        )
    }
}
