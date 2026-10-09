//! The eyepiece sockets follow the operator's Guide toggle on their open connection; the
//! operator's own `/ws/stream` keeps the camera its URL names.

use std::time::Duration;

use futures_util::SinkExt;
use serial_test::parallel;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::render::display::RenderReadyFrame;
use crate::session::camera::lifecycle::{finalize_disconnect, DisconnectCause};
use crate::session::services::CameraService;
use crate::session::state::{CameraRole, ConnectedCameraInfo, StreamKind};

/// Small and distinct, so a payload's header says which camera it came from.
const MAIN: (usize, usize) = (400, 300);
const GUIDE: (usize, usize) = (320, 200);
const GUIDE_NAME: &str = "Guide Mock";
const QUIET: Duration = Duration::from_millis(300);

fn size(dims: (usize, usize)) -> (u32, u32) {
    (dims.0 as u32, dims.1 as u32)
}

fn install_guide(state: &AppState) {
    state.roster.install(
        ConnectedCameraInfo {
            id: "mock_guide".to_string(),
            provider: "Mock".to_string(),
            index: 1,
            role: CameraRole::Guide,
            info: crate::camera::CameraInfo {
                name: GUIDE_NAME.to_string(),
                ..Default::default()
            },
        },
        false,
    );
}

/// Publish a guide frame the way the guide loop does: the latest frame, then one payload
/// per watched family under one counter.
fn publish_guide(state: &AppState, fill: f32) {
    let ready = Arc::new(RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(Frame::filled(GUIDE.0, GUIDE.1, 3, fill).unwrap()),
        pipeline_config: crate::render::RenderPipelineConfig {
            contrast: false,
            auto_stretch: false,
            saturation_boost: false,
            ..Default::default()
        },
        stretch_result: None,
    });
    let stream = &state.guide_stream;
    stream.set_latest_raw_frame(Arc::clone(&ready));
    let counter = stream.begin_frame();
    for kind in StreamKind::all() {
        if stream.viewer_count(kind) == 0 {
            continue;
        }
        let payload = match kind {
            StreamKind::Jpeg => crate::session::encoding::encode_rgb8_jpeg_bounded(&ready, 4096, 4096),
            StreamKind::Lossless => crate::session::encoding::encode_rgb8_lz4_chunked(&ready, 1, 4096, 4096),
        }
        .unwrap();
        stream.set_payload(kind, counter, payload);
    }
    stream.publish_frame();
}

fn counts(state: &AppState, kind: StreamKind) -> (usize, usize) {
    (state.main_stream.viewer_count(kind), state.guide_stream.viewer_count(kind))
}

async fn view(server: &TestServer, camera: &str) {
    let response = server.view(camera).await;
    assert_eq!(response.status, 200, "viewing {camera}: {}", response.body);
}

/// Every frame that arrives until the socket goes quiet, as their sizes.
async fn drain_sizes(client: &mut Client) -> Vec<(u32, u32)> {
    let mut sizes = Vec::new();
    while let Ok(payload) = tokio::time::timeout(QUIET, next_frame(client)).await {
        sizes.push(dimensions(&payload));
    }
    sizes
}

/// The core path, plus the viewer census the producers gate on: after the switch the
/// imaging render task must stop encoding LZ4 for nobody, and switching back restores it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn an_eyepiece_quality_client_follows_the_toggle_on_its_open_socket() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect_registered(LOSSLESS).await;
    server.render(MAIN, 0.25).await;
    assert_lossless(&next_frame(&mut client).await, size(MAIN), "imaging frame");
    publish_guide(&server.state, 0.5);

    view(&server, "guide").await;
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "guide frame on the same socket");
    eventually("the viewer to move to the guide stream", || {
        counts(&server.state, StreamKind::Lossless) == (0, 1)
    })
    .await;

    server.render(MAIN, 0.3).await;
    assert_no_frame(&mut client, QUIET, "imaging frame after the switch").await;
    publish_guide(&server.state, 0.6);
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "the next guide frame");

    // Back to the imaging camera: its last frame is shown at once, not after an exposure.
    view(&server, "main").await;
    assert_lossless(&next_frame(&mut client).await, size(MAIN), "the imaging camera's last frame");
    eventually("the viewer to move back", || counts(&server.state, StreamKind::Lossless) == (1, 0)).await;
    publish_guide(&server.state, 0.7);
    assert_no_frame(&mut client, QUIET, "guide frame after switching back").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn an_eyepiece_jpeg_client_follows_the_toggle_too() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect_registered(EYEPIECE).await;
    server.render(MAIN, 0.25).await;
    assert_jpeg(&next_frame(&mut client).await, size(MAIN), "imaging frame");
    publish_guide(&server.state, 0.5);

    view(&server, "guide").await;
    assert_jpeg(&next_frame(&mut client).await, size(GUIDE), "guide frame");
    eventually("the viewer to move", || counts(&server.state, StreamKind::Jpeg) == (0, 1)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn an_eyepiece_opened_while_the_guide_camera_is_viewed_starts_on_it() {
    let server = start_server().await;
    install_guide(&server.state);
    server.render(MAIN, 0.25).await;
    publish_guide(&server.state, 0.5);
    view(&server, "guide").await;

    let mut client = server.connect_registered(LOSSLESS).await;
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "first frame");
    assert_eq!(counts(&server.state, StreamKind::Lossless), (0, 1));
}

