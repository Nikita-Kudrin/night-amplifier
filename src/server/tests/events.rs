//! Tests for WebSocket events

use serde_json::json;

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

    let event = ServerEvent::camera_reconnect_failed("Test Camera", CameraRole::Main, 5, "still unreachable");
    let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
    assert_eq!(json["type"], "camera_reconnect_failed");
    assert_eq!(json["name"], "Test Camera");
    assert_eq!(json["role"], "main");
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
async fn the_events_socket_opens_with_every_value_topic_then_synced() {
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
    let opening = super::image_stream_clients::events_until_synced(&mut client).await;

    assert_eq!(
        opening,
        [
            json!({"type": "state_changed", "state": "Idle"}),
            json!({"type": "capture_counts", "frame_count": 0, "stacked_count": 0, "rejected_count": 0,
                   "dropped_count": 0, "delivered_count": 0, "last_rejection_reason": null}),
            json!({"type": "camera_phases", "cameras": [{"name": "Guiding", "role": "guide", "phase": "guiding"}]}),
            json!({"type": "camera_recovery_cleared", "role": "main"}),
            json!({"type": "camera_recovery_cleared", "role": "guide"}),
            json!({"type": "viewed_camera_changed", "camera": "main"}),
            json!({"type": "stack_crop_changed"}),
            json!({"type": "disk_writer_warning_cleared"}),
            json!({"type": "target_cleared"}),
            json!({"type": "push_direction_cleared"}),
            json!({"type": "plate_solving_status", "in_progress": false, "target_name": null,
                   "stage": null, "last_result": null}),
            json!({"type": "push_to_blocked", "reason": null}),
            json!({"type": "astap_install_status", "state": "idle"}),
            json!({"type": "catalog_install_status", "state": "idle"}),
            json!({"type": "synced", "cause": "connect", "dropped": 0}),
        ]
    );
}

/// A client that fell behind gets the current state again, not the backlog it lagged on:
/// the stale events would only be overwritten by the snapshot, and feeding them to a slow
/// client lags it again. It is not warned either; the sync is the whole answer.
///
/// Single-threaded on purpose, as `a_lagged_events_client_is_told_the_viewed_camera_again`:
/// the burst is sent without yielding, so the handler's receiver lags.
#[tokio::test(flavor = "current_thread")]
async fn a_lagged_events_client_is_resynced_instead_of_fed_the_backlog() {
    use super::image_stream_clients::{events_until, events_until_synced};

    let server = super::image_stream_clients::start_server().await;
    let mut client = server.connect("/ws/events").await;
    let opening = events_until_synced(&mut client).await;

    for i in 0..400 {
        server.state.send_error(format!("burst {i}"));
    }
    let resync = events_until_synced(&mut client).await;
    server.state.send_error("after the lag".to_string());
    let after = events_until(&mut client, |event| event["message"] == "after the lag").await;

    let synced = resync.last().unwrap();
    assert_eq!(synced["cause"], "lagged");
    assert!(synced["dropped"].as_u64().unwrap() > 0, "{synced}");
    assert_eq!(resync[..resync.len() - 1], opening[..opening.len() - 1], "the snapshot again");
    let backlog = resync.iter().chain(&after).filter(|e| e["type"] == "error" && e["message"] != "after the lag");
    assert_eq!(backlog.count(), 0, "stale events reached the client");
    assert!(!resync.iter().chain(&after).any(|e| e["type"] == "warning"));
}

/// Another tab, or a reloaded one, sees an install already under way rather than an idle
/// installer it could start a second install from.
#[tokio::test]
async fn a_client_that_connects_mid_install_is_told_how_far_it_got() {
    use crate::push_to::PushToEvents;

    let server = super::image_stream_clients::start_server().await;
    let port = server.state.installs.begin(crate::session::state::Installer::Catalog).unwrap();
    port.emit(crate::push_to::PushToEvent::catalog_install_progress("NGC.csv", 300, Some(1200)));

    let mut client = server.connect("/ws/events").await;
    let opening = super::image_stream_clients::events_until_synced(&mut client).await;

    let status = opening.iter().find(|e| e["type"] == "catalog_install_status").unwrap();
    assert_eq!(
        *status,
        json!({"type": "catalog_install_status", "state": "running", "progress": {
            "type": "catalog_install_progress", "file_name": "NGC.csv",
            "bytes_downloaded": 300, "total_bytes": 1200, "percent": 25.0,
        }})
    );
}

