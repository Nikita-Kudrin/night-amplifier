//! Tests for capture start and stop endpoints

use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;

use super::helpers::*;
use crate::server::state::*;

// ============================================================================
// Capture Start Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_capture_start_no_camera_selected() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/capture/start", json!({})).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("No camera selected"));
}

#[tokio::test]
async fn test_capture_start_camera_not_connected() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(
        &app,
        "/api/capture/start",
        json!({"camera_id": "nonexistent_0"}),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("not connected"));
}

#[tokio::test]
async fn test_capture_start_success() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/capture/start", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("started"));
    assert_eq!(json["data"]["camera_id"], "mock_0");

    // Give the capture loop time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Note: The capture loop will fail to open the mock camera (it's not a real
    // camera in the registry), so it will return to Idle. But the API call itself
    // succeeded, which is what we're testing here.
    // In a real scenario with a connected camera, the state would be Capturing.

    // Clean up - stop capture
    state.request_cancel();
}

#[tokio::test]
async fn test_capture_start_already_capturing() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/capture/start", json!({})).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("already in progress"));
}

#[tokio::test]
async fn test_capture_start_with_specific_camera() {
    let state = create_test_state();
    add_mock_camera(&state, "specific_camera_0").await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(
        &app,
        "/api/capture/start",
        json!({"camera_id": "specific_camera_0"}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["camera_id"], "specific_camera_0");

    // Clean up
    state.request_cancel();
}

#[tokio::test]
async fn test_capture_start_while_stopping() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    state.set_capture_state(CaptureState::Stopping).await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/capture/start", json!({})).await;

    // Should allow starting when in Stopping state
    assert_eq!(status, StatusCode::OK);
}

// ============================================================================
// Capture Stop Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_capture_stop_when_idle() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/capture/stop", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("No capture in progress"));
}

#[tokio::test]
async fn test_capture_stop_when_capturing() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/capture/stop", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("stopping"));

    // Verify state changed to Stopping
    assert_eq!(state.capture_state().await, CaptureState::Stopping);
    assert!(state.is_cancelled());
}
