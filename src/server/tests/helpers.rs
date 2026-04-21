//! Test helpers for server API tests

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

use crate::camera::{CameraInfo, ImageFormat, SensorType};
use crate::server::api::*;
use crate::server::state::*;
use crate::CfaPattern;

/// Create a test app state with default configuration (no settings persistence)
pub fn create_test_state() -> Arc<AppState> {
    let (state, _disk_writer) = AppState::new_for_testing();
    Arc::new(state)
}

/// Create a test router with the given state
pub fn create_test_router(state: Arc<AppState>) -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/api/capabilities", get(get_capabilities))
        .route("/api/capture/start", post(start_capture))
        .route("/api/capture/stop", post(stop_capture))
        .route("/api/capture/status", get(get_capture_status))
        .route("/api/settings", get(get_settings))
        .route("/api/settings", post(update_settings))
        .route("/api/cameras", get(list_cameras))
        .route("/api/cameras/{camera_id}", get(get_camera_info))
        .route("/api/cameras/{camera_id}/connect", post(connect_camera))
        .route(
            "/api/cameras/{camera_id}/disconnect",
            post(disconnect_camera),
        )
        .with_state(state)
}

/// Helper to make a GET request and return status + JSON body
pub async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));

    (status, json)
}

/// Helper to make a POST request with JSON body
pub async fn post_json(app: &axum::Router, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));

    (status, json)
}

/// Add a mock camera to the state for testing
pub async fn add_mock_camera(state: &Arc<AppState>, camera_id: &str) {
    let info = CameraInfo {
        name: "Test Camera".to_string(),
        id: 0,
        max_width: 1920,
        max_height: 1080,
        pixel_size_x_um: 2.9,
        pixel_size_y_um: 2.9,
        sensor_type: SensorType::Color,
        bayer_pattern: Some(CfaPattern::Rggb),
        has_cooler: true,
        min_temp_c: Some(-40.0),
        max_temp_c: Some(20.0),
        has_shutter: false,
        is_usb3: true,
        bit_depth: 12,
        supported_bins: vec![1, 2, 4],
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
        min_exposure_us: 100,
        max_exposure_us: 3600_000_000,
        min_gain: 0,
        max_gain: 500,
        unity_gain: 100,
        hcg_gain: 120,
        sensor_modes: Vec::new(),
    };

    let mut cameras = state.cameras.write().await;
    cameras.insert(
        camera_id.to_string(),
        ConnectedCameraInfo {
            id: camera_id.to_string(),
            provider: "Mock".to_string(),
            index: 0,
            info,
        },
    );

    // Set as selected camera
    let mut selected = state.selected_camera.write().await;
    *selected = Some(camera_id.to_string());
}
