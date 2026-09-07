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
async fn test_update_settings_sensor_mode_override_roundtrip() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({ "sensor_mode_override": "low_readout_noise" }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["sensor_mode_override"], "low_readout_noise");

    let (_, json) = get_json(&app, "/api/settings").await;
    assert_eq!(json["data"]["sensor_mode_override"], "low_readout_noise");
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
async fn test_update_settings_raw_frame_saving() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let per_mode = |json: &serde_json::Value, mode: &str| {
        json["data"]["raw_frame_saving"][mode].as_bool().unwrap()
    };

    // Every mode starts off
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!per_mode(&json, "live_view"));
    assert!(!per_mode(&json, "wanderer"));
    assert!(!per_mode(&json, "stacking"));

    // The whole group is sent at once, so one mode can be enabled without the others
    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({ "raw_frame_saving": {"live_view": true, "wanderer": false, "stacking": true} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(per_mode(&json, "live_view"));
    assert!(!per_mode(&json, "wanderer"));
    assert!(per_mode(&json, "stacking"));

    // Verify the selection persists
    let (status, json) = get_json(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(per_mode(&json, "live_view"));
    assert!(per_mode(&json, "stacking"));

    // And that it can be cleared
    let (status, json) = post_json(
        &app,
        "/api/settings",
        json!({ "raw_frame_saving": {"live_view": false, "wanderer": false, "stacking": false} }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!per_mode(&json, "live_view"));
    assert!(!per_mode(&json, "stacking"));
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
    assert_eq!(settings.raw_frame_saving, RawFrameSaving::default()); // Off in every mode
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
        raw_frame_saving: RawFrameSaving::default(),
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
    assert_eq!(response.raw_frame_saving, RawFrameSaving::default());
    assert!(response.save_stacked_image);
}

#[test]
fn test_settings_response_includes_save_options() {
    use crate::server::SettingsResponse;

    let raw_frame_saving = RawFrameSaving {
        live_view: true,
        wanderer: false,
        stacking: true,
        guide: false,
    };
    let settings = CaptureSettings {
        raw_frame_saving,
        save_stacked_image: false,
        ..Default::default()
    };

    let response = SettingsResponse::from(&settings);
    assert_eq!(response.raw_frame_saving, raw_frame_saving);
    assert!(!response.save_stacked_image);
}

/// Live view saves raw frames when Live view is one of the modes selected — the whole
/// point of the per-mode switches. This used to be unconditionally off.
#[tokio::test]
async fn test_disk_writer_enabled_in_live_view_when_live_view_is_selected() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": false,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": true, "wanderer": false, "stacking": false}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(state.disk_writer.is_enabled());
}

/// Selecting a mode enables saving in that mode only. Stacking-only selection must
/// leave a Live view session writing nothing, or the switches mean nothing.
#[tokio::test]
async fn test_disk_writer_disabled_in_live_view_when_only_stacking_is_selected() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": false,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": false, "wanderer": false, "stacking": true}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_enabled_in_wanderer_when_wanderer_is_selected() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": true,
            "wanderer_mode": true,
            "raw_frame_saving": {"live_view": false, "wanderer": true, "stacking": false}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(state.disk_writer.is_enabled());
}

/// Wanderer has a stack but throws it away on every move, so the stacked-image switch
/// must not bring the writer up there on its own.
#[tokio::test]
async fn test_stacked_image_alone_does_not_enable_the_writer_in_wanderer() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": true, "save_stacked_image": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_disk_writer_enabled_in_stacking_mode() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": true,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": false, "wanderer": false, "stacking": true}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(state.disk_writer.is_enabled());
}

/// Switching into a mode that saves nothing has to take the writer back down, or it
/// keeps a session directory open for frames that will never be queued.
#[tokio::test]
async fn test_disk_writer_disabled_when_switching_to_a_mode_that_saves_nothing() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": true,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": false, "wanderer": false, "stacking": true}
        }),
    )
    .await;
    assert!(state.disk_writer.is_enabled());

    post_json(&app, "/api/settings", json!({ "stacking": false })).await;
    assert!(!state.disk_writer.is_enabled());
}

