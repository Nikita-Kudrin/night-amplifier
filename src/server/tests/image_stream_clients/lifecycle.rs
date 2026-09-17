//! Clients leaving, stalling and churning must not disturb anyone else.

use std::time::Duration;

use serial_test::parallel;

use super::*;
use crate::server::state::StreamKind;

/// Leaving — cleanly or by dropping the connection — releases the viewer, and the
/// remaining clients keep receiving frames. A family whose last viewer left is no longer
/// encoded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn leaving_clients_release_their_viewer_and_others_keep_streaming() {
    let server = start_server().await;
    let stream = Arc::clone(&server.state.main_stream);
    let mut stays = server.connect_registered(LIVE_VIEW).await;
    let mut closes = server.connect_registered(LIVE_VIEW).await;
    let drops = server.connect_registered(LIVE_VIEW).await;
    let mut eyepiece = server.connect_registered(LOSSLESS).await;

    closes.close(None).await.unwrap();
    drop(drops);
    eventually("two JPEG viewers to leave", || stream.viewer_count(StreamKind::Jpeg) == 1).await;

    server.render(IMX533, 0.25).await;
    assert_jpeg(&next_frame(&mut stays).await, (1440, 1440), "remaining client");
    assert_lossless(&next_frame(&mut eyepiece).await, (1440, 1440), "eyepiece");

    eyepiece.close(None).await.unwrap();
    eventually("the lossless viewer to leave", || stream.viewer_count(StreamKind::Lossless) == 0).await;
    server.render(IMX533, 0.5).await;
    next_frame(&mut stays).await;
    let counter = stream.frame_counter();
    assert!(stream.payload(StreamKind::Lossless, counter).is_none(), "encoded for nobody");

    drop(stays);
    eventually("every viewer to leave", || !stream.has_viewers()).await;
}

/// A client that stops reading must not hold up the others: its socket backs up, theirs
/// keep receiving every frame. Noisy frames keep LZ4 from shrinking the payloads, so the
/// stalled socket's buffers actually fill.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_stalled_client_does_not_hold_up_the_others() {
    let server = start_server().await;
    let _stalled = server.connect_registered(LOSSLESS).await;
    let mut healthy = server.connect_registered(LOSSLESS).await;
    let mut live = server.connect_registered(LIVE_VIEW).await;
    {
        let mut settings = server.state.settings.write().await;
        settings.eyepiece.stream_resolution = crate::server::state::EyepieceStreamResolution::Native;
    }

    for seed in 1..=4 {
        server.render_noisy((2000, 2000), seed).await;
        let frame = next_frame(&mut healthy).await;
        assert_lossless(&frame, (2000, 2000), "healthy lossless client");
        assert!(frame.len() > 4_000_000, "payload compressed too well to back up a socket");
        next_frame(&mut live).await;
    }
}

/// Clients connecting and disconnecting in a storm — a flaky network reconnecting every
/// tab — leave the viewer count exact and a steady client unaffected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_reconnect_storm_leaves_counts_exact_and_steady_clients_unaffected() {
    let server = start_server().await;
    let stream = Arc::clone(&server.state.main_stream);
    let mut steady = server.connect_registered(LOSSLESS).await;
    server.render(IMX533, 0.25).await;
    next_frame(&mut steady).await;

    let storm = (0..20).map(|i| {
        let server = &server;
        async move {
            let path = [LIVE_VIEW, EYEPIECE, LOSSLESS][i % 3];
            let mut client = server.connect(path).await;
            if i % 2 == 0 {
                next_frame(&mut client).await;
            }
            if i % 4 == 0 {
                client.close(None).await.ok();
            }
        }
    });
    futures_util::future::join_all(storm).await;
    // Releases can wait on a first-frame encode — see the test below.
    eventually_within(FRAME_TIMEOUT, "the storm's viewers to leave", || {
        stream.viewer_count(StreamKind::Jpeg) == 0 && stream.viewer_count(StreamKind::Lossless) == 1
    })
    .await;

    server.render(IMX533, 0.5).await;
    assert_lossless(&next_frame(&mut steady).await, (1440, 1440), "steady client");
}

/// A client that leaves while its first frame is still being encoded registers nothing
/// that outlives it. The handler notices the closed socket only once that encode is done
/// and the send fails, so the release can take one encode — seconds for a 3008² frame in
/// a debug build under a full suite, hence the frame timeout rather than the usual 5 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_client_leaving_before_its_first_frame_leaks_nothing() {
    let server = start_server().await;
    server.render(IMX533, 0.25).await;

    for _ in 0..5 {
        let client = server.connect(LOSSLESS).await;
        drop(client);
    }
    let stream = Arc::clone(&server.state.main_stream);
    eventually_within(FRAME_TIMEOUT, "the departed viewers to be released", || !stream.has_viewers())
        .await;
}

/// When capture stops the stream is cleared: a client arriving then gets nothing (not a
/// frame from a camera that has gone), and every client resumes with the next session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_stopped_stream_serves_nothing_until_the_next_session() {
    let server = start_server().await;
    let mut before = server.connect_registered(LIVE_VIEW).await;
    server.render(IMX533, 0.25).await;
    next_frame(&mut before).await;

    server.state.main_stream.clear().await;
    let mut during = server.connect_registered(LIVE_VIEW).await;
    assert_no_frame(&mut during, Duration::from_millis(300), "joined a stopped stream").await;

    server.render(IMX533, 0.5).await;
    let resumed = next_frame(&mut before).await;
    assert_eq!(next_frame(&mut during).await, resumed);
}
