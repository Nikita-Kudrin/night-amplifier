//! WebSocket handlers for real-time image streaming and events
//!
//! - `/ws/stream`, `/ws/eyepiece`: dynamic JPEG (SA10), `?source=guide` for the guide camera
//! - `/ws/eyepiece_quality`: lossless RGB8+LZ4 (SA09), always the imaging camera
//! - `/ws/events`: JSON event notifications

mod image_stream;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

use super::events::ServerEvent;
use super::state::{AppState, CameraRole};
pub use image_stream::{PeerAddr, StreamClient, StreamEndpoint};

/// Every WebSocket route, relative to the `/ws` nest.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/eyepiece_quality", get(eyepiece_quality_handler))
        .route("/eyepiece", get(eyepiece_handler))
        .route("/stream", get(stream_handler))
        .route("/events", get(events_handler))
}

/// Lossless RGB8+LZ4 frames for the eyepiece quality view, at Eyepiece Streaming
/// Resolution. Always the imaging camera: the guide scope has neither the focal length nor
/// the field the view is built around.
pub async fn eyepiece_quality_handler(
    ws: WebSocketUpgrade,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let stream = Arc::clone(&state.main_stream);
    let client = StreamClient {
        endpoint: StreamEndpoint::EyepieceQuality,
        camera: CameraRole::Main,
        peer,
    };
    ws.on_upgrade(move |socket| image_stream::serve(socket, state, stream, client))
}

/// JPEG at Streaming Resolution for the live view page.
pub async fn stream_handler(
    ws: WebSocketUpgrade,
    Query(source): Query<StreamSourceQuery>,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    jpeg_stream(ws, StreamEndpoint::LiveView, source.role(), peer, state)
}

/// JPEG at Streaming Resolution for the eyepiece overlay page.
pub async fn eyepiece_handler(
    ws: WebSocketUpgrade,
    Query(source): Query<StreamSourceQuery>,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    jpeg_stream(ws, StreamEndpoint::Eyepiece, source.role(), peer, state)
}

fn jpeg_stream(
    ws: WebSocketUpgrade,
    endpoint: StreamEndpoint,
    camera: CameraRole,
    peer: Option<std::net::SocketAddr>,
    state: Arc<AppState>,
) -> impl IntoResponse {
    let stream = Arc::clone(state.stream(camera));
    let client = StreamClient {
        endpoint,
        camera,
        peer,
    };
    ws.on_upgrade(move |socket| image_stream::serve(socket, state, stream, client))
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

