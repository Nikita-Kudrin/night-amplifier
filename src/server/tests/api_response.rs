//! Tests for API response types

use axum::http::StatusCode;

use super::helpers::*;
use crate::camera::{CameraInfo, SensorType};
use crate::server::state::{CaptureSession, CaptureSettings, CaptureState, StackingType};
use crate::server::{CameraInfoResponse, CaptureStatusResponse, SettingsResponse};

// ============================================================================
// API Response Format Tests
// ============================================================================

#[tokio::test]
async fn test_success_response_format() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (_, json) = get_json(&app, "/api/capture/status").await;

    // Check response structure
    assert!(json.get("success").is_some());
    assert!(json.get("data").is_some());
    assert!(json.get("error").is_none() || json["error"].is_null());
}

#[tokio::test]
async fn test_error_response_format() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (_, json) = get_json(&app, "/api/cameras/nonexistent").await;

    // Check error response structure
    assert_eq!(json["success"], false);
    assert!(json.get("error").is_some());
    assert!(!json["error"].is_null());
}

// ============================================================================
// Camera Info Response Tests
// ============================================================================

#[test]
fn test_camera_info_response_from_info() {
    let info = CameraInfo {
        name: "Test Camera".to_string(),
        id: 42,
        max_width: 4096,
        max_height: 3072,
        pixel_size_um: 3.76,
        sensor_type: SensorType::Color,
        has_cooler: true,
        bit_depth: 14,
        min_exposure_us: 50,
        max_exposure_us: 7200_000_000,
        min_gain: 0,
        max_gain: 600,
        ..Default::default()
    };

    let response = CameraInfoResponse::from_info(&info, "test_42");

    assert_eq!(response.id, "test_42");
    assert_eq!(response.name, "Test Camera");
    assert_eq!(response.max_width, 4096);
    assert_eq!(response.max_height, 3072);
    assert_eq!(response.pixel_size_um, 3.76);
    assert_eq!(response.sensor_type, "Color");
    assert!(response.has_cooler);
    assert_eq!(response.bit_depth, 14);
    assert_eq!(response.min_exposure_us, 50);
    assert_eq!(response.max_exposure_us, 7200_000_000);
    assert_eq!(response.min_gain, 0);
    assert_eq!(response.max_gain, 600);
}

// ============================================================================
// Capture Status Response Tests
// ============================================================================

#[test]
fn test_capture_status_response_from_session() {
    let session = CaptureSession {
        state: CaptureState::Capturing,
        frame_count: 100,
        stacked_count: 95,
        rejected_count: 5,
        last_error: Some("Test error".to_string()),
        started_at: Some(1234567890),
        exposure_us: 2_000_000,
        gain: 150,
    };

    let response = CaptureStatusResponse::from(&session);

    assert_eq!(response.state, "Capturing");
    assert_eq!(response.frame_count, 100);
    assert_eq!(response.stacked_count, 95);
    assert_eq!(response.rejected_count, 5);
    assert_eq!(response.last_error, Some("Test error".to_string()));
    assert_eq!(response.started_at, Some(1234567890));
    assert_eq!(response.exposure_us, 2_000_000);
    assert_eq!(response.gain, 150);
}

// ============================================================================
// Settings Response Tests
// ============================================================================

#[test]
fn test_settings_response_from_settings() {
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
