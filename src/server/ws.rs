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
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

use super::events::ServerEvent;
use super::state::AppState;

/// WebSocket handler for image streaming
///
/// Streams the latest captured/stacked frame as binary data.
/// Clients connect to `/ws/stream` to receive frames.
///
/// Protocol:
/// - Server sends binary messages containing frame data (LZ4 compressed RGB8)
/// - Client can send "ping" text messages to keep connection alive
/// - Server pushes frames as soon as they are rendered
pub async fn stream_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_stream(socket, state))
}

/// Handle the image stream WebSocket connection
async fn handle_stream(mut socket: WebSocket, state: Arc<AppState>) {
    let mut last_frame_counter: u64 = state.frame_counter.load(Ordering::SeqCst);

    // Send initial frame if available
    if let Some(frame_data) = state.get_latest_frame().await {
        if socket
            .send(Message::Binary(frame_data.as_ref().clone().into()))
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
                        if socket.send(Message::Binary(frame_data.as_ref().clone().into())).await.is_err() {
                            break;
                        }
                        last_frame_counter = current_counter;
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