/// The viewers have no say: a `?source=` on an eyepiece URL changes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn an_eyepiece_url_cannot_override_the_operator() {
    let server = start_server().await;
    install_guide(&server.state);
    server.render(MAIN, 0.25).await;
    publish_guide(&server.state, 0.5);
    view(&server, "guide").await;

    let mut client = server.connect_registered(&format!("{EYEPIECE}?source=main")).await;
    assert_jpeg(&next_frame(&mut client).await, size(GUIDE), "first frame");
    assert_eq!(counts(&server.state, StreamKind::Jpeg), (0, 1));
}

/// The operator's own socket shows the camera its URL names; the toggle reaches it by
/// the page reconnecting, never by the server moving it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn the_live_view_socket_is_not_moved_by_the_toggle() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut live = server.connect_registered(LIVE_VIEW).await;
    server.render(MAIN, 0.25).await;
    assert_jpeg(&next_frame(&mut live).await, size(MAIN), "imaging frame");
    publish_guide(&server.state, 0.5);

    view(&server, "guide").await;
    assert_no_frame(&mut live, QUIET, "live view after the toggle").await;
    assert_eq!(counts(&server.state, StreamKind::Jpeg), (1, 0));
    server.render(MAIN, 0.3).await;
    assert_jpeg(&next_frame(&mut live).await, size(MAIN), "next imaging frame");
}

/// Losing the guide camera hands the viewers back to the imaging camera on the same socket.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_guide_disconnect_returns_the_eyepiece_to_the_imaging_camera() {
    let server = start_server().await;
    install_guide(&server.state);
    server.render(MAIN, 0.25).await;
    publish_guide(&server.state, 0.5);
    view(&server, "guide").await;
    let mut client = server.connect_registered(LOSSLESS).await;
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "guide frame");

    finalize_disconnect(&server.state, CameraRole::Guide, GUIDE_NAME, DisconnectCause::Requested).await;

    assert_eq!(server.state.viewed_camera.get(), CameraRole::Main);
    assert_lossless(&next_frame(&mut client).await, size(MAIN), "imaging frame after the disconnect");
    eventually("the viewer to move back", || counts(&server.state, StreamKind::Lossless) == (1, 0)).await;
}

/// A guide camera that has not rendered yet leaves the screen as it was, and the socket
/// open for the first guide frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn switching_to_a_guide_camera_with_no_frame_yet_waits_for_one() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect_registered(LOSSLESS).await;
    server.render(MAIN, 0.25).await;
    assert_lossless(&next_frame(&mut client).await, size(MAIN), "imaging frame");

    view(&server, "guide").await;
    assert_no_frame(&mut client, QUIET, "nothing to show yet").await;
    client.send(Message::text("ping")).await.unwrap();
    let pong = tokio::time::timeout(FRAME_TIMEOUT, futures_util::StreamExt::next(&mut client)).await;
    assert!(matches!(pong, Ok(Some(Ok(Message::Text(ref t)))) if t.as_str() == "pong"), "{pong:?}");

    eventually("the viewer to move", || counts(&server.state, StreamKind::Lossless) == (0, 1)).await;
    publish_guide(&server.state, 0.5);
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "first guide frame");
}

