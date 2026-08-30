//! Tests for WebSocket events

use crate::server::events::ServerEvent;
use crate::server::state::CaptureState;

#[tokio::test]
async fn test_event_to_json_all_variants() {
    // StateChanged
    let event = ServerEvent::state_changed(CaptureState::Capturing);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "state_changed");
    assert_eq!(json["state"], "Capturing");

    // FrameCaptured
    let event = ServerEvent::frame_captured(42, 10, 2, None);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "frame_captured");
    assert_eq!(json["frame_number"], 42);
    assert_eq!(json["stacked_count"], 10);
    assert_eq!(json["rejected_count"], 2);

    // FrameRejected
    let event = ServerEvent::frame_rejected(5, 3, 2, "Bad alignment");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "frame_rejected");
    assert_eq!(json["frame_number"], 5);
    assert_eq!(json["stacked_count"], 3);
    assert_eq!(json["rejected_count"], 2);
    assert_eq!(json["reason"], "Bad alignment");

    // SettingsUpdated
    let event = ServerEvent::SettingsUpdated;
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "settings_updated");

    // CameraConnected
    let event = ServerEvent::camera_connected("Test Camera");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_connected");
    assert_eq!(json["name"], "Test Camera");

    // CameraDisconnected
    let event = ServerEvent::camera_disconnected("Test Camera");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_disconnected");
    assert_eq!(json["name"], "Test Camera");

    // Error
    let event = ServerEvent::error("Something went wrong");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "error");
    assert_eq!(json["message"], "Something went wrong");

    // CameraPersistentlyUnresponsive
    let event = ServerEvent::camera_persistently_unresponsive("Test Camera", 3);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_persistently_unresponsive");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["consecutive_timeouts"], 3);

    // The web client reads these field names directly. A mismatch here renders
    // as "Camera undefined has stopped responding" and no test catches it
    // unless both sides are pinned to the same shape.
    let event = ServerEvent::camera_reconnecting("Test Camera", 2, 5, 10);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_reconnecting");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["attempt"], 2);
    assert_eq!(json["of"], 5);
    assert_eq!(json["next_attempt_in_s"], 10);

    let event = ServerEvent::camera_reconnect_failed("Test Camera", 5, "still unreachable");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_reconnect_failed");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["attempts"], 5);
    assert_eq!(json["reason"], "still unreachable");

    let event = ServerEvent::capture_resumed("Test Camera", 514);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "capture_resumed");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["stacked_count"], 514);
}
