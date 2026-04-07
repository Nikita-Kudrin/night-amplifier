//! Camera operations API handlers

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use super::super::dto::{ApiResponse, CameraInfoResponse, CameraListEntry, MessageResponse};
use super::super::error::ApiError;
use super::super::services::CameraService;
use super::super::state::AppState;

/// GET /api/cameras
///
/// List available cameras (both connected and discovered)
pub async fn list_cameras(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cameras = CameraService::list_cameras(&state).await;

    let cameras_list: Vec<CameraListEntry> = cameras
        .into_iter()
        .map(|cam| CameraListEntry {
            id: cam.id.clone(),
            name: cam.name,
            connected: cam.connected,
            provider: cam.provider,
            index: cam.index,
            info: CameraInfoResponse::from_info(&cam.info, &cam.id),
        })
        .collect();

    (StatusCode::OK, ApiResponse::ok(cameras_list))
}

/// GET /api/cameras/:camera_id
///
/// Get detailed info for a specific camera
pub async fn get_camera_info(
    State(state): State<Arc<AppState>>,
    Path(camera_id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CameraInfoResponse>>) {
    match CameraService::get_camera_info(&state, &camera_id).await {
        Ok(cam_info) => {
            let response = CameraInfoResponse::from_info(&cam_info.info, &camera_id);
            (StatusCode::OK, ApiResponse::ok(response))
        }
        Err(e) => (StatusCode::NOT_FOUND, ApiResponse::err(e.to_string())),
    }
}

/// POST /api/cameras/:camera_id/connect
///
/// Connect to a camera
pub async fn connect_camera(
    State(state): State<Arc<AppState>>,
    Path(camera_id): Path<String>,
) -> impl IntoResponse {
    match CameraService::connect_camera(&state, &camera_id).await {
        Ok(cam_info) => {
            // Check if this was already connected (service returns existing info)
            let cameras = state.cameras.read().await;
            let was_already_connected = cameras.len() > 0
                && cameras.get(&camera_id).map(|c| &c.info.name) == Some(&cam_info.info.name);
            drop(cameras);

            let message = if was_already_connected {
                "Camera already connected".to_string()
            } else {
                format!("Camera '{}' connected", cam_info.info.name)
            };

            (
                StatusCode::OK,
                ApiResponse::ok(MessageResponse {
                    message,
                    camera_id: Some(camera_id),
                }),
            )
        }
        Err(e) => {
            let status = match &e {
                ApiError::InvalidCameraIdFormat | ApiError::InvalidCameraIndex => {
                    StatusCode::BAD_REQUEST
                }
                ApiError::CameraOpenFailed(_) | ApiError::Internal(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, ApiResponse::err(e.to_string()))
        }
    }
}

/// POST /api/cameras/:camera_id/disconnect
///
/// Disconnect from a camera
pub async fn disconnect_camera(
    State(state): State<Arc<AppState>>,
    Path(camera_id): Path<String>,
) -> impl IntoResponse {
    match CameraService::disconnect_camera(&state, &camera_id).await {
        Ok(_camera_name) => (
            StatusCode::OK,
            ApiResponse::ok(MessageResponse {
                message: "Camera disconnected".to_string(),
                camera_id: Some(camera_id),
            }),
        ),
        Err(e) => {
            let status = match &e {
                ApiError::CameraInUse => StatusCode::CONFLICT,
                ApiError::CameraNotConnected(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, ApiResponse::err(e.to_string()))
        }
    }
}
