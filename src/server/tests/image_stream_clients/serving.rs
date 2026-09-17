//! Many clients, one payload per family.

use std::time::Duration;

use futures_util::SinkExt;
use serial_test::parallel;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::server::state::{EyepieceStreamResolution, Resolution, StreamKind};

/// Clients of `/` and `/eyepiece` all receive the very same bytes at Streaming Resolution,
/// whatever viewport an old frontend still reports.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn every_jpeg_client_receives_identical_bytes_at_streaming_resolution() {
    let server = start_server().await;
    let mut clients = Vec::new();
    for (path, viewport) in [
        (LIVE_VIEW, Some((1920, 1080))),
        (LIVE_VIEW, Some((3840, 2160))),
        (LIVE_VIEW, None),
        (EYEPIECE, Some((1280, 720))),
        (EYEPIECE, Some((2880, 1440))),
        ("/ws/eyepiece?source=main", None),
    ] {
        let mut client = server.connect_registered(path).await;
        if let Some((w, h)) = viewport {
            send_viewport(&mut client, w, h).await;
        }
        clients.push(client);
    }

    server.render(IMX533, 0.25).await;

    let first = next_frame(&mut clients[0]).await;
    assert_jpeg(&first, (1440, 1440), "default Streaming Resolution");
    for (i, client) in clients.iter_mut().enumerate().skip(1) {
        assert_eq!(next_frame(client).await, first, "client {i} got different bytes");
    }
}

/// `/eyepiece_quality` has its own payload at its own setting; neither family's setting
/// leaks into the other.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn lossless_clients_share_their_own_payload_independent_of_jpeg() {
    let server = start_server().await;
    {
        let mut settings = server.state.settings.write().await;
        settings.streaming_resolution = Resolution::Hd1080;
        settings.eyepiece.stream_resolution = EyepieceStreamResolution::Uhd2160;
    }
    let mut eyepieces = vec![
        server.connect_registered(LOSSLESS).await,
        server.connect_registered(LOSSLESS).await,
    ];
    let mut phone = server.connect_registered(LIVE_VIEW).await;
    send_viewport(&mut eyepieces[1], 1920, 1080).await;

    server.render(IMX533, 0.5).await;

    let lossless = next_frame(&mut eyepieces[0]).await;
    assert_lossless(&lossless, (2160, 2160), "Eyepiece Streaming Resolution");
    assert_eq!(next_frame(&mut eyepieces[1]).await, lossless);
    assert_jpeg(&next_frame(&mut phone).await, (1080, 1080), "Streaming Resolution");
}

/// A dozen clients across all three endpoints, joining at once, over several frames:
/// every client receives every frame, and every frame is identical within a family.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn many_concurrent_clients_receive_every_frame_identically() {
    let server = start_server().await;
    let paths = [LIVE_VIEW, EYEPIECE, LOSSLESS, "/ws/stream?source=main"];
    let joins = paths.iter().cycle().take(12).map(|path| server.connect_registered(path));
    let mut clients: Vec<_> = futures_util::future::join_all(joins).await;
    // Concurrent `connect_registered` calls all read the count before any joined, so each
    // returns once *a* viewer registered: wait for all of them before rendering.
    let stream = &server.state.main_stream;
    eventually("all twelve viewers to register", || {
        stream.viewer_count(StreamKind::Jpeg) == 9 && stream.viewer_count(StreamKind::Lossless) == 3
    })
    .await;

    let mut previous: Option<Vec<Vec<u8>>> = None;
    for fill in [0.3, 0.6, 0.9] {
        server.render(IMX533, fill).await;
        let receipts = clients.iter_mut().map(next_frame);
        let frames = futures_util::future::join_all(receipts).await;

        for family in [StreamKind::Jpeg, StreamKind::Lossless] {
            let of_family: Vec<_> = paths
                .iter()
                .cycle()
                .zip(&frames)
                .filter(|(path, _)| kind_of(path) == family)
                .map(|(_, frame)| frame)
                .collect();
            assert!(of_family.windows(2).all(|w| w[0] == w[1]), "{family:?} differs at {fill}");
        }
        if let Some(previous) = &previous {
            assert_ne!(previous[0], frames[0], "a client was sent the previous frame again");
        }
        previous = Some(frames);
    }
}

/// A client joining a watched stream mid-session is shown the frame everyone else is
/// looking at right away, then follows along.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_late_joiner_gets_the_current_frame_then_follows() {
    let server = start_server().await;
    let mut early = server.connect_registered(LOSSLESS).await;
    server.render(IMX533, 0.25).await;
    let current = next_frame(&mut early).await;

    let mut late = server.connect(LOSSLESS).await;
    assert_eq!(next_frame(&mut late).await, current, "the late joiner saw a different frame");

    server.render(IMX533, 0.75).await;
    let next = next_frame(&mut early).await;
    assert_ne!(next, current);
    assert_eq!(next_frame(&mut late).await, next);
}

