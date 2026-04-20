//! Tests for settings endpoints

use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;

use super::helpers::*;
use crate::server::events::ServerEvent;
use crate::server::state::*;

#[tokio::test]
async fn test_get_settings_default() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/settings").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["exposure_us"], 1_000_000);
    assert_eq!(json["data"]["gain"], 0);
    assert_eq!(json["data"]["offset"], 10);
    assert_eq!(json["data"]["bin"], 1);
    assert_eq!(json["data"]["auto_stretch"], true);
    assert_eq!(json["data"]["stacking"], true);
}

#[tokio::test]
async fn test_update_settings_single_field() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/settings", json!({"exposure_us": 5_000_000})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["exposure_us"], 5_000_000);
    // Other fields should remain default
    assert_eq!(json["data"]["gain"], 0);
}

#[tokio::test]
async fn test_update_settings_cooler_fields() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({
            "cooler_enabled": true,
            "target_temp_c": -12.5,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["cooler_enabled"], true);
    assert_eq!(json["data"]["target_temp_c"], -12.5);
}

#[tokio::test]
async fn test_update_settings_target_temp_clamped() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (_, json) = post_json(&app, "/api/settings", json!({"target_temp_c": -200.0})).await;
    assert_eq!(json["data"]["target_temp_c"], -60.0);

    let (_, json) = post_json(&app, "/api/settings", json!({"target_temp_c": 999.0})).await;
    assert_eq!(json["data"]["target_temp_c"], 30.0);
}

#[tokio::test]
async fn test_update_settings_multiple_fields() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({
            "exposure_us": 2_000_000,
            "gain": 150,
            "offset": 20,
            "bin": 2,
            "auto_stretch": false,
            "stacking": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["exposure_us"], 2_000_000);
    assert_eq!(json["data"]["gain"], 150);
    assert_eq!(json["data"]["offset"], 20);
    assert_eq!(json["data"]["bin"], 2);
    assert_eq!(json["data"]["auto_stretch"], false);
    assert_eq!(json["data"]["stacking"], false);
}

#[tokio::test]
async fn test_update_settings_rejection_sigma_clamped() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Test upper bound clamping
    let (status, json) = post_json(&app, "/api/settings", json!({"rejection_sigma": 15.0})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["rejection_sigma"], 10.0);

    // Test lower bound clamping
    let (status, json) = post_json(&app, "/api/settings", json!({"rejection_sigma": 0.1})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["rejection_sigma"], 0.5);
}

#[tokio::test]
async fn test_update_settings_empty_request() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Empty object should not change anything
    let (status, json) = post_json(&app, "/api/settings", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["exposure_us"], 1_000_000); // Default value
}

#[tokio::test]
async fn test_update_settings_broadcasts_event() {
    let state = create_test_state();
    let mut events_rx = state.subscribe_events();
    let app = create_test_router(Arc::clone(&state));

    post_json(&app, "/api/settings", json!({"gain": 100})).await;

    // Check that event was broadcast
    let event = tokio::time::timeout(tokio::time::Duration::from_millis(100), events_rx.recv())
        .await
        .expect("Timeout waiting for event")
        .expect("Failed to receive event");

    assert!(matches!(event, ServerEvent::SettingsUpdated { .. }));
}

#[tokio::test]
async fn test_settings_with_null_values() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Null values should be ignored (fields are Optional)
    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({
            "exposure_us": null,
            "gain": 100
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["gain"], 100);
    assert_eq!(json["data"]["exposure_us"], 1_000_000); // Default unchanged
}

#[tokio::test]
async fn test_settings_persist_across_requests() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Update setting
    post_json(&app, "/api/settings", json!({"exposure_us": 5_000_000})).await;

    // Read it back
    let (_, json) = get_json(&app, "/api/settings").await;
    assert_eq!(json["data"]["exposure_us"], 5_000_000);

    // Update another setting
    post_json(&app, "/api/settings", json!({"gain": 200})).await;

    // Both should be set
    let (_, json) = get_json(&app, "/api/settings").await;
    assert_eq!(json["data"]["exposure_us"], 5_000_000);
    assert_eq!(json["data"]["gain"], 200);
}

