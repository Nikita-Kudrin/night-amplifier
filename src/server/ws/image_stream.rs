//! Image streams: `/ws/stream`, `/ws/eyepiece` (JPEG) and `/ws/eyepiece_quality`
//! (RGB8+LZ4). One handler serves all three; they differ only in the payload family.
//!
//! The size is a setting, not negotiated: every client of a family receives the same
//! bytes, at Streaming Resolution (JPEG) or Eyepiece Streaming Resolution (lossless). A
//! changed setting reaches connected clients with the next rendered frame. Text a client
//! sends other than `ping` is ignored, so frontends that still report their viewport keep
//! working.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use tracing::info;

use crate::server::state::{AppState, CameraRole, FrameStream, Resolution, StreamKind, ViewerGuard};

/// The WebSocket endpoints that stream rendered frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEndpoint {
    /// `/ws/stream`, the live view page `/`.
    LiveView,
    /// `/ws/eyepiece`, the page `/eyepiece`.
    Eyepiece,
    /// `/ws/eyepiece_quality`, the page `/eyepiece_quality`.
    EyepieceQuality,
}

impl StreamEndpoint {
    pub const fn kind(self) -> StreamKind {
        match self {
            Self::LiveView | Self::Eyepiece => StreamKind::Jpeg,
            Self::EyepieceQuality => StreamKind::Lossless,
        }
    }

    /// The frontend page that opens this socket.
    pub const fn page(self) -> &'static str {
        match self {
            Self::LiveView => "/",
            Self::Eyepiece => "/eyepiece",
            Self::EyepieceQuality => "/eyepiece_quality",
        }
    }

    pub const fn socket_path(self) -> &'static str {
        match self {
            Self::LiveView => "/ws/stream",
            Self::Eyepiece => "/ws/eyepiece",
            Self::EyepieceQuality => "/ws/eyepiece_quality",
        }
    }
}

/// The client's address, when the server was started with connect info. Requests
/// driven through `tower::ServiceExt::oneshot` carry none, so this never rejects.
pub struct PeerAddr(pub Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

/// Who is on the other end of a stream connection, for logging.
#[derive(Debug, Clone, Copy)]
pub struct StreamClient {
    pub endpoint: StreamEndpoint,
    pub camera: CameraRole,
    pub peer: Option<SocketAddr>,
}

/// The resolution a family streams at now — the same live setting the producers read.
pub(super) async fn configured_resolution(state: &AppState, kind: StreamKind) -> Resolution {
    state.settings.read().await.stream_resolution(kind)
}

/// Serve one image stream connection until the client leaves.
pub async fn serve(
    mut socket: WebSocket,
    state: Arc<AppState>,
    stream: Arc<FrameStream>,
    client: StreamClient,
) {
    let kind = client.endpoint.kind();
    // Registered before anything is sent, so a frame the producer renders from here on
    // includes this family. Dropped — and decremented — even if this handler unwinds.
    let _viewer = ViewerGuard::new(Arc::clone(&stream), kind);

    let resolution = configured_resolution(&state, kind).await;
    let output = describe_output(&stream, resolution).await;
    info!(
        page = client.endpoint.page(),
        socket = client.endpoint.socket_path(),
        camera = ?client.camera,
        peer = %client.peer.map_or_else(|| "unknown".to_owned(), |p| p.to_string()),
        resolution = resolution.label(),
        output = %output,
        "Image stream client connected"
    );

    // Subscribed before the first send, so a frame published while that send is in
    // flight is latched rather than lost.
    let mut frames = stream.subscribe_frames();
    let mut last_sent: u64 = 0;
    if let Some((counter, payload)) = payload_for_client(&state, &stream, kind).await {
        if socket.send(Message::Binary(payload)).await.is_err() {
            return;
        }
        last_sent = counter;
    }

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) if text == "ping" => {
                        if socket.send(Message::Text("pong".into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }

            changed = frames.changed() => {
                // Only the producer holds the sender, and it outlives every handler.
                if changed.is_err() {
                    break;
                }
                let current = *frames.borrow_and_update();
                if current <= last_sent {
                    continue;
                }
                // Missing when the producer moved on to a newer frame before this client
                // woke; that frame's own publication follows.
                let Some(payload) = stream.payload(kind, current) else {
                    continue;
                };
                if socket.send(Message::Binary(payload)).await.is_err() {
                    break;
                }
                last_sent = current;
            }
        }
    }
}

/// The size the latest frame streams at under `resolution`, as a log string.
async fn describe_output(stream: &FrameStream, resolution: Resolution) -> String {
    match stream.get_latest_raw_frame().await {
        Some(frame) => {
            let (frame_w, frame_h) = (frame.linear_frame.width(), frame.linear_frame.height());
            let (max_w, max_h) = resolution.bounding_box();
            let (w, h) = crate::server::encoding::output_dimensions(frame_w, frame_h, max_w, max_h);
            format!("{w}x{h} (frame {frame_w}x{frame_h})")
        }
        None => "no frame yet".to_owned(),
    }
}

/// The payload a newly-connected client should be shown now: the producer's when it is
/// current, otherwise one on-demand encode of the latest frame at the configured
/// resolution, stored for every other client.
///
/// The producer skips a family nobody watches, so without this the first client stays
/// black until the next exposure — a minute at 60 s subs, and forever once capture has
/// stopped. On-demand encodes of a family are serialised: clients arriving together wait
/// for the first encode and reuse it, rather than each converting the frame (and
/// allocating its denoise buffers).
pub(super) async fn payload_for_client(
    state: &AppState,
    stream: &Arc<FrameStream>,
    kind: StreamKind,
) -> Option<(u64, bytes::Bytes)> {
    let counter = stream.frame_counter();
    if let Some(payload) = stream.payload(kind, counter) {
        return Some((counter, payload));
    }

    let _encoding = stream.on_demand_encode_lock(kind).lock().await;
    let counter = stream.frame_counter();
    if let Some(payload) = stream.payload(kind, counter) {
        return Some((counter, payload));
    }

    let frame = stream.get_latest_raw_frame().await?;
    let (max_w, max_h) = configured_resolution(state, kind).await.bounding_box();
    // Single LZ4 chunk: this borrows a blocking thread while the render task may be
    // mid-frame, and one client's first frame is not worth taking cores off the stack.
    let encoded = tokio::task::spawn_blocking(move || match kind {
        StreamKind::Jpeg => crate::server::encoding::encode_rgb8_jpeg_bounded(&frame, max_w, max_h),
        StreamKind::Lossless => {
            crate::server::encoding::encode_rgb8_lz4_chunked(&frame, 1, max_w, max_h)
        }
    })
    .await
    .ok()?
    .ok()?;

    Some((counter, stream.set_payload(kind, counter, encoded)))
}

#[cfg(test)]
#[path = "image_stream_tests.rs"]
mod tests;
