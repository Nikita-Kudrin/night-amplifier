//! Tests for camera endpoints

use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;

use super::helpers::*;
use crate::camera::{CameraInfo, SensorType};
use crate::session::events::ServerEvent;
use crate::session::state::*;

// ============================================================================
// Camera List Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_list_cameras_empty() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"].is_array());
}

#[tokio::test]
async fn test_list_cameras_with_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    let cameras = json["data"].as_array().unwrap();

    // Find our mock camera
    let mock_camera = cameras
        .iter()
        .find(|c| c["id"] == "mock_0")
        .expect("Mock camera should be in list");

    assert_eq!(mock_camera["name"], "Test Camera");
    assert_eq!(mock_camera["connected"], true);
    assert_eq!(mock_camera["info"]["max_width"], 1920);
    assert_eq!(mock_camera["info"]["max_height"], 1080);
}

/// A sync re-reads the list without enumerating: the connected cameras are there all the
/// same (whether it enumerates is `list_known_cameras`' own tests').
#[tokio::test]
async fn a_list_without_discovery_still_lists_the_connected_cameras() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras?discover=false").await;

    assert_eq!(status, StatusCode::OK);
    let connected: Vec<_> = json["data"].as_array().unwrap().iter().filter(|c| c["connected"] == true).collect();
    assert_eq!(connected.len(), 1);
    assert_eq!(connected[0]["id"], "mock_0");
    let (status, _) = get_json(&app, "/api/cameras?discover=maybe").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_multiple_cameras_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "camera_0").await;

    // Add a second camera manually, as the guide camera: a role holds one.
    let guide = ConnectedCameraInfo {
        id: "camera_1".to_string(),
        provider: "Mock".to_string(),
        index: 1,
        role: CameraRole::Guide,
        info: CameraInfo {
            name: "Second Camera".to_string(),
            sensor_type: SensorType::Mono,
            ..Default::default()
        },
    };
    state.roster.install(guide, false);

    let app = create_test_router(Arc::clone(&state));

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    let cameras = json["data"].as_array().unwrap();
    assert!(cameras.len() >= 2);

    // Check both cameras are present
    let ids: Vec<&str> = cameras.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"camera_0"));
    assert!(ids.contains(&"camera_1"));
}

// ============================================================================
// Camera Info Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_camera_info_success() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras/mock_0").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["id"], "mock_0");
    assert_eq!(json["data"]["name"], "Test Camera");
    assert_eq!(json["data"]["max_width"], 1920);
    assert_eq!(json["data"]["has_cooler"], true);
    assert_eq!(json["data"]["bit_depth"], 12);
}

#[tokio::test]
async fn test_get_camera_info_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/cameras/nonexistent").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("not found"));
}

// ============================================================================
// Camera Connect Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_connect_camera_invalid_id_format() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/invalidformat/connect", json!({})).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("Invalid camera ID"));
}

#[tokio::test]
async fn test_connect_camera_invalid_index() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) =
        post_json(&app, "/api/cameras/provider_notanumber/connect", json!({})).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["success"], false);
    assert!(json["error"]
        .as_str()
        .unwrap()
        .contains("Invalid camera index"));
}

#[tokio::test]
async fn test_connect_camera_already_connected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/mock_0/connect", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("already connected"));
}

#[tokio::test]
async fn test_connect_camera_provider_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/unknownprovider_0/connect", json!({})).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json["success"], false);
}

// ============================================================================
// Camera Disconnect Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_disconnect_camera_success() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);
    assert!(json["data"]["message"]
        .as_str()
        .unwrap()
        .contains("disconnected"));

    // Verify camera was removed
    assert!(!state.roster.contains("mock_0"));
}

#[tokio::test]
async fn test_disconnect_camera_not_connected() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = post_json(&app, "/api/cameras/nonexistent/disconnect", json!({})).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("not connected"));
}

/// Disconnect is a request the observer must be able to make at any moment: a capture
/// running on the camera is stopped first, the way Stop would, rather than refused.
#[tokio::test]
async fn disconnecting_a_capturing_camera_stops_the_capture_first() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    state.set_capture_state(CaptureState::Capturing);
    // Stands in for the capture pipeline: it winds down once asked to stop.
    let pipeline = {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            while state.capture_state() != CaptureState::Stopping {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            state.end_capture_state();
        })
    };
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;
    pipeline.await.unwrap();

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data"]["warming_up"], false);
    assert_eq!(state.capture_state(), CaptureState::Idle);
    assert!(!state.roster.contains("mock_0"));
}