/// Several clients arriving together at a family nobody watched while the frame rendered
/// all get it, from one shared on-demand encode.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn simultaneous_first_clients_of_an_unwatched_family_all_get_the_frame() {
    let server = start_server().await;
    server.render(IMX533, 0.25).await;

    for path in [LOSSLESS, LIVE_VIEW] {
        let joins = (0..8).map(|_| async {
            let mut client = server.connect(path).await;
            next_frame(&mut client).await
        });
        let frames = futures_util::future::join_all(joins).await;
        assert!(frames.windows(2).all(|w| w[0] == w[1]), "{path}: first frames differ");
        assert_eq!(dimensions(&frames[0]), (1440, 1440), "{path}");
    }
}

/// Clients waiting before anything was rendered get nothing, then the first frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn clients_waiting_for_the_first_frame_each_get_it() {
    let server = start_server().await;
    let mut jpeg = server.connect_registered(EYEPIECE).await;
    let mut lossless = server.connect_registered(LOSSLESS).await;
    assert_no_frame(&mut lossless, Duration::from_millis(300), "before any render").await;

    server.render(IMX533, 0.25).await;
    assert_jpeg(&next_frame(&mut jpeg).await, (1440, 1440), "first JPEG frame");
    assert_lossless(&next_frame(&mut lossless).await, (1440, 1440), "first lossless frame");
}

/// Odd widths (IMX464 at 1440p is 2539 px wide) arrive as complete frames, and a setting
/// above the frame's size never upscales it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn odd_widths_are_complete_and_small_frames_are_not_upscaled() {
    let server = start_server().await;
    let mut eyepiece = server.connect_registered(LOSSLESS).await;
    let mut live = server.connect_registered(LIVE_VIEW).await;

    server.render(IMX464, 0.25).await;
    assert_lossless(&next_frame(&mut eyepiece).await, (2539, 1440), "IMX464 at 1440p");
    assert_jpeg(&next_frame(&mut live).await, (2539, 1440), "IMX464 at 1440p");

    {
        let mut settings = server.state.settings.write().await;
        settings.streaming_resolution = Resolution::Uhd2160;
        settings.eyepiece.stream_resolution = EyepieceStreamResolution::Native;
    }
    server.render((1201, 901), 0.25).await;
    assert_lossless(&next_frame(&mut eyepiece).await, (1201, 901), "Native, smaller frame");
    assert_jpeg(&next_frame(&mut live).await, (1201, 901), "4K, smaller frame");
}

/// Viewport reports, garbage and binary from a client change nothing: no extra frame,
/// no re-encode, and pings are still answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn client_messages_other_than_ping_are_ignored() {
    let server = start_server().await;
    let mut chatty = server.connect_registered(LOSSLESS).await;
    server.render(IMX533, 0.25).await;
    let frame = next_frame(&mut chatty).await;

    send_viewport(&mut chatty, 1920, 1080).await;
    send_viewport(&mut chatty, 7680, 4320).await;
    chatty.send(Message::text("not json")).await.unwrap();
    chatty.send(Message::binary(vec![1, 2, 3])).await.unwrap();
    assert_no_frame(&mut chatty, Duration::from_millis(300), "after client messages").await;

    chatty.send(Message::text("ping")).await.unwrap();
    let pong = tokio::time::timeout(Duration::from_secs(5), futures_util::StreamExt::next(&mut chatty))
        .await
        .unwrap();
    assert!(matches!(pong, Some(Ok(Message::Text(ref t))) if t.as_str() == "pong"), "{pong:?}");

    server.render(IMX533, 0.5).await;
    let next = next_frame(&mut chatty).await;
    assert_eq!(dimensions(&next), dimensions(&frame), "a viewport report changed the size");
}

/// A guide-camera client is served by the guide stream only: imaging frames never reach
/// it, and it does not make the imaging producer encode anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_guide_client_is_isolated_from_the_imaging_stream() {
    let server = start_server().await;
    let mut guide = server.connect_registered("/ws/stream?source=guide").await;
    assert_eq!(server.state.main_stream.viewer_count(StreamKind::Jpeg), 0);

    server.render(IMX533, 0.25).await;
    assert_no_frame(&mut guide, Duration::from_millis(300), "guide client, imaging frame").await;
    let counter = server.state.main_stream.frame_counter();
    assert!(server.state.main_stream.payload(StreamKind::Jpeg, counter).is_none());
}
