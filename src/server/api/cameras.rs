//! Camera operations API handlers

use crate::server::error::HttpStatus;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::session::camera::lifecycle::{DisconnectOutcome, WarmupPolicy};
use super::super::dto::{
    ApiResponse, CameraInfoResponse, CameraListEntry, ConnectCameraRequest, DisconnectCameraRequest,
    DisconnectResponse, MessageResponse, ViewedCameraBody,
};
use crate::session::services::CameraService;
use crate::session::state::{AppState, CameraRole};
use super::optional_body::OptionalBody;

#[derive(Debug, serde::Deserialize)]
pub struct ListCamerasQuery {
    /// `false` lists what the last enumeration found instead of enumerating again: what a
    /// client re-reads after a sync, which must not cost seconds of USB work each time.
    #[serde(default = "enumerate_by_default")]
    discover: bool,
}

fn enumerate_by_default() -> bool {
    true
}

/// GET /api/cameras[?discover=false]
///
/// List available cameras (both connected and discovered)
pub async fn list_cameras(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListCamerasQuery>,
) -> impl IntoResponse {
    let cameras = if query.discover {
        CameraService::list_cameras(&state).await
    } else {
        CameraService::list_known_cameras(&state).await
    };

    let cameras_list: Vec<CameraListEntry> = cameras.into_iter().map(|cam| cam.entry()).collect();

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
/// Connect to a camera. The optional body `{"role": "main" | "guide"}` says which
/// position it takes; no body means the imaging camera.
pub async fn connect_camera(
    State(state): State<Arc<AppState>>,
    Path(camera_id): Path<String>,
    body: Option<Json<ConnectCameraRequest>>,
) -> impl IntoResponse {
    let role = body
        .and_then(|Json(req)| req.role)
        .unwrap_or(CameraRole::Main);

    // Read before connecting: `connect` is idempotent and returns the existing info, so
    // asking afterwards cannot tell the two apart.
    let was_already_connected = state.roster.contains(&camera_id);

    match CameraService::connect_camera(&state, &camera_id, role).await {
        Ok(cam_info) => {
            let message = if was_already_connected {
                "Camera already connected".to_string()
            } else {
                format!(
                    "Camera '{}' connected as the {} camera",
                    cam_info.info.name,
                    role.label()
                )
            };

            (
                StatusCode::OK,
                ApiResponse::ok(MessageResponse {
                    message,
                    camera_id: Some(camera_id),
                }),
            )
        }
        Err(e) => (e.status_code(), ApiResponse::err(e.to_string())),
    }
}

/// POST /api/cameras/:camera_id/disconnect
///
/// Disconnect a camera: stops a capture running on it, and warms a cooled camera up first.
/// The optional body `{"skip_warmup": true}` closes it at once instead, including one
/// already warming up.
pub async fn disconnect_camera(
    State(state): State<Arc<AppState>>,
    Path(camera_id): Path<String>,
    OptionalBody(request): OptionalBody<DisconnectCameraRequest>,
) -> impl IntoResponse {
    let warmup = if request.skip_warmup {
        WarmupPolicy::Skip
    } else {
        WarmupPolicy::WhenPossible
    };
    match CameraService::disconnect_camera(&state, &camera_id, warmup).await {
        Ok(outcome) => {
            let (message, warming_up, remaining) = match outcome {
                DisconnectOutcome::Disconnected => ("Camera disconnected", false, None),
                DisconnectOutcome::WarmingUp { remaining } => (
                    "Camera warming up; it disconnects once warm",
                    true,
                    remaining,
                ),
            };
            (
                StatusCode::OK,
                ApiResponse::ok(DisconnectResponse {
                    message: message.to_string(),
                    camera_id,
                    warming_up,
                    warmup_remaining_s: remaining.map(|left| left.as_secs()),
                }),
            )
                .into_response()
        }
        Err(e) => (e.status_code(), ApiResponse::err::<()>(e.to_string())).into_response(),
    }
}

/// DELETE /api/cameras/recovery/{role}
///
/// Dismiss the warning automatic recovery left when it gave up on `role`'s camera. The
/// server restates a give-up on every sync until a camera connects into the role, so a
/// dismissal only on the page came back after every reconnect.
pub async fn dismiss_recovery_failure(
    State(state): State<Arc<AppState>>,
    Path(role): Path<CameraRole>,
) -> impl IntoResponse {
    let message = if CameraService::dismiss_recovery_failure(&state, role) {
        "Recovery warning dismissed."
    } else {
        "No recovery warning to dismiss."
    };
    (
        StatusCode::OK,
        ApiResponse::ok(MessageResponse {
            message: message.to_string(),
            camera_id: None,
        }),
    )
}

/// POST /api/cameras/daylight-balance/{role}/calibrate
///
/// Calibrate `role`'s daylight balance on the Planetary frame it last rendered (the Moon or
/// a white-light Sun) and save it to that camera's profile. Answers the balance saved.
pub async fn calibrate_daylight_balance(
    State(state): State<Arc<AppState>>,
    Path(role): Path<CameraRole>,
) -> impl IntoResponse {
    match CameraService::calibrate_daylight_balance(&state, role).await {
        Ok(balance) => (StatusCode::OK, ApiResponse::ok(balance)),
        Err(e) => (e.status_code(), ApiResponse::err(e.to_string())),
    }
}

/// PUT /api/view/camera
///
/// The operator's Guide toggle on `/`: which camera `/eyepiece` and `/eyepiece_quality`
/// show. Every client hears the change as `viewed_camera_changed`.
pub async fn select_viewed_camera(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ViewedCameraBody>,
) -> impl IntoResponse {
    match CameraService::select_viewed_camera(&state, request.camera) {
        Ok(()) => (
            StatusCode::OK,
            ApiResponse::ok(ViewedCameraBody {
                camera: state.viewed_camera.get(),
            }),
        ),
        Err(e) => (e.status_code(), ApiResponse::err(e.to_string())),
    }
}