#[tokio::test]
async fn test_disconnect_camera_clears_selected() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let app = create_test_router(Arc::clone(&state));

    // Verify it's selected
    assert_eq!(state.roster.selected().as_deref(), Some("mock_0"));

    post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    // Verify selected was cleared
    assert!(state.roster.selected().is_none());
}

#[tokio::test]
async fn test_disconnect_camera_broadcasts_event() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    let mut events_rx = state.subscribe_events();
    let app = create_test_router(Arc::clone(&state));

    post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;

    // Disconnect may emit CameraPhaseChanged before CameraDisconnected; loop
    // briefly until the disconnect event arrives.
    let saw_disconnect = tokio::time::timeout(tokio::time::Duration::from_millis(500), async {
        loop {
            match events_rx.recv().await {
                Ok(ServerEvent::CameraDisconnected { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(saw_disconnect, "Expected CameraDisconnected event");
}

// ============================================================================
// Connected Camera Info Tests
// ============================================================================

#[test]
fn test_connected_camera_info_clone() {
    let info = ConnectedCameraInfo {
        id: "test_0".to_string(),
        provider: "Test".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: CameraInfo {
            name: "Test Camera".to_string(),
            sensor_type: SensorType::Color,
            ..Default::default()
        },
    };

    let cloned = info.clone();

    assert_eq!(cloned.id, "test_0");
    assert_eq!(cloned.provider, "Test");
    assert_eq!(cloned.index, 0);
    assert_eq!(cloned.info.name, "Test Camera");
}


/// Start a warm-up on the mock camera without a monitor, the state a Disconnect leaves it in.
async fn warming_up(state: &Arc<AppState>) {
    state.set_camera_phase(CameraRole::Main, "Test Camera", CameraPhase::WarmingUp);
    state
        .slot(CameraRole::Main)
        .begin_warmup(std::time::Instant::now() + std::time::Duration::from_secs(120));
}

/// The list carries each connected camera's phase, so a page opened mid-session knows a
/// camera is warming up without having seen the event that said so.
#[tokio::test]
async fn the_camera_list_reports_phase_and_warmup_time() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    warming_up(&state).await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = get_json(&app, "/api/cameras").await;

    assert_eq!(status, StatusCode::OK);
    let entry = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|camera| camera["id"] == "mock_0")
        .expect("the connected camera is listed");
    assert_eq!(entry["phase"], "warming_up");
    let remaining = entry["warmup_remaining_s"].as_u64().expect("time left is reported");
    assert!((110..=120).contains(&remaining), "{remaining}");
}

/// A second ordinary Disconnect reports the warm-up rather than cutting it short;
/// `skip_warmup` ends it now.
#[tokio::test]
async fn disconnect_during_a_warmup_reports_it_unless_told_to_skip_it() {
    let state = create_test_state();
    add_mock_camera(&state, "mock_0").await;
    warming_up(&state).await;
    let app = create_test_router(Arc::clone(&state));

    let (status, json) = post_json(&app, "/api/cameras/mock_0/disconnect", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["warming_up"], true);
    assert!(json["data"]["warmup_remaining_s"].as_u64().is_some());
    assert!(state.roster.contains("mock_0"));

    let (status, json) = post_json(
        &app,
        "/api/cameras/mock_0/disconnect",
        json!({"skip_warmup": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data"]["warming_up"], false);
    assert!(!state.roster.contains("mock_0"));
    assert_eq!(state.camera_phase(CameraRole::Main), CameraPhase::Disconnected);
}

/// An INDI server that answers every client's `getProperties` with one camera, `name`.
#[cfg(feature = "indi")]
async fn serve_indi(name: &'static str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut received = String::new();
                let mut chunk = [0u8; 1024];
                while let Ok(n @ 1..) = socket.read(&mut chunk).await {
                    received.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    if received.contains("getProperties") {
                        let definitions = format!(
                            "<defNumberVector device='{name}' name='CCD_EXPOSURE' state='Idle'>\n\
                             <defNumber name='CCD_EXPOSURE_VALUE' min='0' max='3600' step='1'>1</defNumber>\n\
                             </defNumberVector>\n"
                        );
                        let _ = socket.write_all(definitions.as_bytes()).await;
                        received.clear();
                    }
                }
            });
        }
    });
    port
}

/// INDI cameras arrive by `camera_discovered`, after the list. Every client lists the
/// cameras again after a sync and on every connect, so a list without them dropped them.
#[cfg(feature = "indi")]
#[tokio::test(flavor = "multi_thread")]
async fn a_relisted_camera_list_keeps_the_indi_cameras_it_announced() {
    let state = create_test_state();
    let port = serve_indi("Fake CCD").await;
    state.settings.update(|settings| {
        settings.indi_server_host = "127.0.0.1".to_string();
        settings.indi_server_port = port;
    });
    let mut events = state.subscribe_events();
    let app = create_test_router(Arc::clone(&state));

    let (_, first) = get_json(&app, "/api/cameras").await;
    assert!(!first["data"].as_array().unwrap().iter().any(|c| c["provider"] == "indi"), "INDI answers later");
    let discovered = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(ServerEvent::CameraDiscovered { camera }) = events.recv().await {
                return camera;
            }
        }
    })
    .await
    .expect("the INDI camera was never announced");
    assert_eq!(discovered.name, "Fake CCD");

    let (_, relisted) = get_json(&app, "/api/cameras").await;
    let indi: Vec<_> = relisted["data"].as_array().unwrap().iter().filter(|c| c["provider"] == "indi").collect();
    assert_eq!(indi.len(), 1, "{relisted}");
    assert_eq!((&indi[0]["id"], &indi[0]["name"], &indi[0]["connected"]), (&json!(discovered.id), &json!("Fake CCD"), &json!(false)));
}

