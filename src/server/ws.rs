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
use super::state::AppState;

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
        if socket
            .send(Message::Binary(frame_data))
            .await
            .is_err()
        {
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
                        if text == "ping" {
                            if socket.send(Message::Text("pong".into())).await.is_err() {
                                break;
                            }
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

/// Clamp requested dimensions to the same bounds used by
/// `encode_rgb8_jpeg_dynamic` so cache keys match actual output.
fn clamp_resolution(req_w: Option<u32>, req_h: Option<u32>) -> (u32, u32) {
    let w = req_w.unwrap_or(1920).clamp(1920, 3840);
    let h = req_h.unwrap_or(1080).clamp(1080, 2160);
    (w, h)
}

/// Encode a raw frame as JPEG (with cache) and send it over the WebSocket.
///
/// Returns the frame counter of the frame that was encoded, or `Err` if the
/// socket should be closed.
async fn encode_jpeg_and_send(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    client_width: Option<u32>,
    client_height: Option<u32>,
) -> Result<u64, ()> {
    let counter = state.frame_counter.load(Ordering::SeqCst);
    let (max_w, max_h) = clamp_resolution(client_width, client_height);

    // Check cache first — another client may have already encoded this frame
    if let Some(cached) = state.get_cached_jpeg(counter, max_w, max_h).await {
        return if socket
            .send(Message::Binary(cached.clone()))
            .await
            .is_ok()
        {
            Ok(counter)
        } else {
            Err(())
        };
    }

    let Some(frame) = state.get_latest_raw_frame().await else {
        return Ok(counter);
    };

    let frame_clone = frame.clone();
    let cw = client_width;
    let ch = client_height;
    let encoded_result = tokio::task::spawn_blocking(move || {
        crate::server::encoding::encode_rgb8_jpeg_dynamic(&frame_clone, cw, ch)
    })
    .await
    .unwrap_or_else(|_| Err("Task panicked".to_string()));

    let Ok(encoded) = encoded_result else {
        return Ok(counter);
    };

    // Frame-skip: if a newer frame arrived while we were encoding, discard
    // this stale result and let the caller loop to encode the fresher frame.
    let current_counter = state.frame_counter.load(Ordering::SeqCst);
    if current_counter > counter {
        return Ok(counter);
    }

    // Store in cache for other clients at the same resolution
    let cached = state.cache_jpeg(counter, max_w, max_h, encoded).await;

    if socket
        .send(Message::Binary(cached))
        .await
        .is_err()
    {
        return Err(());
    }

    Ok(counter)
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

/// Handle dynamic JPEG image streaming with client-specified resolution
async fn handle_dynamic_jpeg_stream(mut socket: WebSocket, state: Arc<AppState>) {
    let mut last_frame_counter: u64 = state.frame_counter.load(Ordering::SeqCst);
    let mut client_width: Option<u32> = None;
    let mut client_height: Option<u32> = None;

    // Send initial frame if available
    if state.get_latest_raw_frame().await.is_some() {
        match encode_jpeg_and_send(&mut socket, &state, client_width, client_height).await {
            Ok(counter) => last_frame_counter = counter,
            Err(()) => return,
        }
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
                        } else if let Ok(req) = serde_json::from_str::<ResolutionRequest>(&text) {
                            client_width = req.width;
                            client_height = req.height;
                            // Re-send the latest frame immediately at the new resolution
                            if state.get_latest_raw_frame().await.is_some() {
                                match encode_jpeg_and_send(&mut socket, &state, client_width, client_height).await {
                                    Ok(counter) => last_frame_counter = counter,
                                    Err(()) => break,
                                }
                            }
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

                // Only send if there's a new frame
                if current_counter > last_frame_counter {
                    match encode_jpeg_and_send(&mut socket, &state, client_width, client_height).await {
                        Ok(counter) => last_frame_counter = counter,
                        Err(()) => break,
                    }
                }
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
                        if text == "ping" {
                            if socket.send(Message::Text("pong".into())).await.is_err() {
                                break;
                            }
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
