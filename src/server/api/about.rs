use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::license::{LicenseStatus, LICENSE_UPDATER, PRO_LICENSE_DATA};
use crate::server::dto::ApiResponse;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub struct UpdateLicenseRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct SoftwareLicensesResponse {
    pub version: String,
    pub core_license: String,
    pub third_party_licenses: Option<String>,
}

/// GET /api/about/license
pub async fn get_license() -> impl IntoResponse {
    let active = crate::license::is_pro_active();
    let details = if active {
        if let Some(lock) = PRO_LICENSE_DATA.get() {
            if let Ok(data) = lock.read() {
                data.clone()
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    (
        StatusCode::OK,
        ApiResponse::ok(LicenseStatus { active, details }),
    )
}

/// POST /api/about/license
pub async fn update_license(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(payload): axum::extract::Json<UpdateLicenseRequest>,
) -> impl IntoResponse {
    if let Some(updater) = LICENSE_UPDATER.get() {
        match updater(payload.token) {
            Ok(details) => {
                // A licence activated after startup: the benchmark did not run then. Under a
                // running capture it waits for the capture's end (`end_capture_state`).
                if !super::ai_compute::capture_running(state.capture_state()) {
                    crate::render::denoise::ai::start_benchmark(&state.plugins);
                }
                (
                    StatusCode::OK,
                    ApiResponse::ok(LicenseStatus {
                        active: true,
                        details: Some(details),
                    }),
                )
            }
            Err(e) => (StatusCode::BAD_REQUEST, ApiResponse::<()>::err(&e)),
        }
    } else {
        (
            StatusCode::NOT_IMPLEMENTED,
            ApiResponse::<()>::err("License updater not registered (Community Version)"),
        )
    }
}

/// GET /api/about/software-licenses
pub async fn get_software_licenses() -> impl IntoResponse {
    let core_license = std::fs::read_to_string("LICENSE")
        .unwrap_or_else(|_| "License details not found on disk.".to_string());

    let third_party_licenses = std::fs::read_to_string("licenses.txt").ok();

    let version = crate::app::APP_VERSION
        .get()
        .cloned()
        .unwrap_or_else(|| "Unknown".to_string());

    (
        StatusCode::OK,
        ApiResponse::ok(SoftwareLicensesResponse {
            version,
            core_license,
            third_party_licenses,
        }),
    )
}
