//! WebSocket handlers for real-time image streaming and events
//!
//! This module provides WebSocket endpoints for:
//! - Live image streaming (binary JPEG frames)
//! - Event notifications (state changes, frame captures, errors)

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

use super::events::ServerEvent;
use super::state::{AppState, CameraRole, FrameStream, JpegTier, StreamKind, TierClientGuard};

/// WebSocket handler for raw image streaming (eyepiece quality)
///
/// Streams the latest captured/stacked frame as binary data (LZ4).
/// Clients connect to `/ws/eyepiece_quality` to receive lossless frames.
///
/// Protocol:
/// - Server sends binary messages containing frame data (LZ4 compressed RGB8)
/// - Client can send "ping" text messages to keep connection alive
/// - Server pushes frames as soon as they are rendered
pub async fn eyepiece_quality_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_eyepiece_quality(socket, state))
}

/// Handle the lossless image stream WebSocket connection. Like the JPEG handler,
/// the client's viewport selects a resolution tier — the render task box-averages
/// down to it rather than shipping a near-native frame for the browser to minify.
/// Not cosmetic: an area average to display size removes noise the GPU's four-tap
/// bilinear minification treats as aliasing instead, measured at 1.22x fewer output
/// levels of sky sigma for a 1440p view of an IMX533 frame (1.03x on IMX464, which
/// the 1440 tier barely shrinks — `display_output_tests` reports both).
async fn handle_eyepiece_quality(mut socket: WebSocket, state: Arc<AppState>) {
    // Eyepiece view is always the imaging camera: the guide scope has neither the
    // focal length nor the field the simulation is built around.
    let stream = Arc::clone(&state.main_stream);

    // The only registration this connection makes: the render task reads both
    // "is anyone watching" and "what box" off the tier counters, so one guard
    // carries both and they cannot drift apart. Dropped — and decremented —
    // even if this handler unwinds.
    let mut tier_guard = TierClientGuard::new(
        Arc::clone(&stream),
        StreamKind::Lossless,
        JpegTier::LOSSLESS_DEFAULT,
    );

    // Subscribed before the first send, so a frame published while that send is in
    // flight is latched rather than lost.
    let mut frames = stream.subscribe_frames();
    let mut last_frame_counter: u64 = stream.frame_counter();

    let primed = lossless_payload_for_new_client(&stream, tier_guard.tier()).await;
    if let Some((counter, payload)) = primed {
        if socket.send(Message::Binary(payload)).await.is_err() {
            return;
        }
        last_frame_counter = counter;
    }

    loop {
        tokio::select! {
            // Check for incoming messages (pings, resolution requests, close requests)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle ping/pong or commands
                        if text == "ping" {
                            if socket.send(Message::Text("pong".into())).await.is_err() {
                                break;
                            }
                            continue;
                        }

                        let Ok(req) = serde_json::from_str::<ResolutionRequest>(&text) else {
                            continue;
                        };
                        let requested = JpegTier::for_request(req.width, req.height);
                        // Most viewport changes stay inside the same tier. Skipping those
                        // matters more here than on the JPEG path: a re-prime below is a
                        // full-size LZ4 encode on this task, and `updateBounds` reports on
                        // every layout change the ResizeObserver sees.
                        if requested == tier_guard.tier() {
                            continue;
                        }
                        tier_guard.set_tier(requested);
                        let primed =
                            lossless_payload_for_new_client(&stream, requested).await;
                        if let Some((counter, payload)) = primed {
                            if socket.send(Message::Binary(payload)).await.is_err() {
                                break;
                            }
                            last_frame_counter = counter;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        // Client disconnected
                        break;
                    }
                    Some(Err(_)) => {
                        // Error receiving message
                        break;
                    }
                    _ => {}
                }
            }

            // Send frames when a new one is ready
            changed = frames.changed() => {
                // Only the producer holds the sender, and it outlives every handler.
                if changed.is_err() {
                    break;
                }
                let current_counter = *frames.borrow_and_update();
                if current_counter <= last_frame_counter {
                    continue;
                }
                // The producer writes the payload before publishing the counter, so a
                // tag older than the counter means it skipped this frame, not that we
                // raced it.
                let Some((counter, payload)) = stream.get_latest_frame().await else {
                    continue;
                };
                if counter <= last_frame_counter {
                    continue;
                }
                if socket.send(Message::Binary(payload)).await.is_err() {
                    break;
                }
                last_frame_counter = counter;
            }
        }
    }
}

