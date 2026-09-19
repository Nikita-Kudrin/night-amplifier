//! A changed resolution reaches every connected client with the next frame.

use std::time::Duration;

use serde_json::json;
use serial_test::parallel;

use super::*;
use crate::server::state::{EyepieceStreamResolution, Resolution};

/// Changing Streaming Resolution through the settings endpoint resizes every JPEG client
/// on the next frame — not before — and leaves the eyepiece stream alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_streaming_resolution_change_reaches_every_jpeg_client_on_the_next_frame() {
    let server = start_server().await;
    let mut jpeg = vec![
        server.connect_registered(LIVE_VIEW).await,
        server.connect_registered(LIVE_VIEW).await,
        server.connect_registered(EYEPIECE).await,
    ];
    let mut eyepiece = server.connect_registered(LOSSLESS).await;
    server.render(IMX533, 0.25).await;
    for client in &mut jpeg {
        assert_jpeg(&next_frame(client).await, (1440, 1440), "before the change");
    }
    next_frame(&mut eyepiece).await;

    let response = server.post_settings(json!({"streaming_resolution": "hd1080"})).await;
    assert_eq!(response.status, 200, "{}", response.body);
    for client in &mut jpeg {
        assert_no_frame(client, Duration::from_millis(200), "between change and frame").await;
    }

    server.render(IMX533, 0.5).await;
    for client in &mut jpeg {
        assert_jpeg(&next_frame(client).await, (1080, 1080), "after the change");
    }
    assert_lossless(&next_frame(&mut eyepiece).await, (1440, 1440), "eyepiece untouched");
}

/// Changing Eyepiece Streaming Resolution resizes only `/eyepiece_quality`, including to
/// Native.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn an_eyepiece_resolution_change_reaches_only_the_lossless_clients() {
    let server = start_server().await;
    let mut eyepieces = vec![
        server.connect_registered(LOSSLESS).await,
        server.connect_registered(LOSSLESS).await,
    ];
    let mut live = server.connect_registered(LIVE_VIEW).await;

    let mut eyepiece_settings = serde_json::to_value(&server.state.settings.read().await.eyepiece).unwrap();
    eyepiece_settings["stream_resolution"] = json!("native");
    let response = server.post_settings(json!({"eyepiece": eyepiece_settings})).await;
    assert_eq!(response.status, 200, "{}", response.body);

    server.render(IMX533, 0.25).await;
    for client in &mut eyepieces {
        assert_lossless(&next_frame(client).await, (3008, 3008), "Native eyepiece");
    }
    assert_jpeg(&next_frame(&mut live).await, (1440, 1440), "live view untouched");
}

/// A client joining between a change and the next frame is shown the frame everybody
/// already has — at its size — and switches with everyone on the next frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_client_joining_after_a_change_matches_everyone_until_the_next_frame() {
    let server = start_server().await;
    let mut existing = server.connect_registered(LIVE_VIEW).await;
    server.render(IMX533, 0.25).await;
    let current = next_frame(&mut existing).await;

    server.state.settings.write().await.streaming_resolution = Resolution::Uhd2160;
    let mut joiner = server.connect(LIVE_VIEW).await;
    assert_eq!(next_frame(&mut joiner).await, current, "the joiner saw a different frame");

    server.render(IMX533, 0.5).await;
    assert_jpeg(&next_frame(&mut existing).await, (2160, 2160), "existing client");
    assert_jpeg(&next_frame(&mut joiner).await, (2160, 2160), "joiner");
}

/// Several changes between two frames: only the last one applies.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn only_the_last_of_several_changes_between_frames_applies() {
    let server = start_server().await;
    let mut client = server.connect_registered(LIVE_VIEW).await;

    for resolution in ["native", "hd1080", "uhd2160", "hd1080"] {
        let response = server.post_settings(json!({"streaming_resolution": resolution})).await;
        assert_eq!(response.status, 200, "{}", response.body);
    }
    server.render(IMX533, 0.25).await;
    assert_jpeg(&next_frame(&mut client).await, (1080, 1080), "last change");
}

