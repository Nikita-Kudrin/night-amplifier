use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::license::{LicenseStatus, LICENSE_UPDATER, PRO_LICENSE_DATA};
use crate::server::dto::ApiResponse;

#[derive(Debug, Deserialize)]
pub struct UpdateLicenseRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct SoftwareLicensesResponse {
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
    axum::extract::Json(payload): axum::extract::Json<UpdateLicenseRequest>,
) -> impl IntoResponse {
    if let Some(updater) = LICENSE_UPDATER.get() {
        match updater(payload.token) {
            Ok(details) => (
                StatusCode::OK,
                ApiResponse::ok(LicenseStatus {
                    active: true,
                    details: Some(details),
                }),
            ),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                ApiResponse::<()>::err(&e),
            ),
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
    let core_license = std::fs::read_to_string("LICENSE").unwrap_or_else(|_| "License details not found on disk.".to_string());
    
    let third_party_licenses = match std::fs::read_to_string("licenses.txt") {
        Ok(text) => Some(text),
        Err(_) => None,
    };

    (
        StatusCode::OK,
        ApiResponse::ok(SoftwareLicensesResponse {
            core_license,
            third_party_licenses,
        }),
    )
}