async fn delete_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let request = axum::http::Request::builder().method("DELETE").uri(uri).body(axum::body::Body::empty()).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(json!({})))
}

/// A give-up is restated on every sync until a camera connects into the role, so the
/// observer's dismissal goes to the server and ends it for every client at once.
#[tokio::test]
async fn dismissing_a_recovery_give_up_clears_it_for_every_client() {
    let state = create_test_state();
    let failed = RecoveryStatus::Failed { name: "Ares".into(), attempts: 9, reason: "unplugged".into() };
    state.recovery_status.set(CameraRole::Guide, failed);
    let mut events = state.subscribe_events();
    let app = crate::server::api::create_router().with_state(Arc::clone(&state));

    let (status, body) = delete_json(&app, "/cameras/recovery/guide").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["message"], "Recovery warning dismissed.");
    assert_eq!(state.recovery_status.get(CameraRole::Guide), RecoveryStatus::None);
    assert!(matches!(events.try_recv(), Ok(ServerEvent::CameraRecoveryCleared { role: CameraRole::Guide })));

    let (status, body) = delete_json(&app, "/cameras/recovery/guide").await;
    assert_eq!((status, &body["data"]["message"]), (StatusCode::OK, &json!("No recovery warning to dismiss.")));
    assert!(events.try_recv().is_err(), "nothing more to announce");
}

#[tokio::test]
async fn dismissing_names_a_role() {
    let app = crate::server::api::create_router().with_state(create_test_state());

    let (status, _) = delete_json(&app, "/cameras/recovery/finder").await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// What a stream holds after a render: `frame` as the encoders were handed it.
fn rendered(frame: crate::Frame, tone: crate::render::ToneProfile) -> Arc<crate::render::display::RenderReadyFrame> {
    rendered_through(frame, tone, crate::render::planetary::DaylightBalance::IDENTITY)
}

/// [`rendered`], `frame` having gone through `daylight_balance`.
fn rendered_through(
    frame: crate::Frame,
    tone: crate::render::ToneProfile,
    daylight_balance: crate::render::planetary::DaylightBalance,
) -> Arc<crate::render::display::RenderReadyFrame> {
    Arc::new(crate::render::display::RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(frame),
        pipeline_config: crate::render::RenderPipelineConfig { tone, daylight_balance, ..Default::default() },
        stretch_result: None,
    })
}

/// A 0.3-0.7 disc of `radius` px, each channel times `tint`, on a black sky.
fn tinted_disc(radius: f32, tint: [f32; 3], channels: usize) -> crate::Frame {
    let (w, h) = (300, 200);
    let mut frame = crate::Frame::zeros(w, h, channels).unwrap();
    for y in 0..h {
        for x in 0..w {
            let r = (x as f32 - 150.0).hypot(y as f32 - 100.0) / radius;
            let level = if r < 1.0 { 0.3 + 0.4 * (1.0 - r * r).sqrt() } else { 0.0 };
            for (c, t) in tint.iter().take(channels).enumerate() {
                frame.set_pixel(x, y, c, level * t);
            }
        }
    }
    frame
}

const RAW_IMX533: [f32; 3] = [0.943, 1.0, 0.484];

