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

    let mut last_frame_counter: u64 = stream.frame_counter();

    // Send initial frame if available
    if let Some(frame_data) = stream.get_latest_frame().await {
        if socket.send(Message::Binary(frame_data)).await.is_err() {
            return;
        }
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

                        // A viewport report. Unlike the JPEG path there is no
                        // per-tier cache to re-prime, so the next rendered frame
                        // simply arrives at the new size.
                        let Ok(req) = serde_json::from_str::<ResolutionRequest>(&text) else {
                            continue;
                        };
                        tier_guard.set_tier(JpegTier::for_request(req.width, req.height));
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
            _ = stream.frame_ready().notified() => {
                let current_counter = stream.frame_counter();

                // Only send if there's a new frame
                if current_counter > last_frame_counter {
                    if let Some(frame_data) = stream.get_latest_frame().await {
                        // Send binary frame data
                        if socket.send(Message::Binary(frame_data)).await.is_err() {
                            break;
                        }
                        last_frame_counter = current_counter;
                    }
                }
            }
        }
    }
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
            _ = stream.frame_ready().notified() => {
                let current_counter = stream.frame_counter();
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
