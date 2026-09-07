//! Capture control API handlers

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

use super::super::dto::{
    ApiResponse, CaptureStatusResponse, MessageResponse, StartCaptureRequest, StopCaptureRequest,
};
use super::super::error::ApiError;
use super::super::services::CaptureService;
use super::super::state::{AppState, CameraRole};
use super::optional_body::OptionalBody;

/// POST /api/capture/start
///
/// Start the camera the request names. The optional body `{"role": "main" | "guide"}`
/// says which one; no body means the imaging camera, so a client that predates roles
/// keeps working.
pub async fn start_capture(
    State(state): State<Arc<AppState>>,
    OptionalBody(request): OptionalBody<StartCaptureRequest>,
) -> impl IntoResponse {
    let role = request.role.unwrap_or(CameraRole::Main);

    match CaptureService::start(&state, request.camera_id, role).await {
        Ok(camera_id) => (
            StatusCode::OK,
            ApiResponse::ok(MessageResponse {
                message: match role {
                    CameraRole::Main => "Capture started".to_string(),
                    CameraRole::Guide => "Guide camera started".to_string(),
                },
                camera_id: Some(camera_id),
            }),
        ),
        Err(e) => (e.status_code(), ApiResponse::err(e.to_string())),
    }
}

/// POST /api/capture/stop
///
/// Stop the camera the request names. See [`start_capture`] for `role`.
pub async fn stop_capture(
    State(state): State<Arc<AppState>>,
    OptionalBody(request): OptionalBody<StopCaptureRequest>,
) -> impl IntoResponse {
    let role = request.role.unwrap_or(CameraRole::Main);

    let was_running = CaptureService::stop(&state, role).await;

    let message = match (role, was_running) {
        (CameraRole::Main, true) => "Capture stopping",
        (CameraRole::Main, false) => "No capture in progress",
        (CameraRole::Guide, true) => "Guide camera stopping",
        (CameraRole::Guide, false) => "The guide camera is not running",
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