/// Calibrate reads the Moon the camera's stream last rendered and saves the balance to that
/// camera; every client hears the settings change.
#[tokio::test]
async fn calibrating_on_a_rendered_moon_saves_the_balance_that_greys_it() {
    use crate::render::ToneProfile;
    let state = create_test_state();
    state.stream(CameraRole::Main).set_latest_raw_frame(rendered(tinted_disc(80.0, RAW_IMX533, 3), ToneProfile::Planetary));
    let mut events = state.subscribe_events();
    let app = crate::server::api::create_router().with_state(Arc::clone(&state));

    let (status, body) = post_json(&app, "/cameras/daylight-balance/main/calibrate", json!({})).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let blue = body["data"]["blue"].as_f64().unwrap();
    assert!((blue - 1.0 / 0.484).abs() < 0.03, "{body}");
    let saved = state.settings.snapshot().daylight_balance;
    assert!((f64::from(saved.blue) - blue).abs() < 1e-6, "{saved:?}");
    assert!(matches!(events.try_recv(), Ok(ServerEvent::SettingsUpdated)));
}

/// After Stop nothing renders again, so the stream holds the frame the old balance made: a
/// second press on it must save the same balance, not the correction applied twice.
#[tokio::test]
async fn calibrating_twice_on_the_same_frame_saves_one_balance() {
    use crate::render::ToneProfile;
    let state = create_test_state();
    state.stream(CameraRole::Main).set_latest_raw_frame(rendered(tinted_disc(80.0, RAW_IMX533, 3), ToneProfile::Planetary));
    let app = crate::server::api::create_router().with_state(Arc::clone(&state));
    let calibrate = || post_json(&app, "/cameras/daylight-balance/main/calibrate", json!({}));

    let (_, first) = calibrate().await;
    let (status, second) = calibrate().await;

    assert_eq!(status, StatusCode::OK, "{second}");
    let blue = |body: &serde_json::Value| body["data"]["blue"].as_f64().unwrap();
    assert!((blue(&second) - blue(&first)).abs() < 0.01, "first {first}, second {second}");
}

/// Reset, then Calibrate on the frame held from before it: that frame went through the old
/// balance, so calibrating it gives that balance back rather than none at all.
#[tokio::test]
async fn calibrating_after_a_reset_on_a_frame_rendered_before_it_saves_the_balance_back() {
    use crate::render::planetary::DaylightBalance;
    use crate::render::ToneProfile;
    let state = create_test_state();
    let moon = DaylightBalance { red: 1.0 / 0.943, green: 1.0, blue: 1.0 / 0.484 }.sanitized();
    let mut balanced = tinted_disc(80.0, RAW_IMX533, 3);
    moon.apply(&mut balanced);
    state.stream(CameraRole::Main).set_latest_raw_frame(rendered_through(balanced, ToneProfile::Planetary, moon));
    assert!(state.settings.snapshot().daylight_balance.is_identity(), "reset: nothing saved");
    let app = crate::server::api::create_router().with_state(Arc::clone(&state));

    let (status, body) = post_json(&app, "/cameras/daylight-balance/main/calibrate", json!({})).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let saved = state.settings.snapshot().daylight_balance;
    assert!((saved.red - moon.red).abs() < 0.01 && (saved.blue - moon.blue).abs() < 0.02, "{saved:?} for {moon:?}");
}

/// Each refusal names what to change, at its own status.
#[tokio::test]
async fn calibration_refuses_what_it_cannot_measure() {
    use crate::render::ToneProfile;
    let state = create_test_state();
    let app = crate::server::api::create_router().with_state(Arc::clone(&state));
    let calibrate = || post_json(&app, "/cameras/daylight-balance/main/calibrate", json!({}));

    assert_eq!(calibrate().await.0, StatusCode::CONFLICT, "nothing rendered yet");
    let stream = state.stream(CameraRole::Main);
    for (frame, tone, refused) in [
        (tinted_disc(80.0, RAW_IMX533, 3), ToneProfile::DeepSky, StatusCode::CONFLICT),
        (tinted_disc(80.0, RAW_IMX533, 1), ToneProfile::Planetary, StatusCode::BAD_REQUEST),
        (tinted_disc(8.0, RAW_IMX533, 3), ToneProfile::Planetary, StatusCode::UNPROCESSABLE_ENTITY),
        (tinted_disc(80.0, [1.0, 0.2, 0.15], 3), ToneProfile::Planetary, StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        stream.set_latest_raw_frame(rendered(frame, tone));
        let (status, body) = calibrate().await;
        assert_eq!(status, refused, "{tone:?}: {body}");
        assert_eq!(body["success"], false);
    }
    let error = calibrate().await.1["error"].as_str().unwrap_or_default().to_owned();
    assert!(error.contains("too coloured"), "an H-alpha disc is named as coloured, not missing: {error}");
    assert!(state.settings.snapshot().daylight_balance.is_identity(), "a refusal saves nothing");
    assert_eq!(post_json(&app, "/cameras/daylight-balance/finder/calibrate", json!({})).await.0, StatusCode::BAD_REQUEST);
}
