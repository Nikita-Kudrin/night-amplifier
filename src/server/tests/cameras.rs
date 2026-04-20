//! Tests for camera endpoints

use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;

use super::helpers::*;
use crate::camera::{CameraInfo, SensorType};
use crate::server::events::ServerEvent;
use crate::server::state::*;

// ============================================================================
// Camera List Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_list_cameras_empty() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn test_list_cameras_with_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    let cameras = json["data"].as_array().unwrap();

    // Find our mock camera
    let mock_camera = cameras
        .iter()
        .find(|c| c["id"] == "mock_0")
        .expect("Mock camera should be in list");

    assert_eq!(mock_camera["name"], "Test Camera");
    assert_eq!(mock_camera["connected"], true);
    assert_eq!(mock_camera["info"]["max_width"], 1920);
    assert_eq!(mock_camera["info"]["max_height"], 1080);
}

#[tokio::test]
async fn test_multiple_cameras_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "camera_0").await;

    // Add a second camera manually
    {
        let mut cameras = state.cameras.write().await;
        cameras.insert(
            "camera_1".to_string(),
            ConnectedCameraInfo {
                id: "camera_1".to_string(),
                provider: "Mock".to_string(),
                index: 1,
                info: CameraInfo {
                    name: "Second Camera".to_string(),
                    sensor_type: SensorType::Mono,
                    ..Default::default()
                },
            },
        );
    }

    let app = create_test_router(Arc::clone(&state));

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    let cameras = json["data"].as_array().unwrap();
    assert!(cameras.len() >= 2);

    // Check both cameras are present
    let ids: Vec<&str> = cameras.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"camera_0"));
    assert!(ids.contains(&"camera_1"));
}

// ============================================================================
// Camera Info Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_camera_info_success() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras/mock_0").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["id"], "mock_0");
    assert_eq!(json["data"]["name"], "Test Camera");
    assert_eq!(json["data"]["max_width"], 1920);
    assert_eq!(json["data"]["has_cooler"], true);
    assert_eq!(json["data"]["bit_depth"], 12);
}

#[tokio::test]
async fn test_get_camera_info_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras/nonexistent").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("not found"));
}

// ============================================================================
// Camera Connect Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_connect_camera_invalid_id_format() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/invalidformat/connect", json!({})).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("Invalid camera ID"));
}

#[tokio::test]
async fn test_connect_camera_invalid_index() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) =
        post_json(&app, "/api/cameras/provider_notanumber/connect", json!({})).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("Invalid camera index"));
}

#[tokio::test]
async fn test_connect_camera_already_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/mock_0/connect", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("already connected"));
}

#[tokio::test]
async fn test_connect_camera_provider_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/unknownprovider_0/connect", json!({})).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json["success"], false);
}

// ============================================================================
// Camera Disconnect Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_disconnect_camera_success() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("disconnected"));

    // Verify camera was removed
    let cameras = state.cameras.read().await;
    assert!(!cameras.contains_key("mock_0"));
}

#[tokio::test]
async fn test_disconnect_camera_not_connected() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/nonexistent/disconnect", json!({})).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn test_disconnect_camera_while_capturing() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("while capturing"));
}

#[tokio::test]
async fn test_disconnect_camera_clears_selected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(Arc::clone(&state));

    // Verify it's selected
    assert_eq!(
        state.selected_camera.read().await.as_deref(),
        Some("mock_0")
    );

    post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    // Verify selected was cleared
    assert!(state.selected_camera.read().await.is_none());
}

#[tokio::test]
async fn test_disconnect_camera_broadcasts_event() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let mut events_rx = state.subscribe_events();
    let app = create_test_router(Arc::clone(&state));

    post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    // Disconnect may emit CameraPhaseChanged before CameraDisconnected; loop
    // briefly until the disconnect event arrives.
    let saw_disconnect = tokio::time::timeout(
        tokio::time::Duration::from_millis(500),
        async {
            loop {
                match events_rx.recv().await {
                    Ok(ServerEvent::CameraDisconnected { .. }) => return true,
                    Ok(_) => continue,
                    Err(_) => return false,
                }
            }
        },
    )
    .await
    .unwrap_or(false);

    assert!(saw_disconnect, "Expected CameraDisconnected event");
}

// ============================================================================
// Connected Camera Info Tests
// ============================================================================

#[test]
fn test_connected_camera_info_clone() {
    let info = ConnectedCameraInfo {
        id: "test_0".to_string(),
        provider: "Test".to_string(),
        index: 0,
        info: CameraInfo {
            name: "Test Camera".to_string(),
            sensor_type: SensorType::Color,
            ..Default::default()
        },
    };

    let cloned = info.clone();

    assert_eq!(cloned.id, "test_0");
    assert_eq!(cloned.provider, "Test");
    assert_eq!(cloned.index, 0);
    assert_eq!(cloned.info.name, "Test Camera");
}