/// Flipping a switch between sessions must not open a directory: capture start opens the
/// right one, and doing it here left an empty dated folder behind on every toggle.
#[tokio::test]
async fn test_no_session_directory_is_opened_while_idle() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": false,
            "raw_frame_saving": {"live_view": true, "wanderer": false, "stacking": false}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(state.disk_writer.is_enabled());
    assert!(
        state.disk_writer.session_dir().is_none(),
        "an idle settings change opened a capture directory"
    );
}

/// The session directory records the mode that filled it. A mode change mid-capture has
/// to roll it, or stacked subs keep landing in a folder named `-live`.
#[tokio::test]
async fn test_session_directory_rolls_when_the_capture_mode_changes() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.set_capture_state(CaptureState::Capturing).await;

    post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": false,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": true, "wanderer": true, "stacking": true}
        }),
    )
    .await;

    let live_dir = state.disk_writer.session_dir().expect("a live session");
    assert_eq!(
        CaptureMode::from_session_dir_name(&state.disk_writer.session_name().unwrap()),
        Some(CaptureMode::LiveView)
    );

    post_json(&app, "/api/settings", json!({ "stacking": true })).await;

    let stacking_dir = state.disk_writer.session_dir().expect("a stacking session");
    assert_ne!(
        stacking_dir, live_dir,
        "the live-view folder was reused for a stacking capture"
    );
    assert_eq!(
        CaptureMode::from_session_dir_name(&state.disk_writer.session_name().unwrap()),
        Some(CaptureMode::Stacking)
    );
}

/// Rolling back into a mode already used this session, inside the same wall-clock
/// second, must still open a fresh folder. The timestamp alone does not distinguish
/// them, and `create_dir_all` succeeds silently on an existing directory — so the two
/// segments used to merge, and a Planetary `capture.ser` was truncated by the second.
#[tokio::test]
async fn rolling_back_within_a_second_opens_a_separate_directory() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.set_capture_state(CaptureState::Capturing).await;

    post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": true,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": true, "wanderer": true, "stacking": true}
        }),
    )
    .await;
    let first_stacking = state.disk_writer.session_dir().expect("a stacking session");

    post_json(&app, "/api/settings", json!({ "stacking": false })).await;
    let live = state.disk_writer.session_dir().expect("a live session");
    assert_ne!(first_stacking, live);

    post_json(&app, "/api/settings", json!({ "stacking": true })).await;
    let second_stacking = state.disk_writer.session_dir().expect("a stacking session");

    assert_ne!(
        second_stacking, first_stacking,
        "the second Stacking segment reopened the first segment's directory: FITS \
         frames merge into one folder and a Planetary capture.ser is truncated"
    );
}

/// Rolling must happen only on a real mode change. An unrelated settings update while
/// saving is on has to leave the open folder alone, or a session ends up scattered
/// across a folder per slider move.
#[tokio::test]
async fn test_unrelated_settings_updates_keep_the_same_session_directory() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    state.set_capture_state(CaptureState::Capturing).await;

    post_json(
        &app,
        "/api/settings",
        json!({
            "stacking": true,
            "wanderer_mode": false,
            "raw_frame_saving": {"live_view": false, "wanderer": false, "stacking": true}
        }),
    )
    .await;
    let first = state.disk_writer.session_dir();
    assert!(first.is_some());

    post_json(&app, "/api/settings", json!({ "gain": 120 })).await;

    assert_eq!(state.disk_writer.session_dir(), first);
}

