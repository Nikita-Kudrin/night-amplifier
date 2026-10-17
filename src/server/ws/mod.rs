//! WebSocket handlers for real-time image streaming and events
//!
//! - `/ws/stream`: dynamic JPEG (SA10) for the operator on `/`, `?source=guide` for the
//!   guide camera
//! - `/ws/eyepiece` (JPEG), `/ws/eyepiece_quality` (lossless RGB8+LZ4, SA09): whichever
//!   camera the operator views, switched on the open socket
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
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::session::events::{ServerEvent, SyncCause};
use crate::session::state::{AppState, CameraRole};
pub use image_stream::{PeerAddr, StreamClient, StreamEndpoint, StreamSource, NO_FRAME};

/// The close code a page sends as it drops a socket that answered nothing — no message,
/// no `pong` — for its liveness window (`useWebSocket`'s `dropDead`).
pub const CLOSE_UNRESPONSIVE: u16 = 4000;

/// Every WebSocket route, relative to the `/ws` nest.
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/eyepiece_quality", get(eyepiece_quality_handler))
        .route("/eyepiece", get(eyepiece_handler))
        .route("/stream", get(stream_handler))
        .route("/events", get(events_handler))
}

/// Lossless RGB8+LZ4 frames for the eyepiece quality view, at Eyepiece Streaming
/// Resolution, from the camera the operator views. A `?source=` is ignored: the viewers
/// have no say in it.
pub async fn eyepiece_quality_handler(
    ws: WebSocketUpgrade,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    image_stream_socket(ws, StreamEndpoint::EyepieceQuality, StreamSource::Viewed, peer, state)
}

/// JPEG at Streaming Resolution for the eyepiece overlay page, from the camera the
/// operator views, like [`eyepiece_quality_handler`].
pub async fn eyepiece_handler(
    ws: WebSocketUpgrade,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    image_stream_socket(ws, StreamEndpoint::Eyepiece, StreamSource::Viewed, peer, state)
}

/// JPEG at Streaming Resolution for the live view page.
pub async fn stream_handler(
    ws: WebSocketUpgrade,
    Query(source): Query<StreamSourceQuery>,
    PeerAddr(peer): PeerAddr,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let source = StreamSource::Fixed(source.role());
    image_stream_socket(ws, StreamEndpoint::LiveView, source, peer, state)
}

fn image_stream_socket(
    ws: WebSocketUpgrade,
    endpoint: StreamEndpoint,
    source: StreamSource,
    peer: Option<std::net::SocketAddr>,
    state: Arc<AppState>,
) -> impl IntoResponse {
    let client = StreamClient {
        endpoint,
        source,
        peer,
    };
    ws.on_upgrade(move |socket| image_stream::serve(socket, state, client))
}

/// Which camera's stream the live view asked for, as `?source=main|guide`.
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
/// - Server opens with a sync (see [`send_sync`]) and repeats it after the client lagged
/// - Client can send "ping" text messages to keep connection alive
pub async fn events_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_events(socket, state))
}

/// Bring a client up to date: every value topic's current events, then `synced`, on which
/// it re-reads every refetched topic (see `session::events::Topic`). Sent after subscribing,
/// so a client always converges: a value is changed before its event is sent, and every
/// event sent after the snapshot was read follows on this socket. Returns `false` once the
/// socket is gone.
async fn send_sync(socket: &mut WebSocket, state: &AppState, cause: SyncCause, dropped: u64) -> bool {
    let synced = ServerEvent::synced(cause, dropped);
    for event in state.sync_snapshot().into_iter().chain([synced]) {
        if socket.send(Message::Text(event.to_json().into())).await.is_err() {
            return false;
        }
    }
    true
}

/// Handle the events WebSocket connection
async fn handle_events(mut socket: WebSocket, state: Arc<AppState>) {
    let mut events_rx = state.subscribe_events();

    if !send_sync(&mut socket, &state, SyncCause::Connect, 0).await {
        return;
    }

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if text == "ping" && socket.send(Message::Text("pong".into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        if frame.is_some_and(|frame| frame.code == CLOSE_UNRESPONSIVE) {
                            warn!("An events client dropped its socket as unresponsive; it reconnects and syncs");
                        }
                        break;
                    }
                    None | Some(Err(_)) => break,
                    _ => {}
                }
            }

            event = events_rx.recv() => {
                match event {
                    Ok(event) => {
                        if socket.send(Message::Text(event.to_json().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(dropped)) => {
                        // The backlog (up to the channel's 256) is stale beside the snapshot
                        // that replaces it, and feeding it to a slow client lags it again.
                        events_rx = events_rx.resubscribe();
                        debug!(dropped, "Events client lagged; resyncing it");
                        if !send_sync(&mut socket, &state, SyncCause::Lagged, dropped).await {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }
}
