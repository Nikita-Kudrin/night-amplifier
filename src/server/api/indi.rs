//! INDI API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::super::dto::ApiResponse;
use super::super::state::AppState;
use crate::camera::CameraProvider;

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
/// Test connection to an INDI server
pub async fn test_connection(
    State(_state): State<Arc<AppState>>,
    Json(request): Json<IndiTestRequest>,
) -> impl IntoResponse {
    let host = request.host;
    let port = request.port;

    #[cfg(feature = "indi")]
    {
        let provider = crate::camera::IndiProvider::new(host.clone(), port);
        match provider.list_cameras_async().await {
            Ok(cameras) => {
                let msg = format!("Successfully connected to INDI server at {}:{}. Found {} cameras.", host, port, cameras.len());
                let response = IndiTestResponse {
                    success: true,
                    message: msg,
                };
                (StatusCode::OK, ApiResponse::ok(response))
            }
            Err(e) => {
                let msg = format!("Failed to connect to INDI server at {}:{}. Error: {}", host, port, e);
                let response = IndiTestResponse {
                    success: false,
                    message: msg.clone(),
                };
                (StatusCode::BAD_REQUEST, ApiResponse::err(msg))
            }
        }
    }

    #[cfg(not(feature = "indi"))]
    {
        let msg = "INDI support is not enabled in this build.".to_string();
        (StatusCode::NOT_IMPLEMENTED, ApiResponse::err(msg))
    }
}
