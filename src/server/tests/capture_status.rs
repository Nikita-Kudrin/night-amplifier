//! Tests for capture status endpoint

use axum::http::StatusCode;
use std::sync::Arc;

use super::helpers::*;
use crate::server::state::*;

#[tokio::test]
async fn test_capture_status_initial_state() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/capture/status").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["state"], "Idle");
    assert_eq!(json["data"]["frame_count"], 0);
    assert_eq!(json["data"]["stacked_count"], 0);
    assert_eq!(json["data"]["rejected_count"], 0);
}

#[tokio::test]
async fn test_capture_status_after_state_change() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    // Change state to Capturing
    state.set_capture_state(CaptureState::Capturing).await;

    let (status, json) = get_json(&app, "/api/capture/status").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["state"], "Capturing");
}

#[tokio::test]
async fn test_capture_status_with_frame_counts() {
    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    // Simulate some frame captures
    state.reset_session().await;
    state.frame_captured(true).await;
    state.frame_captured(true).await;
    state.frame_captured(false).await;
    state.frame_rejected("Test rejection".to_string()).await;

    let (status, json) = get_json(&app, "/api/capture/status").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["frame_count"], 4);
    assert_eq!(json["data"]["stacked_count"], 2);
    assert_eq!(json["data"]["rejected_count"], 2);
}
