//! `auto_crop` at the sockets: a stack the stacking task cut reaches `/ws/stream`, both
//! eyepiece sockets and the snapshot route cut alike, and `/ws/events` hears the cut. With
//! the setting off, every one of them carries the whole stack.

use serial_test::parallel;

use super::viewed_camera::{install_guide, publish_guide, view, GUIDE};
use super::*;
use crate::stacking::CropRect;

/// Small enough that no family's resolution scales it, so a header's size is the crop's.
const STACK: (usize, usize) = (400, 300);
const CROP: CropRect = CropRect { x: 40, y: 32, width: 320, height: 240 };

fn auto_crop(server: &TestServer, on: bool) {
    server.state.settings.update(|settings| settings.auto_crop = on);
}

/// The snapshot route's PNG size, through the real handler, waiting out a 503 from another
/// test holding the process-wide slot.
async fn snapshot_size(server: &TestServer) -> (u32, u32) {
    use tower::ServiceExt;
    let router = axum::Router::new()
        .route("/snapshot", axum::routing::get(crate::server::api::eyepiece::get_snapshot))
        .with_state(Arc::clone(&server.state));
    for _ in 0..100 {
        let request = axum::http::Request::get("/snapshot").body(axum::body::Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        if response.status() == axum::http::StatusCode::SERVICE_UNAVAILABLE {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        }
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let reader = ::png::Decoder::new(std::io::Cursor::new(bytes.to_vec())).read_info().unwrap();
        return (reader.info().width, reader.info().height);
    }
    panic!("the snapshot slot never came free");
}

async fn sizes_on_every_socket(server: &TestServer, crop: Option<CropRect>) -> Vec<(u32, u32)> {
    let mut clients = vec![
        server.connect_registered(LIVE_VIEW).await,
        server.connect_registered(EYEPIECE).await,
        server.connect_registered(LOSSLESS).await,
    ];
    server.render_stack(STACK, 0.25, crop).await;
    let mut sizes = Vec::new();
    for client in &mut clients {
        sizes.push(dimensions(&next_frame(client).await));
    }
    sizes.push(snapshot_size(server).await);
    sizes
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn every_socket_streams_the_cropped_stack() {
    let server = start_server().await;
    auto_crop(&server, true);

    let sizes = sizes_on_every_socket(&server, Some(CROP)).await;

    assert_eq!(sizes, vec![(320, 240); 4], "live view, eyepiece, eyepiece quality, snapshot");
}

/// The stacking task sends no crop while the setting is off; and a crop that arrives under
/// a frame whose settings have it off is ignored all the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn with_the_setting_off_every_socket_streams_the_whole_stack() {
    for crop in [None, Some(CROP)] {
        let server = start_server().await;
        auto_crop(&server, false);
        assert_eq!(sizes_on_every_socket(&server, crop).await, vec![(400, 300); 4], "sent {crop:?}");
    }
}

/// The guide camera keeps no stack, so the eyepiece showing it shows its frames whole while
/// the main stack is cut.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_viewed_guide_camera_is_never_cropped() {
    let server = start_server().await;
    auto_crop(&server, true);
    install_guide(&server.state);
    publish_guide(&server.state, 0.4);
    let mut eyepiece = server.connect_registered(EYEPIECE).await;
    server.render_stack(STACK, 0.25, Some(CROP)).await;
    assert_eq!(dimensions(&next_frame(&mut eyepiece).await), (320, 240));

    view(&server, "guide").await;
    let switched = next_frame(&mut eyepiece).await;
    assert_eq!(dimensions(&switched), (GUIDE.0 as u32, GUIDE.1 as u32));
}

async fn next_event(client: &mut Client) -> serde_json::Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), client.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).unwrap(),
            Ok(Some(Ok(_))) => continue,
            other => panic!("expected a JSON event, got {other:?}"),
        }
    }
}

async fn next_crop_event(client: &mut Client) -> serde_json::Value {
    loop {
        let event = next_event(client).await;
        if event["type"] == "stack_crop_changed" {
            return event;
        }
    }
}

/// A page that connects mid-session is told the cut its frames carry, and hears each change
/// — the live view shifts its pan by the origin's move instead of refitting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn the_events_socket_carries_the_crop() {
    let server = start_server().await;
    auto_crop(&server, true);

    let mut early = server.connect("/ws/events").await;
    assert_eq!(next_crop_event(&mut early).await, serde_json::json!({"type": "stack_crop_changed"}));

    server.render_stack(STACK, 0.25, Some(CROP)).await;
    let crop = serde_json::json!({
        "type": "stack_crop_changed",
        "crop": {"x": 40, "y": 32, "width": 320, "height": 240, "source_width": 400, "source_height": 300},
        "frame": {"width": 320, "height": 240},
    });
    assert_eq!(next_crop_event(&mut early).await, crop);

    let mut late = server.connect("/ws/events").await;
    assert_eq!(next_crop_event(&mut late).await, crop, "a late page was not told the crop");

    server.render_stack(STACK, 0.25, None).await;
    let whole = serde_json::json!({"type": "stack_crop_changed", "frame": {"width": 400, "height": 300}});
    assert_eq!(next_crop_event(&mut early).await, whole);
}

/// The live view waits for the frame a change names, so the size it is told must be the size
/// `/ws/stream` delivers — here one the streaming box scales down.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_crop_change_names_the_live_frame_it_arrives_with() {
    let server = start_server().await;
    auto_crop(&server, true);
    server.state.settings.update(|settings| {
        settings.streaming_resolution = crate::session::state::Resolution::Hd1080;
    });
    let mut events = server.connect("/ws/events").await;
    next_crop_event(&mut events).await;
    let mut live = server.connect_registered(LIVE_VIEW).await;

    server.render_stack((2400, 1600), 0.25, Some(CropRect { x: 200, y: 0, width: 2000, height: 1600 })).await;

    let (width, height) = dimensions(&next_frame(&mut live).await);
    assert!(height <= 1080, "the box did not scale the frame: {width}x{height}");
    let event = next_crop_event(&mut events).await;
    assert_eq!(event["frame"], serde_json::json!({"width": width, "height": height}));
}

/// A page that fell behind may have missed a cut, and would shift its pan by the wrong
/// origin: it is told the current one again.
///
/// Single-threaded on purpose, as `a_lagged_events_client_is_told_the_viewed_camera_again`:
/// the burst is sent without yielding, so the handler's receiver lags.
#[tokio::test(flavor = "current_thread")]
async fn a_lagged_events_client_is_told_the_crop_again() {
    let server = start_server().await;
    let mut client = server.connect("/ws/events").await;
    events_until_synced(&mut client).await;

    let crop = crate::session::events::StreamCrop {
        x: 40,
        y: 32,
        width: 320,
        height: 240,
        source_width: 400,
        source_height: 300,
    };
    server.state.stream_crop.publish(Some(crop), crate::session::events::StreamFrameSize { width: 320, height: 240 });
    for i in 0..400 {
        server.state.send_error(format!("burst {i}"));
    }

    let resync = events_until_synced(&mut client).await;
    assert_eq!(resync.last().unwrap()["cause"], "lagged");
    let crops: Vec<_> = resync.iter().filter(|e| e["type"] == "stack_crop_changed").collect();
    assert_eq!(crops.len(), 1, "{resync:?}");
    assert_eq!(crops[0]["crop"]["x"], 40);
}