/// The lost `astap_install_completed` once left the installer "installing" forever; the
/// resync after the lag says the install is over. Single-threaded, as the lag test above.
#[tokio::test(flavor = "current_thread")]
async fn a_client_that_lagged_through_the_end_of_an_install_is_told_it_completed() {
    use super::image_stream_clients::events_until_synced;
    use crate::push_to::{InstallStage, PushToEvent, PushToEvents};

    let server = super::image_stream_clients::start_server().await;
    let mut client = server.connect("/ws/events").await;
    events_until_synced(&mut client).await;

    let port = server.state.installs.begin(crate::session::state::Installer::Astap).unwrap();
    port.emit(PushToEvent::astap_install_completed("D80 Database", Some(&InstallStage::DatabaseCompleted)));
    drop(port);
    for i in 0..400 {
        server.state.send_error(format!("burst {i}"));
    }
    let resync = events_until_synced(&mut client).await;

    assert_eq!(resync.last().unwrap()["cause"], "lagged");
    assert!(!resync.iter().any(|e| e["type"] == "astap_install_completed"), "{resync:?}");
    assert!(resync.contains(&json!({"type": "astap_install_status", "state": "completed"})), "{resync:?}");
}

/// What a lagged client missed of Push-To comes back in the resync: a lost
/// `position_solved` left "Searching…" up, and a lost `push_to_blocked` was never resent.
#[tokio::test(flavor = "current_thread")]
async fn a_lagged_events_client_recovers_the_push_to_state_it_missed() {
    use super::image_stream_clients::events_until_synced;
    use crate::push_to::PushToBlocker;
    use crate::session::events::PushToTarget;

    let server = super::image_stream_clients::start_server().await;
    let mut client = server.connect("/ws/events").await;
    events_until_synced(&mut client).await;

    let announcer = &server.state.push_to_announcer;
    announcer.target_changed(PushToTarget {
        name: Some("Andromeda".into()),
        designation: Some("M31".into()),
        ra_degrees: 10.68,
        dec_degrees: 41.27,
    });
    announcer.announce(ServerEvent::PlateSolvingStarted { target_name: Some("M31".into()) });
    announcer.announce(ServerEvent::position_solved(10.7, 41.3, "00:42:44", "+41:16:09", None, 0.9, 0.0));
    announcer.announce_blocker(Some(PushToBlocker::TelescopeMoving));
    for i in 0..400 {
        server.state.send_error(format!("burst {i}"));
    }
    let resync = events_until_synced(&mut client).await;

    assert_eq!(resync.last().unwrap()["cause"], "lagged");
    let restated = |kind: &str| resync.iter().find(|e| e["type"] == kind).cloned();
    assert_eq!(
        restated("target_changed"),
        Some(json!({"type": "target_changed", "name": "Andromeda", "designation": "M31",
                    "ra_degrees": 10.68, "dec_degrees": 41.27}))
    );
    assert_eq!(
        restated("plate_solving_status"),
        Some(json!({"type": "plate_solving_status", "in_progress": false, "target_name": "M31",
                    "stage": null, "last_result": "success"}))
    );
    assert_eq!(
        restated("push_to_blocked"),
        Some(json!({"type": "push_to_blocked", "reason": "Telescope is moving"}))
    );
}

/// A clear lost to a lag once left "Slow disk" and "Reconnecting" up for the night; the
/// resync restates both as over, and a client that lagged mid-episode is told it is still on.
/// Single-threaded, as the lag test above.
#[tokio::test(flavor = "current_thread")]
async fn a_client_that_lagged_through_a_clear_is_told_the_warnings_are_over() {
    use super::image_stream_clients::events_until_synced;
    use crate::session::state::RecoveryStatus;

    let server = super::image_stream_clients::start_server().await;
    let mut client = server.connect("/ws/events").await;
    events_until_synced(&mut client).await;
    let lag = |state: &crate::session::state::AppState| {
        for i in 0..400 {
            state.send_error(format!("burst {i}"));
        }
    };

    server.state.disk_warning.raise(40);
    let reconnecting = RecoveryStatus::Reconnecting { name: "Ares".into(), attempt: 3, of: 9, next_attempt_in_s: 5 };
    server.state.recovery_status.set(CameraRole::Main, reconnecting);
    lag(&server.state);
    let during = events_until_synced(&mut client).await;
    assert!(during.contains(&json!({"type": "disk_writer_warning", "queue_depth": 40})), "{during:?}");
    assert!(
        during.contains(&json!({"type": "camera_reconnecting", "name": "Ares", "role": "main",
                                "attempt": 3, "of": 9, "next_attempt_in_s": 5})),
        "{during:?}"
    );

    server.state.disk_warning.clear();
    server.state.recovery_status.clear(CameraRole::Main);
    lag(&server.state);
    let after = events_until_synced(&mut client).await;
    assert_eq!(after.last().unwrap()["cause"], "lagged");
    assert!(after.contains(&json!({"type": "disk_writer_warning_cleared"})), "{after:?}");
    assert!(after.contains(&json!({"type": "camera_recovery_cleared", "role": "main"})), "{after:?}");
    assert!(!after.iter().any(|e| e["type"] == "disk_writer_warning" || e["type"] == "camera_reconnecting"));
}
