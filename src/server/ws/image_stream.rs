//! Image streams: `/ws/stream`, `/ws/eyepiece` (JPEG) and `/ws/eyepiece_quality`
//! (RGB8+LZ4). One handler serves all three; they differ only in the payload family and
//! in whose camera they show (see [`StreamSource`]).
//!
//! The size is a setting, not negotiated: every client of a family receives the same
//! bytes, at Streaming Resolution (JPEG) or Eyepiece Streaming Resolution (lossless). A
//! changed setting reaches connected clients with the next rendered frame. Text a client
//! sends other than `ping` is ignored, so frontends that still report their viewport keep
//! working.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use tracing::{info, warn};

use super::CLOSE_UNRESPONSIVE;

use crate::session::state::{AppState, CameraRole, FrameStream, Resolution, StreamKind, ViewerGuard};

/// Sent as text when an eyepiece socket switches to a camera with no frame yet: drop the
/// picture on screen. On the image socket, not `/ws/events`, so it is ordered with frames.
pub const NO_FRAME: &str = "no_frame";

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

/// Which camera's stream a connection serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamSource {
    /// One camera for the connection's whole life: `/ws/stream?source=`, the operator's view.
    Fixed(CameraRole),
    /// Whichever camera the operator views, switched in place on the open socket: the
    /// eyepiece pages, whose viewers have no control of their own.
    Viewed,
}

/// Who is on the other end of a stream connection, for logging.
#[derive(Debug, Clone, Copy)]
pub struct StreamClient {
    pub endpoint: StreamEndpoint,
    pub source: StreamSource,
    pub peer: Option<SocketAddr>,
}

/// The stream a connection is attached to: its viewer registration and frame wake-ups.
struct Attachment {
    camera: CameraRole,
    stream: Arc<FrameStream>,
    frames: tokio::sync::watch::Receiver<u64>,
    /// Registered before anything is sent, so a frame the producer renders from here on
    /// includes this family. Dropped — and decremented — even if the handler unwinds.
    _viewer: ViewerGuard,
}

impl Attachment {
    /// Subscribed before the first send, so a frame published while that send is in
    /// flight is latched rather than lost.
    fn new(state: &AppState, camera: CameraRole, kind: StreamKind) -> Self {
        let stream = Arc::clone(state.stream(camera));
        let viewer = ViewerGuard::new(Arc::clone(&stream), kind);
        let frames = stream.subscribe_frames();
        Self {
            camera,
            stream,
            frames,
            _viewer: viewer,
        }
    }
}

/// A frame send this slow is worth a warning: the `pong` the page's liveness check waits
/// for queues behind it, and past that window (`IMAGE_LIVENESS_MS`, `useImageStream`) the
/// page drops the socket mid-frame and reconnects — over and over, on a link that slow.
#[cfg(not(test))]
const SLOW_FRAME_SEND: Duration = Duration::from_secs(10);
#[cfg(test)]
const SLOW_FRAME_SEND: Duration = Duration::ZERO;

/// Sends one client its frames, timing each: the last one's size and duration are what
/// explain a page that dropped the socket as unresponsive.
struct FrameSender {
    client: StreamClient,
    last: Option<(usize, Duration)>,
}

impl FrameSender {
    fn new(client: StreamClient) -> Self {
        Self { client, last: None }
    }

    /// `false` once the socket is gone.
    async fn send(&mut self, socket: &mut WebSocket, payload: bytes::Bytes) -> bool {
        let bytes = payload.len();
        let started = Instant::now();
        let delivered = socket.send(Message::Binary(payload)).await.is_ok();
        let took = started.elapsed();
        self.last = Some((bytes, took));
        if took >= SLOW_FRAME_SEND {
            warn!(
                page = self.client.endpoint.page(),
                peer = %describe_peer(self.client.peer),
                bytes,
                took = ?took,
                kib_per_s = (bytes as f64 / 1024.0 / took.as_secs_f64().max(1e-6)).round(),
                delivered,
                "Image stream frame was slow to send; past its liveness window the page drops the socket mid-frame"
            );
        }
        delivered
    }

    /// Warn when the page closed the socket because nothing came back to its ping, naming
    /// the frame that was in flight.
    fn closed(&self, frame: Option<&CloseFrame>) {
        let Some(frame) = frame.filter(|frame| frame.code == CLOSE_UNRESPONSIVE) else {
            return;
        };
        let (bytes, took) = self.last.map_or((None, None), |(bytes, took)| (Some(bytes), Some(took)));
        warn!(
            page = self.client.endpoint.page(),
            peer = %describe_peer(self.client.peer),
            reason = %frame.reason,
            last_frame_bytes = ?bytes,
            last_frame_took = ?took,
            "The page dropped the image stream as unresponsive"
        );
    }
}

/// The resolution a family streams at now — the same live setting the producers read.
pub(super) async fn configured_resolution(state: &AppState, kind: StreamKind) -> Resolution {
    state.settings.snapshot().stream_resolution(kind)
}