#[tokio::test]
async fn test_update_settings_save_raw_frames() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Initially save_raw_frames should be false (default)
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!json["data"]["save_raw_frames"].as_bool().unwrap());

    // Enable save_raw_frames
    let (status, json) = post_json(&app, "/api/settings", json!({ "save_raw_frames": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data"]["save_raw_frames"].as_bool().unwrap());

    // Verify setting persists
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data"]["save_raw_frames"].as_bool().unwrap());

    // Disable save_raw_frames
    let (status, json) =
        post_json(&app, "/api/settings", json!({ "save_raw_frames": false })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!json["data"]["save_raw_frames"].as_bool().unwrap());
}

#[tokio::test]
async fn test_update_settings_save_stacked_image() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Initially save_stacked_image should be false (default)
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!json["data"]["save_stacked_image"].as_bool().unwrap());

    // Enable save_stacked_image
    let (status, json) =
        post_json(&app, "/api/settings", json!({ "save_stacked_image": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data"]["save_stacked_image"].as_bool().unwrap());

    // Verify setting persists
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data"]["save_stacked_image"].as_bool().unwrap());

    // Disable save_stacked_image
    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({ "save_stacked_image": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!json["data"]["save_stacked_image"].as_bool().unwrap());
}

#[test]
fn test_capture_settings_default_values() {
    let settings = CaptureSettings::default();

    assert_eq!(settings.exposure_us, 1_000_000);
    assert_eq!(settings.gain, 0);
    assert_eq!(settings.offset, 10);
    assert_eq!(settings.bin, 1);
    assert!(settings.auto_stretch);
    assert!(settings.stacking);
    assert_eq!(settings.rejection_sigma, 2.5);
    assert!(settings.background_subtraction);
}

#[test]
fn test_capture_settings_default_save_frames() {
    let settings = CaptureSettings::default();
    assert!(!settings.save_raw_frames); // Should be disabled by default
    assert!(!settings.save_stacked_image); // Should be disabled by default
}

#[test]
fn test_capture_settings_to_capture_config() {
    let settings = CaptureSettings {
        exposure_us: 3_000_000,
        gain: 150,
        offset: 25,
        bin: 2,
        ..Default::default()
    };

    let config = settings.to_capture_config();

    assert_eq!(config.exposure_us, 3_000_000);
    assert_eq!(config.gain, 150);
    assert_eq!(config.offset, 25);
    assert_eq!(config.bin, 2);
}

#[test]
fn test_settings_response_from_settings() {
    use crate::server::SettingsResponse;

    let settings = CaptureSettings {
        exposure_us: 5_000_000,
        gain: 200,
        offset: 30,
        bin: 2,
        auto_stretch: false,
        stacking: false,
        rejection_sigma: 3.0,
        background_subtraction: false,
        save_raw_frames: false,
        save_stacked_image: true,
        stacking_type: StackingType::DeepSky,
        ..Default::default()
    };

    let response = SettingsResponse::from(&settings);

    assert_eq!(response.exposure_us, 5_000_000);
    assert_eq!(response.gain, 200);
    assert_eq!(response.offset, 30);
    assert_eq!(response.bin, 2);
    assert!(!response.auto_stretch);
    assert!(!response.stacking);
    assert_eq!(response.rejection_sigma, 3.0);
    assert!(!response.background_subtraction);
    assert!(!response.save_raw_frames);
    assert!(response.save_stacked_image);
}

#[test]
fn test_settings_response_includes_save_options() {
    use crate::server::SettingsResponse;

    let settings = CaptureSettings {
        save_raw_frames: true,
        save_stacked_image: false,
        ..Default::default()
    };

    let response = SettingsResponse::from(&settings);
    assert!(response.save_raw_frames);
    assert!(!response.save_stacked_image);
}

#[tokio::test]
async fn test_disk_writer_disabled_in_live_view_mode() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Set live view mode (stacking=false) and enable save_raw_frames
    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": false, "wanderer_mode": false, "save_raw_frames": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_disabled_in_wanderer_mode() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Set wanderer mode (stacking=true, wanderer_mode=true) and enable both save options
    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": true, "save_raw_frames": true, "save_stacked_image": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_enabled_in_stacking_mode() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Set stacking mode (stacking=true, wanderer_mode=false) and enable save_raw_frames
    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": false, "save_raw_frames": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_disabled_when_switching_from_stacking_to_live_view() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Enable stacking mode + save_raw_frames
    post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": false, "save_raw_frames": true }),
    )
    .await;
    assert!(state.disk_writer.is_enabled());

    // Switch to live view
    post_json(&app, "/api/settings", json!({ "stacking": false })).await;
    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_disabled_when_switching_from_stacking_to_wanderer() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Enable stacking mode + save_stacked_image
    post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": false, "save_stacked_image": true }),
    )
    .await;
    assert!(state.disk_writer.is_enabled());

    // Switch to wanderer mode
    post_json(&app, "/api/settings", json!({ "wanderer_mode": true })).await;
    assert!(!state.disk_writer.is_enabled());
}
