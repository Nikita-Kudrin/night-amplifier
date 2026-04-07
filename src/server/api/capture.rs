//! Capture control API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::sync::Arc;

use super::super::dto::{ApiResponse, CaptureStatusResponse, MessageResponse, StartCaptureRequest};
use super::super::error::ApiError;
use super::super::services::CaptureService;
use super::super::state::AppState;

/// POST /api/capture/start
///
/// Start a new capture session
pub async fn start_capture(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartCaptureRequest>,
) -> impl IntoResponse {
    match CaptureService::start_capture(&state, request.camera_id).await {
        Ok(camera_id) => (
            StatusCode::OK,
            ApiResponse::ok(MessageResponse {
                message: "Capture started".to_string(),
                camera_id: Some(camera_id),
            }),
        ),
        Err(e) => {
            let status = match &e {
                ApiError::CaptureInProgress => StatusCode::CONFLICT,
                ApiError::NoCameraSelected => StatusCode::BAD_REQUEST,
                ApiError::CameraNotConnected(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, ApiResponse::err(e.to_string()))
        }
    }
}

/// POST /api/capture/stop
///
/// Stop the current capture session
pub async fn stop_capture(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let was_capturing = CaptureService::stop_capture(&state).await;

    let message = if was_capturing {
        "Capture stopping"
    } else {
        "No capture in progress"
    };

    (
        StatusCode::OK,
        ApiResponse::ok(MessageResponse {
            message: message.to_string(),
            camera_id: None,
        }),
    )
}

/// GET /api/capture/status
///
/// Get current capture status
pub async fn get_capture_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let session = state.session.read().await;
    let response = CaptureStatusResponse::from(&*session);
    (StatusCode::OK, ApiResponse::ok(response))
}
