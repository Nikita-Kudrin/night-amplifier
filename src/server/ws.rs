//! WebSocket handlers for real-time image streaming and events
//!
//! This module provides WebSocket endpoints for:
//! - Live image streaming (binary JPEG frames)
//! - Event notifications (state changes, frame captures, errors)

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::events::ServerEvent;
use super::state::{AppState, JpegTier, JpegTierClientGuard};

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

/// Handle the lossless image stream WebSocket connection
async fn handle_eyepiece_quality(mut socket: WebSocket, state: Arc<AppState>) {
    struct ClientGuard(Arc<AppState>);
    impl Drop for ClientGuard {
        fn drop(&mut self) {
            self.0.lz4_clients.fetch_sub(1, Ordering::SeqCst);
        }
    }
    state.lz4_clients.fetch_add(1, Ordering::SeqCst);
    let _guard = ClientGuard(state.clone());

    let mut last_frame_counter: u64 = state.frame_counter.load(Ordering::SeqCst);

    // Send initial frame if available
    if let Some(frame_data) = state.get_latest_frame().await {
        if socket.send(Message::Binary(frame_data)).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            // Check for incoming messages (pings, close requests)
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle ping/pong or commands
                        if text == "ping" && socket.send(Message::Text("pong".into())).await.is_err() {
                            break;
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
            _ = state.frame_ready.notified() => {
                let current_counter = state.frame_counter.load(Ordering::SeqCst);

                // Only send if there's a new frame
                if current_counter > last_frame_counter {
                    if let Some(frame_data) = state.get_latest_frame().await {
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
    state: &Arc<AppState>,
    tier: JpegTier,
) -> Option<(u64, bytes::Bytes)> {
    let counter = state.frame_counter.load(Ordering::SeqCst);
    if let Some(cached) = state.get_tier_jpeg(tier, counter) {
        return Some((counter, cached));
    }

    let frame = state.get_latest_raw_frame().await?;
    let (max_w, max_h) = tier.bounding_box();
    let encoded = tokio::task::spawn_blocking(move || {
        crate::server::encoding::encode_rgb8_jpeg_bounded(&frame, max_w, max_h)
    })
    .await
    .ok()?
    .ok()?;

    Some((counter, state.set_tier_jpeg(tier, counter, encoded)))
}

/// WebSocket handler for JPEG streaming (dynamic resolution).
///
/// Used by both `/ws/stream` (main live view) and `/ws/eyepiece`
/// (eyepiece overlay). Both share the same handler since the protocol
/// is identical — clients send `{width, height}` JSON to set resolution.
pub async fn stream_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_dynamic_jpeg_stream(socket, state))
}

/// Handle dynamic JPEG image streaming with client-specified resolution.
///
/// The client's requested viewport selects a fixed resolution tier; the render
/// task encodes that tier for as long as this handler holds its guard. Steady
/// state is therefore a cache read and a socket write, with no encoding on the
/// per-client path.
async fn handle_dynamic_jpeg_stream(mut socket: WebSocket, state: Arc<AppState>) {
    let mut tier_guard =
        JpegTierClientGuard::new(Arc::clone(&state), JpegTier::for_request(None, None));
    let mut last_frame_counter: u64 = 0;

    if let Some((counter, payload)) = payload_for_new_client(&state, tier_guard.tier()).await {
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
                        if let Some((counter, payload)) = payload_for_new_client(&state, requested).await {
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
            _ = state.frame_ready.notified() => {
                let current_counter = state.frame_counter.load(Ordering::SeqCst);
                if current_counter <= last_frame_counter {
                    continue;
                }
                let Some(payload) = state.get_tier_jpeg(tier_guard.tier(), current_counter) else {
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