#[tokio::test]
async fn test_settings_update_mirrors_to_active_profile() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state.clone());

    let (status, _json) = post_json(
        &app,
        "/api/settings",
        json!({ "gain": 300, "exposure_us": 2_000_000, "cooler_enabled": true, "target_temp_c": -10.0 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    let profile = settings
        .camera_profiles
        .get("Mock/Test Camera")
        .expect("profile should be mirrored for the active camera");
    assert_eq!(profile.gain, 300);
    assert_eq!(profile.exposure_us, 2_000_000);
    assert!(profile.cooler_enabled);
    assert_eq!(profile.target_temp_c, Some(-10.0));
}

#[tokio::test]
async fn test_settings_update_with_no_camera_does_not_create_profile() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (status, _) = post_json(&app, "/api/settings", json!({ "gain": 250 })).await;
    assert_eq!(status, StatusCode::OK);

    let settings = state.settings.read().await;
    assert!(
        settings.camera_profiles.is_empty(),
        "no camera connected → no profile should be created"
    );
}

// --- Focus/Finder mode ---------------------------------------------------------
//
// The mode is a snapshot-and-restore over seven settings, so what these cover is the
// restore: the values the observer chose have to survive a toggle, a second toggle,
// and a write that arrives while the mode is on.

/// Seed the seven managed settings to a mix of on and off, so a restore that blanket-sets
/// them either way fails rather than passing by luck. Returns nothing — read it back from
/// `/api/settings`.
async fn seed_managed_settings(app: &axum::Router) {
    let (status, _) = post_json(
        app,
        "/api/settings",
        json!({
            "background_subtraction": true,
            "sensor_correction": {
                "hot_pixel_rejection": true,
                "hot_pixel_sigma": 7.5,
                "fpn_removal": false,
                "superpixel_debayer": true,
            },
            "denoise": {
                "chroma": false,
                "chroma_strength": 0.25,
                "luma": true,
                "luma_strength": 0.75,
                "star_protection": 0.4,
            },
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_focus_mode_disables_the_managed_settings() {
    let state = create_test_state();
    let app = create_test_router(state);
    seed_managed_settings(&app).await;

    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["focus_mode"], true);
    assert_eq!(json["data"]["background_subtraction"], false);
    assert_eq!(json["data"]["saturation_boost"], false);
    assert_eq!(json["data"]["sensor_correction"]["hot_pixel_rejection"], false);
    assert_eq!(json["data"]["sensor_correction"]["fpn_removal"], false);
    assert_eq!(json["data"]["denoise"]["chroma"], false);
    assert_eq!(json["data"]["denoise"]["luma"], false);
    assert_eq!(json["data"]["eyepiece"]["dither"], false);
}

#[tokio::test]
async fn test_focus_mode_leaves_unmanaged_settings_alone() {
    let state = create_test_state();
    let app = create_test_router(state);
    seed_managed_settings(&app).await;

    let (_, json) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(json["data"]["sensor_correction"]["superpixel_debayer"], true);
    assert_eq!(json["data"]["sensor_correction"]["hot_pixel_sigma"], 7.5);
    assert_eq!(json["data"]["denoise"]["chroma_strength"], 0.25);
    assert_eq!(json["data"]["denoise"]["star_protection"], 0.4);
}

#[tokio::test]
async fn test_focus_mode_round_trip_restores_the_managed_settings() {
    let state = create_test_state();
    let app = create_test_router(state);
    seed_managed_settings(&app).await;

    let (_, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": false })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["focus_mode"], false);
    assert_eq!(json["data"]["background_subtraction"], true);
    assert_eq!(json["data"]["sensor_correction"]["hot_pixel_rejection"], true);
    assert_eq!(json["data"]["sensor_correction"]["fpn_removal"], false);
    assert_eq!(json["data"]["denoise"]["chroma"], false);
    assert_eq!(json["data"]["denoise"]["luma"], true);
    assert_eq!(json["data"]["eyepiece"]["dither"], true);
}

/// The one that destroys settings if it regresses: a second enter must not snapshot the
/// already-forced values over the observer's real ones.
#[tokio::test]
async fn test_focus_mode_enabled_twice_still_restores_the_originals() {
    let state = create_test_state();
    let app = create_test_router(state);
    seed_managed_settings(&app).await;

    for _ in 0..3 {
        let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (_, json) = post_json(&app, "/api/settings", json!({ "focus_mode": false })).await;

    assert_eq!(json["data"]["background_subtraction"], true);
    assert_eq!(json["data"]["sensor_correction"]["hot_pixel_rejection"], true);
    assert_eq!(json["data"]["denoise"]["luma"], true);
    assert_eq!(json["data"]["eyepiece"]["dither"], true);
}

#[tokio::test]
async fn test_focus_mode_wins_over_a_managed_setting_in_the_same_request() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (_, json) = post_json(
        &app,
        "/api/settings",
        json!({ "focus_mode": true, "background_subtraction": true }),
    )
    .await;

    assert_eq!(json["data"]["focus_mode"], true);
    assert_eq!(json["data"]["background_subtraction"], false);

    // ...and the value sent alongside it is what the snapshot restores.
    let (_, json) = post_json(&app, "/api/settings", json!({ "focus_mode": false })).await;
    assert_eq!(json["data"]["background_subtraction"], true);
}

/// A stale client writing a managed setting behind the mode's back must not have its write
/// silently reverted by the next toggle, and must not leak the forced value into the
/// restore either.
#[tokio::test]
async fn test_write_while_focus_mode_is_on_is_absorbed_into_the_snapshot() {
    let state = create_test_state();
    let app = create_test_router(state);
    seed_managed_settings(&app).await;
    let (_, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    let (_, json) = post_json(
        &app,
        "/api/settings",
        json!({
            "denoise": {
                "chroma": true,
                "chroma_strength": 0.9,
                "luma": false,
                "luma_strength": 0.75,
                "star_protection": 0.4,
            },
        }),
    )
    .await;
    assert_eq!(
        json["data"]["denoise"]["chroma"], false,
        "the mode must re-force it off"
    );
    assert_eq!(
        json["data"]["denoise"]["chroma_strength"], 0.9,
        "an unmanaged sibling is not the mode's business"
    );

    let (_, json) = post_json(&app, "/api/settings", json!({ "focus_mode": false })).await;
    assert_eq!(
        json["data"]["denoise"]["chroma"], true,
        "the write must survive as the restored value"
    );
    assert_eq!(json["data"]["denoise"]["chroma_strength"], 0.9);
}

#[tokio::test]
async fn test_focus_mode_is_off_by_default() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let (_, json) = get_json(&app, "/api/settings").await;

    assert_eq!(json["data"]["focus_mode"], false);
    assert!(state.settings.read().await.focus_mode_snapshot.is_none());
}

/// Focus/Finder mode drops `hot_pixel_rejection` and `fpn_removal`, which run on the raw
/// mosaic before demosaic — so the frame the accumulator integrates loses them too. Hot
/// pixels and row/column banding are exactly the defects averaging cannot remove, so a
/// stack integrated under the mode can never be cleaned again. Refused, not warned.
#[tokio::test]
async fn test_focus_mode_is_refused_while_stacking() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state.clone());

    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["success"], false);
    let settings = state.settings.read().await;
    assert!(!settings.focus_mode);
    assert!(
        settings.sensor_correction.hot_pixel_rejection,
        "the refusal must leave the stack's corrections in place"
    );
}

/// The guard is about the *accumulator*, not about the camera being busy. Live view
/// stacks nothing, and framing is exactly when the mode is wanted.
#[tokio::test]
async fn test_focus_mode_is_allowed_during_live_view() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state.clone());
    let (status, _) = post_json(&app, "/api/settings", json!({ "stacking": false })).await;
    assert_eq!(status, StatusCode::OK);

    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["focus_mode"], true);
}

