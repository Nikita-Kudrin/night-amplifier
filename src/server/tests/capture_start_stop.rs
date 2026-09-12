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

/// Live view integrates nothing — `conflicts_with_capture` exempts it, and it is where the
/// observer focuses. 2026-09-07: two live-view starts (12:46:21, 13:37:30) dropped the
/// mode and the observer switched it back on by hand 2-3 s later both times.
#[tokio::test]
async fn test_starting_live_view_keeps_focus_mode() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());

    let (status, json) = post_json(&app, "/api/settings", json!({ "stacking": false })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["stacking"], false);
    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post_json(&app, "/api/capture/start", json!({})).await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    assert!(!settings.stacking);
    assert!(
        settings.focus_mode,
        "a live-view start must not take the observer out of Focus/Finder mode"
    );
    assert!(settings.focus_mode_snapshot.is_some());
    drop(settings);

    state.request_cancel();
}

/// Live view may run under the mode, so switching that same capture to stacking is the
/// third way into the conflict: the 409 only guards *entering* the mode, and the
/// start/resume paths never run. The first stacked frame must already have its corrections.
#[tokio::test]
async fn test_switching_live_view_to_stacking_leaves_focus_mode() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": false, "sensor_correction": { "fpn_removal": true } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
    assert_eq!(status, StatusCode::OK);
    state.set_capture_state(CaptureState::Capturing).await;

    let (status, _) = post_json(&app, "/api/settings", json!({ "stacking": true })).await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    assert!(settings.stacking);
    assert!(
        !settings.focus_mode,
        "a stack must never integrate frames taken under Focus/Finder mode"
    );
    assert!(settings.sensor_correction.fpn_removal);
}

/// The 409 judges the mode the request *leaves* the capture in: a check of the current
/// (live) mode let `focus_mode` and `stacking` through together.
#[tokio::test]
async fn test_focus_mode_and_stacking_in_one_request_are_refused_during_live_view() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.settings.write().await.stacking = false;
    state.set_capture_state(CaptureState::Capturing).await;

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "focus_mode": true, "stacking": true }),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    let settings = state.settings.read().await;
    assert!(!settings.focus_mode);
    assert!(!settings.stacking, "a refused request applies nothing");
}

/// The mirror image: leaving the stack for live view and entering the mode in one
/// request integrates nothing, so it must not be refused.
#[tokio::test]
async fn test_switching_to_live_view_and_entering_focus_mode_in_one_request_is_allowed() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.settings.write().await.stacking = true;
    state.set_capture_state(CaptureState::Capturing).await;

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "focus_mode": true, "stacking": false }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let settings = state.settings.read().await;
    assert!(settings.focus_mode);
    assert!(!settings.stacking);
}

async fn resume_with_focus_mode_on(stacking: bool) -> Arc<AppState> {
    use crate::server::services::CaptureService;

    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    {
        let mut settings = state.settings.write().await;
        settings.stacking = stacking;
        focus_mode::set(&mut settings, true);
    }
    state.set_capture_state(CaptureState::Recovering).await;
    let plan = SessionResumePlan {
        camera_id: "mock_0".to_string(),
        settings: state.settings.read().await.clone(),
        disk_session_dir: None,
        next_frame: 1,
    };

    CaptureService::resume_capture(&state, &plan).await.unwrap();
    state
}

/// A reconnect resumes live view too; it must not cost the observer the mode any more
/// than a fresh live-view start does.
#[tokio::test]
async fn test_resuming_live_view_keeps_focus_mode() {
    let state = resume_with_focus_mode_on(false).await;

    assert!(state.settings.read().await.focus_mode);
    state.request_cancel();
}

#[tokio::test]
async fn test_resuming_a_stack_leaves_focus_mode() {
    let state = resume_with_focus_mode_on(true).await;

    let settings = state.settings.read().await;
    assert!(!settings.focus_mode);
    assert!(settings.focus_mode_snapshot.is_none());
    drop(settings);
    state.request_cancel();
}

/// Planetary integrates, but `build_cfa_pipeline` never runs FPN for it, and the other five
/// managed settings are render-only — so the mode cannot damage a planetary stack.
#[tokio::test]
async fn test_entering_focus_mode_is_allowed_during_a_planetary_capture() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    {
        let mut settings = state.settings.write().await;
        settings.stacking = true;
        settings.stacking_type = StackingType::Planetary;
    }
    state.set_capture_state(CaptureState::Capturing).await;

    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(state.settings.read().await.focus_mode);
}

/// After Stop the capture loop checks `is_cancelled()` before it snapshots settings, so no
/// further frame can be taken under the mode: `Stopping` integrates nothing new either.
#[tokio::test]
async fn test_entering_focus_mode_is_allowed_while_a_stack_is_stopping() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.settings.write().await.stacking = true;
    state.set_capture_state(CaptureState::Stopping).await;

    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(state.settings.read().await.focus_mode);
}

/// Planetary never runs the correction the mode drops, so a planetary stack starts under it.
#[tokio::test]
async fn test_starting_a_planetary_stack_keeps_focus_mode() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());
    {
        let mut settings = state.settings.write().await;
        settings.stacking = true;
        settings.stacking_type = StackingType::Planetary;
        focus_mode::set(&mut settings, true);
    }

    let (status, _) = post_json(&app, "/api/capture/start", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert!(state.settings.read().await.focus_mode);
    state.request_cancel();
}

async fn live_view_under_focus_mode(stacking_type: StackingType) -> Arc<AppState> {
    let state = create_test_state();
    {
        let mut settings = state.settings.write().await;
        settings.stacking = false;
        settings.stacking_type = stacking_type;
        focus_mode::set(&mut settings, true);
    }
    state.set_capture_state(CaptureState::Capturing).await;
    state
}

fn focus_mode_left_announced(
    events: &mut tokio::sync::broadcast::Receiver<crate::server::events::ServerEvent>,
) -> bool {
    std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event, crate::server::events::ServerEvent::FocusModeLeft))
}

#[tokio::test]
async fn test_switching_live_view_to_planetary_stacking_keeps_focus_mode() {
    let state = live_view_under_focus_mode(StackingType::Planetary).await;
    let app = create_test_router(state.clone());

    let (status, _) = post_json(&app, "/api/settings", json!({ "stacking": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(state.settings.read().await.focus_mode);
}

/// The toggle moves behind the observer's back here, unlike a Start they pressed — so the
/// clients are told why.
#[tokio::test]
async fn test_switching_live_view_to_stacking_announces_leaving_focus_mode() {
    let state = live_view_under_focus_mode(StackingType::DeepSky).await;
    let app = create_test_router(state.clone());
    let mut events = state.events.subscribe();

    let (status, _) = post_json(&app, "/api/settings", json!({ "stacking": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(focus_mode_left_announced(&mut events));
}

#[tokio::test]
async fn test_a_settings_write_that_keeps_focus_mode_announces_nothing() {
    let state = live_view_under_focus_mode(StackingType::DeepSky).await;
    let app = create_test_router(state.clone());
    let mut events = state.events.subscribe();

    let (status, _) = post_json(&app, "/api/settings", json!({ "gain": 123 })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(state.settings.read().await.focus_mode);
    assert!(!focus_mode_left_announced(&mut events));
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