/// The payload a lossless client should be shown the moment it arrives, or changes tier.
///
/// Without this the view stays black until the next exposure completes, because the
/// render task writes `latest_frame` only while a lossless client is *already*
/// registered — so the first frame of any connection is always missing, and at 60 s
/// subs that is a minute of nothing. A client connecting after capture stopped would
/// wait forever.
///
/// Deliberately not stored back into the stream: `latest_frame` is sized to the largest
/// connected tier, and overwriting it from a smaller client would shrink the frame every
/// other viewer sees.
async fn lossless_payload_for_new_client(
    stream: &Arc<FrameStream>,
    tier: JpegTier,
) -> Option<(u64, bytes::Bytes)> {
    let counter = stream.frame_counter();

    // Reuse the shared payload when it is both current and already at this client's
    // size; anything else is re-encoded, since a stale tag means a frame from a
    // previous session.
    if let Some((stored, payload)) = stream.get_latest_frame().await {
        if stored == counter && stream.lossless_target_box() == tier.lossless_box() {
            return Some((stored, payload));
        }
    }

    let frame = stream.get_latest_raw_frame().await?;
    let (max_w, max_h) = tier.lossless_box();
    // Single chunk: this runs on a blocking thread borrowed from the pool while the
    // render task may be mid-frame, and the latency of one client's first frame is not
    // worth taking cores off the stack for.
    let encoded = tokio::task::spawn_blocking(move || {
        crate::server::encoding::encode_rgb8_lz4_chunked(&frame, 1, max_w, max_h)
    })
    .await
    .ok()?
    .ok()?;

    Some((counter, bytes::Bytes::from(encoded)))
}