/// 1080p is not an eyepiece option: the endpoint refuses it and changes nothing, while
/// every offered option is accepted and stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn the_eyepiece_setting_refuses_1080p_and_accepts_every_offered_option() {
    let server = start_server().await;
    let mut eyepiece = serde_json::to_value(&server.state.settings.read().await.eyepiece).unwrap();

    eyepiece["stream_resolution"] = json!("hd1080");
    let response = server.post_settings(json!({"eyepiece": eyepiece})).await;
    assert!(response.status >= 400, "1080p was accepted: {}", response.body);
    assert_eq!(
        server.state.settings.read().await.eyepiece.stream_resolution,
        EyepieceStreamResolution::Qhd1440
    );

    for (value, expected) in [
        ("uhd2160", EyepieceStreamResolution::Uhd2160),
        ("native", EyepieceStreamResolution::Native),
        ("qhd1440", EyepieceStreamResolution::Qhd1440),
    ] {
        eyepiece["stream_resolution"] = json!(value);
        let response = server.post_settings(json!({"eyepiece": eyepiece})).await;
        assert_eq!(response.status, 200, "{value}: {}", response.body);
        assert_eq!(server.state.settings.read().await.eyepiece.stream_resolution, expected);
    }

    let response = server.post_settings(json!({"streaming_resolution": "hd1080"})).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(server.state.settings.read().await.streaming_resolution, Resolution::Hd1080);
}

/// The capture loop snapshots settings when an exposure *starts*, and a resolution change
/// does not cancel the exposure. The frame that exposure renders must already follow the
/// change: encoding at the snapshot delayed it by up to two minutes at 60 s subs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_change_during_an_exposure_applies_to_the_frame_that_exposure_renders() {
    let server = start_server().await;
    let mut client = server.connect_registered(LIVE_VIEW).await;
    server.render(IMX533, 0.25).await;
    assert_jpeg(&next_frame(&mut client).await, (1440, 1440), "before the change");

    let exposure_started_with = server.state.settings.read().await.clone();
    let response = server.post_settings(json!({"streaming_resolution": "hd1080"})).await;
    assert_eq!(response.status, 200, "{}", response.body);

    server.render_with_snapshot(IMX533, 0.5, exposure_started_with).await;
    assert_jpeg(&next_frame(&mut client).await, (1080, 1080), "the next frame after the change");
}

/// A client opening a family nobody watched is encoded on demand from the live setting,
/// so the producer must use it too: an eyepiece display opened right after a change during
/// an exposure saw its size go new -> old -> new when the producer used the snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[parallel(image_stream_log)]
async fn a_client_joining_during_an_exposure_never_sees_the_size_flip_back() {
    let server = start_server().await;
    // Rendered with nobody on the lossless family, so no payload exists for it.
    server.render(IMX533, 0.25).await;

    let exposure_started_with = server.state.settings.read().await.clone();
    let mut eyepiece = serde_json::to_value(&server.state.settings.read().await.eyepiece).unwrap();
    eyepiece["stream_resolution"] = json!("native");
    let response = server.post_settings(json!({"eyepiece": eyepiece})).await;
    assert_eq!(response.status, 200, "{}", response.body);

    let mut client = server.connect_registered(LOSSLESS).await;
    let first = dimensions(&next_frame(&mut client).await);

    server.render_with_snapshot(IMX533, 0.5, exposure_started_with).await;
    let second = dimensions(&next_frame(&mut client).await);

    server.render(IMX533, 0.75).await;
    let third = dimensions(&next_frame(&mut client).await);

    assert_eq!(third, (3008, 3008), "the change never applied");
    assert!(
        !(first == third && second != first),
        "the joiner's size flipped: {first:?} -> {second:?} -> {third:?}"
    );
}