/// Wanderer resets its stack when the mount moves, but it still integrates between
/// resets, so it is guarded exactly as continuous stacking is.
#[tokio::test]
async fn test_focus_mode_is_refused_in_wanderer_mode() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state.clone());
    let (status, _) = post_json(
        &app,
        "/api/settings",
        json!({ "stacking": true, "wanderer_mode": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::CONFLICT);
}

/// Never trap the observer in the mode: whatever the capture is doing, they can get their
/// settings back.
#[tokio::test]
async fn test_leaving_focus_mode_is_allowed_while_stacking() {
    let state = create_test_state();
    let app = create_test_router(state.clone());
    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;
    assert_eq!(status, StatusCode::OK);

    state.set_capture_state(CaptureState::Capturing).await;
    let (status, json) = post_json(&app, "/api/settings", json!({ "focus_mode": false })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["focus_mode"], false);
    assert_eq!(json["data"]["sensor_correction"]["hot_pixel_rejection"], true);
}

/// The refusal only guards *entering*. A request that leaves the mode where it is must not
/// be rejected just because a capture is running.
#[tokio::test]
async fn test_an_ordinary_settings_write_is_unaffected_while_stacking() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Capturing).await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/settings", json!({ "gain": 123 })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["gain"], 123);
}

/// The guard keys on "not Idle", not on "Capturing", so the window between Start being
/// accepted and the first frame arriving is covered too — that is what stops a request
/// which read `Idle` a moment earlier from landing behind a starting session.
#[tokio::test]
async fn test_focus_mode_is_refused_while_a_capture_is_starting() {
    let state = create_test_state();
    state.set_capture_state(CaptureState::Starting).await;
    let app = create_test_router(state.clone());

    let (status, _) = post_json(&app, "/api/settings", json!({ "focus_mode": true })).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!state.settings.read().await.focus_mode);
}
