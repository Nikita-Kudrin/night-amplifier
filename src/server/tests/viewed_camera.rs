//! `PUT /api/view/camera`, what `/ws/events` tells a client about it, and the eyepiece
//! snapshot following it.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

use super::helpers::create_test_state;
use super::image_stream_clients::{start_server, Client};
use crate::render::display::RenderReadyFrame;
use crate::session::events::ServerEvent;
use crate::session::state::{AppState, CameraRole, ConnectedCameraInfo};

fn install_guide(state: &AppState) {
    state.roster.install(
        ConnectedCameraInfo {
            id: "mock_guide".to_string(),
            provider: "Mock".to_string(),
            index: 1,
            role: CameraRole::Guide,
            info: crate::camera::CameraInfo {
                name: "Guide Mock".to_string(),
                ..Default::default()
            },
        },
        false,
    );
}

fn router(state: &Arc<AppState>) -> axum::Router {
    crate::server::api::create_router().with_state(Arc::clone(state))
}

async fn request(state: &Arc<AppState>, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = router(state).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, bytes.to_vec())
}

async fn put_view(state: &Arc<AppState>, body: Value) -> (StatusCode, Value) {
    let (status, bytes) = request(state, "PUT", "/view/camera", Some(body)).await;
    (status, serde_json::from_slice(&bytes).unwrap_or(json!({})))
}

fn viewed_events(rx: &mut tokio::sync::broadcast::Receiver<ServerEvent>) -> Vec<CameraRole> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|event| match event {
            ServerEvent::ViewedCameraChanged { camera } => Some(camera),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn selecting_the_guide_camera_answers_with_it_and_tells_every_client_once() {
    let state = create_test_state();
    install_guide(&state);
    let mut rx = state.subscribe_events();

    let (status, body) = put_view(&state, json!({"camera": "guide"})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["camera"], "guide");
    assert_eq!(state.viewed_camera.get(), CameraRole::Guide);

    let (status, _) = put_view(&state, json!({"camera": "guide"})).await;
    assert_eq!(status, StatusCode::OK, "selecting it again is not an error");
    assert_eq!(viewed_events(&mut rx), [CameraRole::Guide], "one change, one event");
}

#[tokio::test]
async fn the_guide_camera_is_refused_while_none_is_connected() {
    let state = create_test_state();
    let mut rx = state.subscribe_events();

    let (status, body) = put_view(&state, json!({"camera": "guide"})).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["success"], false);
    assert!(body["error"].as_str().unwrap().contains("No guide camera"), "{body}");
    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
    assert!(viewed_events(&mut rx).is_empty());
}

#[tokio::test]
async fn an_unknown_camera_is_a_malformed_request() {
    let state = create_test_state();
    install_guide(&state);

    for body in [json!({"camera": "finder"}), json!({}), json!({"camera": 1})] {
        let (status, _) = put_view(&state, body.clone()).await;
        assert!(status.is_client_error(), "{body}: {status}");
    }
    assert_eq!(state.viewed_camera.get(), CameraRole::Main);
}

fn ready_frame(width: usize, height: usize) -> Arc<RenderReadyFrame> {
    Arc::new(RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(crate::frame::Frame::filled(width, height, 3, 0.4).unwrap()),
        pipeline_config: crate::render::RenderPipelineConfig::default(),
        stretch_result: None,
    })
}

/// GET the snapshot, waiting out a 503 from another test holding the process-wide slot.
async fn snapshot(state: &Arc<AppState>) -> (StatusCode, Vec<u8>) {
    for _ in 0..100 {
        let answer = request(state, "GET", "/eyepiece/snapshot", None).await;
        if answer.0 != StatusCode::SERVICE_UNAVAILABLE {
            return answer;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the snapshot slot never came free");
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    let reader = ::png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
    (reader.info().width, reader.info().height)
}

/// A viewer's Download saves the picture they are looking at.
#[tokio::test]
async fn the_snapshot_follows_the_viewed_camera() {
    let state = create_test_state();
    install_guide(&state);
    state.main_stream.set_latest_raw_frame(ready_frame(37, 19));
    state.guide_stream.set_latest_raw_frame(ready_frame(24, 16));

    let (status, png) = snapshot(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(png_size(&png), (37, 19), "the imaging camera's frame");

    put_view(&state, json!({"camera": "guide"})).await;
    let (status, png) = snapshot(&state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(png_size(&png), (24, 16), "the guide camera's frame");
}

/// Not the imaging frame instead: that would hand the viewer a picture they never saw.
#[tokio::test]
async fn a_guide_camera_with_no_frame_yet_has_no_snapshot() {
    let state = create_test_state();
    install_guide(&state);
    state.main_stream.set_latest_raw_frame(ready_frame(37, 19));
    put_view(&state, json!({"camera": "guide"})).await;

    let (status, _) = snapshot(&state).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn next_event(client: &mut Client) -> Value {
    match tokio::time::timeout(Duration::from_secs(5), client.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str(&text).unwrap(),
        other => panic!("expected a JSON event, got {other:?}"),
    }
}

/// A reloaded `/` must show the toggle where the operator left it.
#[tokio::test]
async fn the_events_socket_opens_with_the_guide_camera_when_it_is_viewed() {
    let server = start_server().await;
    install_guide(&server.state);
    crate::session::services::CameraService::select_viewed_camera(&server.state, CameraRole::Guide).unwrap();

    let mut client = server.connect("/ws/events").await;
    let opening: Vec<Value> = [next_event(&mut client).await, next_event(&mut client).await, next_event(&mut client).await].into();

    assert_eq!(opening[2], json!({"type": "viewed_camera_changed", "camera": "guide"}));
}

/// A client that fell behind may have missed the toggle; it is told again.
///
/// Single-threaded on purpose: the burst below is sent without yielding, so the handler
/// cannot keep up and its receiver lags.
#[tokio::test(flavor = "current_thread")]
async fn a_lagged_events_client_is_told_the_viewed_camera_again() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect("/ws/events").await;
    for _ in 0..3 {
        next_event(&mut client).await;
    }

    crate::session::services::CameraService::select_viewed_camera(&server.state, CameraRole::Guide).unwrap();
    for i in 0..400 {
        server.state.send_error(format!("burst {i}"));
    }

    let mut seen = Vec::new();
    loop {
        let event = next_event(&mut client).await;
        let after_warning = seen.iter().any(|e: &Value| e["type"] == "warning");
        seen.push(event.clone());
        if after_warning && event["type"] == "viewed_camera_changed" {
            assert_eq!(event["camera"], "guide");
            return;
        }
        assert!(seen.len() < 1000, "no viewed camera after the lag warning: {:?}", &seen[..5]);
    }
}