#[derive(Deserialize, Debug)]
struct ResolutionRequest {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

/// Fetch the payload a freshly-arrived (or freshly-retiered) client should see.
///
/// The render task does not know about a tier until it has a client, so on
/// connect there is usually nothing cached yet. Rather than leave the view empty
/// until the next frame — which can be a whole exposure away — encode once here
/// and publish it so other clients arriving on the same tier reuse it.
async fn payload_for_new_client(
    stream: &Arc<FrameStream>,
    tier: JpegTier,
) -> Option<(u64, bytes::Bytes)> {
    let counter = stream.frame_counter();
    if let Some(cached) = stream.get_tier_jpeg(tier, counter) {
        return Some((counter, cached));
    }

    let frame = stream.get_latest_raw_frame().await?;
    let (max_w, max_h) = tier.bounding_box();
    let encoded = tokio::task::spawn_blocking(move || {
        crate::server::encoding::encode_rgb8_jpeg_bounded(&frame, max_w, max_h)
    })
    .await
    .ok()?
    .ok()?;

    Some((counter, stream.set_tier_jpeg(tier, counter, encoded)))
}

/// Which camera's stream a client asked for, as `?source=main|guide`.
///
/// A query parameter rather than a second route: the protocol is byte-for-byte the
/// same, and the frontend swaps the source on one socket when the *Guide camera* toggle
/// flips. Anything unrecognised — or absent — is the imaging camera, so every existing
/// client keeps working untouched.
#[derive(Deserialize, Debug, Default)]
pub struct StreamSourceQuery {
    #[serde(default)]
    source: Option<String>,
}

impl StreamSourceQuery {
    fn role(&self) -> CameraRole {
        match self.source.as_deref() {
            Some("guide") => CameraRole::Guide,
            _ => CameraRole::Main,
        }
    }
}

/// WebSocket handler for JPEG streaming (dynamic resolution).
///
/// Used by both `/ws/stream` (main live view) and `/ws/eyepiece`
/// (eyepiece overlay). Both share the same handler since the protocol
/// is identical — clients send `{width, height}` JSON to set resolution.
pub async fn stream_handler(
    ws: WebSocketUpgrade,
    Query(source): Query<StreamSourceQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let stream = Arc::clone(state.stream(source.role()));
    ws.on_upgrade(move |socket| handle_dynamic_jpeg_stream(socket, stream))
}

/// Handle dynamic JPEG image streaming with client-specified resolution.
///
/// The client's requested viewport selects a fixed resolution tier; the render
/// task encodes that tier for as long as this handler holds its guard. Steady
/// state is therefore a cache read and a socket write, with no encoding on the
/// per-client path.
async fn handle_dynamic_jpeg_stream(mut socket: WebSocket, stream: Arc<FrameStream>) {
    // Registering this guard is also what tells the guide loop somebody is watching: it
    // renders and encodes only while a stream has viewers.
    let mut tier_guard = TierClientGuard::new(
        Arc::clone(&stream),
        StreamKind::Jpeg,
        JpegTier::for_request(None, None),
    );
    // Subscribed before the first send, for the same reason as the lossless handler.
    let mut frames = stream.subscribe_frames();
    let mut last_frame_counter: u64 = 0;

    if let Some((counter, payload)) = payload_for_new_client(&stream, tier_guard.tier()).await {
        if socket.send(Message::Binary(payload)).await.is_err() {
            return;
        }
        last_frame_counter = counter;
    }

    loop {
        tokio::select! {
            // Check for incoming messages (pings, resolution requests, close requests)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text == "ping" {
                            if socket.send(Message::Text("pong".into())).await.is_err() {
                                break;
                            }
                            continue;
                        }

                        let Ok(req) = serde_json::from_str::<ResolutionRequest>(&text) else {
                            continue;
                        };
                        let requested = JpegTier::for_request(req.width, req.height);
                        // Most viewport changes stay inside the same tier, in which
                        // case the client already has the right resolution.
                        if requested == tier_guard.tier() {
                            continue;
                        }
                        tier_guard.set_tier(requested);
                        if let Some((counter, payload)) = payload_for_new_client(&stream, requested).await {
                            if socket.send(Message::Binary(payload)).await.is_err() {
                                break;
                            }
                            last_frame_counter = counter;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(_)) => {
                        break;
                    }
                    _ => {}
                }
            }

            // Send frames when a new one is ready
            changed = frames.changed() => {
                if changed.is_err() {
                    break;
                }
                let current_counter = *frames.borrow_and_update();
                if current_counter <= last_frame_counter {
                    continue;
                }
                let Some(payload) = stream.get_tier_jpeg(tier_guard.tier(), current_counter) else {
                    continue;
                };
                if socket.send(Message::Binary(payload)).await.is_err() {
                    break;
                }
                last_frame_counter = current_counter;
            }
        }
    }
}

/// WebSocket handler for server events
///
/// Streams server events (state changes, frame captures, errors) as JSON.
/// Clients connect to `/ws/events` to receive notifications.
///
/// Protocol:
/// - Server sends JSON text messages with event data
/// - Client can send "ping" text messages to keep connection alive
pub async fn events_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_events(socket, state))
}

