//! INDI API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::super::dto::ApiResponse;
use super::super::state::AppState;

#[derive(Debug, Deserialize)]
pub struct IndiTestRequest {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Serialize)]
pub struct IndiTestResponse {
    pub success: bool,
    pub message: String,
}

/// POST /api/indi/test
///
/// Test connection to an INDI server. Always returns HTTP 200; the `success`
/// field in the body indicates whether the INDI server was reachable.
pub async fn test_connection(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<IndiTestRequest>,
) -> impl IntoResponse {
    let response = probe_indi(request.host, request.port).await;
    (StatusCode::OK, ApiResponse::ok(response))
}

#[cfg(feature = "indi")]
async fn probe_indi(host: String, port: u16) -> IndiTestResponse {
    let provider = crate::camera::IndiProvider::new(host.clone(), port);
    match provider.list_cameras_async().await {
        Ok(cameras) => IndiTestResponse {
            success: true,
            message: format!(
                "Connected to INDI server at {}:{}. Found {} camera(s).",
                host,
                port,
                cameras.len()
            ),
        },
        Err(e) => IndiTestResponse {
            success: false,
            message: format!("Cannot reach INDI server at {}:{}: {}", host, port, e),
        },
    }
}

#[cfg(not(feature = "indi"))]
async fn probe_indi(_host: String, _port: u16) -> IndiTestResponse {
    IndiTestResponse {
        success: false,
        message: "INDI support is not compiled in this build.".to_string(),
    }
}