/// Toggling as fast as a finger can, with several viewers of both families: the census
/// ends exact and every viewer ends on the operator's final choice.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn rapid_toggling_leaves_every_viewer_on_the_final_camera() {
    let server = start_server().await;
    install_guide(&server.state);
    server.render(MAIN, 0.25).await;
    publish_guide(&server.state, 0.5);
    let mut clients = Vec::new();
    for path in [LOSSLESS, LOSSLESS, EYEPIECE, EYEPIECE] {
        clients.push(server.connect_registered(path).await);
    }

    for i in 1..=51 {
        let camera = if i % 2 == 1 { CameraRole::Guide } else { CameraRole::Main };
        CameraService::select_viewed_camera(&server.state, camera).unwrap();
        tokio::task::yield_now().await;
    }

    eventually("every viewer on the guide stream", || {
        counts(&server.state, StreamKind::Lossless) == (0, 2) && counts(&server.state, StreamKind::Jpeg) == (0, 2)
    })
    .await;
    publish_guide(&server.state, 0.6);
    for (i, client) in clients.iter_mut().enumerate() {
        let sizes = drain_sizes(client).await;
        assert_eq!(sizes.last(), Some(&size(GUIDE)), "client {i} ended on {sizes:?}");
    }
    server.render(MAIN, 0.3).await;
    for client in &mut clients {
        assert_no_frame(client, QUIET, "imaging frame after the last toggle").await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn viewers_leaving_mid_switch_leak_nothing() {
    let server = start_server().await;
    install_guide(&server.state);
    server.render(MAIN, 0.25).await;
    publish_guide(&server.state, 0.5);
    let mut clients = Vec::new();
    for path in [LOSSLESS, EYEPIECE, LOSSLESS] {
        clients.push(server.connect_registered(path).await);
    }

    let state = Arc::clone(&server.state);
    let toggler = tokio::spawn(async move {
        for i in 0..200 {
            let camera = if i % 2 == 0 { CameraRole::Guide } else { CameraRole::Main };
            CameraService::select_viewed_camera(&state, camera).unwrap();
            tokio::task::yield_now().await;
        }
    });
    for client in clients {
        tokio::task::yield_now().await;
        drop(client);
    }
    toggler.await.unwrap();

    eventually("both streams to lose every viewer", || {
        StreamKind::all().into_iter().all(|kind| counts(&server.state, kind) == (0, 0))
    })
    .await;
}

/// Back to an imaging camera that has never rendered — connected but not capturing, or
/// not connected at all. The eyepiece kept the guide camera's last picture on screen with
/// nothing to replace it, labelled as the imaging camera, and Download then answered 404
/// for the picture the viewer was looking at. `/` drops a frame that belongs to the
/// camera it stopped watching; the open eyepiece socket must be told to do the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn switching_to_a_camera_with_no_frame_tells_the_eyepiece_its_picture_is_gone() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect_registered(LOSSLESS).await;
    view(&server, "guide").await;
    publish_guide(&server.state, 0.5);
    assert_lossless(&next_frame(&mut client).await, size(GUIDE), "guide frame");

    view(&server, "main").await;
    let told = tokio::time::timeout(QUIET, client.next()).await;
    assert!(
        matches!(&told, Ok(Some(Ok(Message::Text(text)))) if text.as_str() == crate::server::ws::NO_FRAME),
        "the socket said nothing, so the guide picture stays up as the imaging camera's: {told:?}"
    );

    // The imaging camera's first frame still follows on the same socket.
    server.render(MAIN, 0.25).await;
    assert_lossless(&next_frame(&mut client).await, size(MAIN), "imaging frame after the marker");
}

/// The marker is for a switch only: a socket opened onto an empty stream has no picture to
/// drop, and a switch that has a frame to send replaces the picture by sending it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn no_frame_is_sent_only_when_a_switch_has_nothing_to_show() {
    let server = start_server().await;
    install_guide(&server.state);
    let mut client = server.connect_registered(EYEPIECE).await;
    assert_nothing_said(&mut client, "on connect").await;

    server.render(MAIN, 0.25).await;
    next_frame(&mut client).await;
    publish_guide(&server.state, 0.5);
    view(&server, "guide").await;
    let frame = next_frame_or_text(&mut client).await;
    assert!(matches!(frame, Message::Binary(_)), "the guide frame, not a marker: {frame:?}");
}

async fn next_frame_or_text(client: &mut Client) -> Message {
    tokio::time::timeout(FRAME_TIMEOUT, client.next())
        .await
        .expect("nothing arrived")
        .expect("stream ended")
        .unwrap()
}

async fn assert_nothing_said(client: &mut Client, context: &str) {
    if let Ok(message) = tokio::time::timeout(QUIET, client.next()).await {
        panic!("{context}: {message:?}");
    }
}
