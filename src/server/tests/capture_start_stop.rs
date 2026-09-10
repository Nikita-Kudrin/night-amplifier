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
    // Names what is missing rather than the internal notion of "selected": the settings
    // panel's selection can be the guide camera, so "no camera selected" described a
    // state the user could be looking straight at a connected camera in.
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("No imaging camera is connected"));
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

    let (status, _json) = post_json(&app, "/api/capture/start", json!({})).await;

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

/// The web client sends `Content-Type: application/json` with no body at all for a
/// main-camera stop — the exact request shape that predates roles. The handler must
/// still read it as the imaging camera rather than rejecting it as malformed JSON.
#[tokio::test]
async fn a_stop_with_a_json_content_type_and_no_body_stops_the_capture() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/capture/stop")
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(state.capture_state().await, CaptureState::Stopping);
}

/// The other order of the same conflict: the observer was already focusing, then pressed
/// Start. `update_settings` refuses to *enter* the mode while stacking, so this is what
/// keeps a session from ever beginning under it — and it restores the observer's own
/// settings at the moment they start mattering.
#[tokio::test]
async fn test_starting_a_capture_leaves_focus_mode_and_restores_the_settings() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "sensor_correction": {
                "hot_pixel_sigma": 7.5,
                "fpn_removal": true,
                "superpixel_debayer": false,
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["sensor_correction"]["fpn_removal"], false);

    let (status, _) = post_json(&app, "/api/capture/start", json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    assert!(!settings.focus_mode, "a capture must never begin under the mode");
    assert!(settings.focus_mode_snapshot.is_none());
    assert!(
        settings.sensor_correction.fpn_removal,
        "the stack's corrections must be back before the first frame"
    );
    assert_eq!(settings.sensor_correction.hot_pixel_sigma, 7.5);
    drop(settings);

    state.request_cancel();
}

/// Starting a capture that was never focusing must not touch the settings at all.
#[tokio::test]
async fn test_starting_a_capture_without_focus_mode_changes_nothing() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "background_subtraction": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post_json(&app, "/api/capture/start", json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    assert!(!settings.focus_mode);
    assert!(
        !settings.background_subtraction,
        "an unrelated setting the observer turned off must stay off"
    );
    drop(settings);

    state.request_cancel();
}