/// Handle the events WebSocket connection
async fn handle_events(mut socket: WebSocket, state: Arc<AppState>) {
    let mut events_rx = state.subscribe_events();

    // Send initial state
    let initial_state = state.capture_state().await;
    let initial_event = ServerEvent::state_changed(initial_state);
    if socket
        .send(Message::Text(initial_event.to_json().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            // Check for incoming messages
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text == "ping" && socket.send(Message::Text("pong".into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(_)) => {
                        break;
                    }
                    _ => {}
                }
            }

            // Forward events to client
            event = events_rx.recv() => {
                match event {
                    Ok(event) => {
                        if socket.send(Message::Text(event.to_json().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        // Client is too slow, send warning
                        let warning = ServerEvent::warning(format!("Dropped {} events (client too slow)", n));
                        let _ = socket.send(Message::Text(warning.to_json().into())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }
}

// event_to_json is now handled by ServerEvent::to_json() in events.rs

// Tests for ServerEvent serialization are now in events.rs

#[cfg(test)]
mod lossless_priming_tests {
    use super::*;
    use crate::frame::Frame;
    use crate::server::state::RenderReadyFrame;
    use crate::server::state::TierClientGuard;

    /// A frame the encoder can run on without a stretch solve behind it.
    fn ready_frame(width: usize, height: usize) -> Arc<RenderReadyFrame> {
        let config = crate::render::RenderPipelineConfig {
            contrast: false,
            auto_stretch: false,
            saturation_boost: false,
            ..Default::default()
        };
        Arc::new(RenderReadyFrame {
            linear_frame: Arc::new(Frame::filled(width, height, 3, 0.25).unwrap()),
            pipeline_config: config,
            stretch_result: None,
        })
    }

    /// Width and height out of the SA09 header.
    fn lz4_dimensions(payload: &[u8]) -> (u32, u32) {
        (
            u32::from_le_bytes(payload[4..8].try_into().unwrap()),
            u32::from_le_bytes(payload[8..12].try_into().unwrap()),
        )
    }

    /// The bug this whole path exists for: the render task writes `latest_frame` only
    /// while a lossless client is *already* registered, so the client that has just
    /// arrived has nothing to show. Without an on-demand encode the eyepiece stays
    /// black until the next exposure completes — a minute, at 60 s subs.
    #[tokio::test]
    async fn a_new_client_is_primed_from_the_raw_frame() {
        let stream = Arc::new(FrameStream::default());
        stream.set_latest_raw_frame(ready_frame(400, 300)).await;
        let counter = stream.begin_frame();
        stream.publish_frame();

        // Exactly the state a fresh connection finds: a rendered frame exists, but no
        // lossless payload was ever encoded for it.
        assert!(stream.get_latest_frame().await.is_none());

        let (tag, payload) = lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .expect("a connecting client was left with nothing to display");

        assert_eq!(tag, counter);
        assert_eq!(lz4_dimensions(&payload), (400, 300));
    }

    /// The other half of the same bug: a client that connects after capture stopped used
    /// to wait forever, because no further frame was ever going to be published.
    #[tokio::test]
    async fn a_client_arriving_after_capture_stopped_still_gets_the_last_frame() {
        let stream = Arc::new(FrameStream::default());
        stream.set_latest_raw_frame(ready_frame(320, 240)).await;
        stream.begin_frame();
        stream.publish_frame();
        // Nothing will ever publish again.

        assert!(lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .is_some());
    }

    /// A payload tagged with an older counter is a leftover from a previous session —
    /// `main_stream` is never cleared — so serving it as current would show the observer
    /// last night's target.
    #[tokio::test]
    async fn a_stale_payload_is_re_encoded_rather_than_served() {
        let stream = Arc::new(FrameStream::default());
        let stale = stream.begin_frame();
        stream.set_latest_frame(stale, vec![0xde; 64]).await;

        stream.set_latest_raw_frame(ready_frame(400, 300)).await;
        let current = stream.begin_frame();
        stream.publish_frame();

        let (tag, payload) = lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .expect("no payload");

        assert_eq!(tag, current, "the stale payload's counter was served as current");
        assert_ne!(payload.as_ref(), &[0xde; 64][..]);
        assert_eq!(lz4_dimensions(&payload), (400, 300));
    }

    /// When the shared payload is both current and already at this client's size, reuse
    /// it — an LZ4 encode of a full frame is not worth repeating per connection.
    #[tokio::test]
    async fn a_current_payload_at_the_right_size_is_reused() {
        let stream = Arc::new(FrameStream::default());
        let _guard = TierClientGuard::new(
            Arc::clone(&stream),
            StreamKind::Lossless,
            JpegTier::Hd1080,
        );
        stream.set_latest_raw_frame(ready_frame(400, 300)).await;
        let counter = stream.begin_frame();
        stream.set_latest_frame(counter, vec![0xab; 64]).await;
        stream.publish_frame();

        let (tag, payload) = lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .expect("no payload");

        assert_eq!(tag, counter);
        assert_eq!(payload.as_ref(), &[0xab; 64][..]);
    }

    /// The shared payload is sized to the *largest* connected tier. A client on a
    /// smaller one must be encoded for itself rather than handed a frame meant for a
    /// bigger screen — and, crucially, must not overwrite the shared payload with its
    /// smaller one.
    #[tokio::test]
    async fn a_smaller_tier_gets_its_own_encode_and_leaves_the_shared_payload_alone() {
        let stream = Arc::new(FrameStream::default());
        let _big = TierClientGuard::new(
            Arc::clone(&stream),
            StreamKind::Lossless,
            JpegTier::Original,
        );
        stream.set_latest_raw_frame(ready_frame(2000, 1500)).await;
        let counter = stream.begin_frame();
        stream.set_latest_frame(counter, vec![0xab; 64]).await;
        stream.publish_frame();

        let (_, payload) = lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .expect("no payload");

        let (w, h) = lz4_dimensions(&payload);
        assert!(w <= 1920 && h <= 1080, "small client got a {}x{} frame", w, h);

        let (_, shared) = stream.get_latest_frame().await.expect("shared payload gone");
        assert_eq!(
            shared.as_ref(),
            &[0xab; 64][..],
            "priming a small client overwrote the payload the large client is served"
        );
    }

    /// `Original` is unbounded for JPEG but capped for LZ4, so priming it from the raw
    /// `bounding_box` would both hand the client a native-resolution frame the stream
    /// will never send again, and never match the shared payload — costing a full encode
    /// on every connection at that tier.
    #[tokio::test]
    async fn the_original_tier_is_primed_at_the_same_cap_the_producer_uses() {
        let stream = Arc::new(FrameStream::default());
        let _guard = TierClientGuard::new(
            Arc::clone(&stream),
            StreamKind::Lossless,
            JpegTier::Original,
        );
        stream.set_latest_raw_frame(ready_frame(5000, 4000)).await;
        let counter = stream.begin_frame();
        stream.publish_frame();

        let (_, payload) = lossless_payload_for_new_client(&stream, JpegTier::Original)
            .await
            .expect("no payload");

        let (cap_w, cap_h) = crate::server::encoding::JPEG_MAX_BOUNDING_BOX;
        let (w, h) = lz4_dimensions(&payload);
        assert!(
            w <= cap_w && h <= cap_h,
            "primed at {w}x{h}, above the {cap_w}x{cap_h} cap the stream encodes into"
        );

        // And a payload the producer wrote at that same cap is reused, not re-encoded.
        stream.set_latest_frame(counter, vec![0xcd; 64]).await;
        let (_, reused) = lossless_payload_for_new_client(&stream, JpegTier::Original)
            .await
            .expect("no payload");
        assert_eq!(reused.as_ref(), &[0xcd; 64][..]);
    }

    /// With no rendered frame at all there is nothing to encode, and the handler must
    /// simply wait rather than send an empty message.
    #[tokio::test]
    async fn nothing_to_prime_from_yields_nothing() {
        let stream = Arc::new(FrameStream::default());
        assert!(lossless_payload_for_new_client(&stream, JpegTier::Hd1080)
            .await
            .is_none());
    }
}