/// Serve one image stream connection until the client leaves.
pub async fn serve(mut socket: WebSocket, state: Arc<AppState>, client: StreamClient) {
    let kind = client.endpoint.kind();
    let follows_view = client.source == StreamSource::Viewed;
    let mut viewed = state.viewed_camera.subscribe();
    let camera = match client.source {
        StreamSource::Fixed(camera) => camera,
        StreamSource::Viewed => *viewed.borrow_and_update(),
    };
    let mut attached = Attachment::new(&state, camera, kind);

    let resolution = configured_resolution(&state, kind).await;
    let output = describe_output(&attached.stream, resolution).await;
    info!(
        page = client.endpoint.page(),
        socket = client.endpoint.socket_path(),
        camera = ?camera,
        follows_view,
        peer = %describe_peer(client.peer),
        resolution = resolution.label(),
        output = %output,
        "Image stream client connected"
    );

    let mut sender = FrameSender::new(client);
    let mut last_sent: u64 = 0;
    if send_current(&mut socket, &mut sender, &state, &attached.stream, kind, &mut last_sent).await == Delivery::Closed {
        return;
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
                    Some(Ok(Message::Close(frame))) => {
                        sender.closed(frame.as_ref());
                        break;
                    }
                    Some(Err(_)) | None => break,
                    _ => {}
                }
            }

            changed = attached.frames.changed() => {
                // Only the producer holds the sender, and it outlives every handler.
                if changed.is_err() {
                    break;
                }
                let current = *attached.frames.borrow_and_update();
                if current <= last_sent {
                    continue;
                }
                // Missing when the producer moved on to a newer frame before this client
                // woke; that frame's own publication follows. Or the stream was cleared,
                // and the picture on screen no longer belongs to anything.
                let Some(payload) = attached.stream.payload(kind, current) else {
                    if !attached.stream.holds_frame() {
                        if socket.send(Message::Text(NO_FRAME.into())).await.is_err() {
                            break;
                        }
                        last_sent = current;
                    }
                    continue;
                };
                if !sender.send(&mut socket, payload).await {
                    break;
                }
                last_sent = current;
            }

            switched = viewed.changed(), if follows_view => {
                // `AppState` holds the sender for as long as any handler runs.
                if switched.is_err() {
                    break;
                }
                let camera = *viewed.borrow_and_update();
                if camera == attached.camera {
                    continue;
                }
                // The new viewer registers before the old one is dropped, so neither
                // producer sees a gap it would read as "nobody watching".
                attached = Attachment::new(&state, camera, kind);
                last_sent = 0;
                info!(
                    page = client.endpoint.page(),
                    camera = ?camera,
                    peer = %describe_peer(client.peer),
                    "Image stream client switched camera"
                );
                let delivery =
                    send_current(&mut socket, &mut sender, &state, &attached.stream, kind, &mut last_sent).await;
                // The client still shows the previous camera's picture, and with nothing to
                // replace it would go on showing it as this camera's, indefinitely.
                if delivery == Delivery::NothingToSend
                    && socket.send(Message::Text(NO_FRAME.into())).await.is_err()
                {
                    break;
                }
                if delivery == Delivery::Closed {
                    break;
                }
            }
        }
    }
}

/// What [`send_current`] managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Delivery {
    Sent,
    /// The stream holds no frame: none rendered yet, or cleared with its camera.
    NothingToSend,
    Closed,
}

/// Send the frame `stream` shows now, if it has one.
async fn send_current(
    socket: &mut WebSocket,
    sender: &mut FrameSender,
    state: &AppState,
    stream: &Arc<FrameStream>,
    kind: StreamKind,
    last_sent: &mut u64,
) -> Delivery {
    let Some((counter, payload)) = payload_for_client(state, stream, kind).await else {
        return Delivery::NothingToSend;
    };
    if !sender.send(socket, payload).await {
        return Delivery::Closed;
    }
    *last_sent = counter;
    Delivery::Sent
}

fn describe_peer(peer: Option<SocketAddr>) -> String {
    peer.map_or_else(|| "unknown".to_owned(), |p| p.to_string())
}

/// The size the latest frame streams at under `resolution`, as a log string.
async fn describe_output(stream: &FrameStream, resolution: Resolution) -> String {
    match stream.get_latest_raw_frame() {
        Some(frame) => {
            let (frame_w, frame_h) = (frame.linear_frame.width(), frame.linear_frame.height());
            let (max_w, max_h) = resolution.bounding_box();
            let (w, h) = crate::render::display::output_dimensions(frame_w, frame_h, max_w, max_h);
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
/// black until the next exposure — a minute at 60 s subs, forever once capture has
/// stopped. On-demand encodes of a family are serialised: clients arriving together
/// wait for the first encode and reuse it, rather than each paying for its own conversion.
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

    let frame = stream.get_latest_raw_frame()?;
    let (max_w, max_h) = configured_resolution(state, kind).await.bounding_box();
    // Single LZ4 chunk: this borrows a blocking thread while the render task may be
    // mid-frame, and one client's first frame is not worth taking cores off the stack.
    let encoded = tokio::task::spawn_blocking(move || match kind {
        StreamKind::Jpeg => crate::session::encoding::encode_rgb8_jpeg_bounded(&frame, max_w, max_h),
        StreamKind::Lossless => {
            crate::session::encoding::encode_rgb8_lz4_chunked(&frame, 1, max_w, max_h)
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
