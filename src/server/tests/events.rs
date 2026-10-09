//! Tests for WebSocket events

use crate::session::events::ServerEvent;
use crate::session::state::{CameraRole, CaptureState};

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
    let event = ServerEvent::camera_reconnecting("Test Camera", CameraRole::Guide, 2, 5, 10);
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_reconnecting");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["role"], "guide");
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

    let json: serde_json::Value =
        serde_json::from_str(&ServerEvent::FocusModeLeft.to_json()).unwrap();
    assert_eq!(json, serde_json::json!({ "type": "focus_mode_left" }));
}


/// A client that connects later — a reloaded page, a phone waking up — gets every camera's
/// phase and the viewed camera straight away. Phases otherwise arrive only as changes, and on 2026-09-20 a page
/// that had missed one offered "Start guide" for a loop that was running.
#[tokio::test]
async fn the_events_socket_opens_with_the_capture_state_phases_and_viewed_camera() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let server = super::image_stream_clients::start_server().await;
    let camera = crate::session::state::ConnectedCameraInfo {
        id: "mock_1".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Guide,
        info: crate::camera::CameraInfo {
            name: "Guiding".to_string(),
            ..Default::default()
        },
    };
    server.state.roster.install(camera, false);
    server
        .state
        .set_camera_phase(CameraRole::Guide, "Guiding", crate::session::state::CameraPhase::Guiding);

    let mut client = server.connect("/ws/events").await;
    let mut events = Vec::new();
    for _ in 0..3 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), client.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                events.push(serde_json::from_str::<serde_json::Value>(&text).unwrap())
            }
            other => panic!("expected a JSON event, got {other:?}"),
        }
    }

    let (state, phases, viewed) = (&events[0], &events[1], &events[2]);
    assert_eq!(state["type"], "state_changed");
    assert_eq!(state["state"], "Idle");
    assert_eq!(phases["type"], "camera_phases");
    assert_eq!(
        phases["cameras"],
        serde_json::json!([{"name": "Guiding", "role": "guide", "phase": "guiding"}])
    );
    assert_eq!(*viewed, serde_json::json!({"type": "viewed_camera_changed", "camera": "main"}));
}
