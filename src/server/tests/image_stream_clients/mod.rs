//! End-to-end tests of the image stream sockets: a real axum server on a loopback port,
//! real WebSocket clients, and the real render task as producer. They pin that every
//! client of a family receives the same payload at the family's configured resolution,
//! however many clients connect, leave, stall or misbehave around it.

mod lifecycle;
mod logging;
mod serving;
mod settings_changes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::helpers::create_test_state;
use crate::frame::Frame;
use crate::server::capture::channel::{QueueDepth, StackedFrame};
use crate::server::state::AppState;

pub(super) type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) const LOSSLESS: &str = "/ws/eyepiece_quality";
pub(super) const LIVE_VIEW: &str = "/ws/stream";
pub(super) const EYEPIECE: &str = "/ws/eyepiece";
pub(super) const IMX533: (usize, usize) = (3008, 3008);
pub(super) const IMX464: (usize, usize) = (2712, 1538);
/// Generous: a debug-build render of a 3008² frame is slow, more so under a full suite.
pub(super) const FRAME_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct TestServer {
    pub state: Arc<AppState>,
    pub addr: SocketAddr,
}

/// The real WebSocket routes and the real settings endpoint, over loopback.
pub(super) async fn start_server() -> TestServer {
    let state = create_test_state();
    let app = axum::Router::new()
        .nest("/ws", crate::server::ws::routes())
        .route(
            "/api/settings",
            axum::routing::get(crate::server::api::get_settings)
                .post(crate::server::api::update_settings),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });
    TestServer { state, addr }
}

impl TestServer {
    pub async fn connect(&self, path: &str) -> Client {
        let (client, _) = tokio_tungstenite::connect_async(format!("ws://{}{path}", self.addr))
            .await
            .unwrap_or_else(|e| panic!("could not connect to {path}: {e}"));
        client
    }

    /// Connect and wait until the server has registered the viewer, so a render that
    /// follows is guaranteed to include this client's family.
    pub async fn connect_registered(&self, path: &str) -> Client {
        let kind = kind_of(path);
        let stream = if path.contains("source=guide") {
            Arc::clone(&self.state.guide_stream)
        } else {
            Arc::clone(&self.state.main_stream)
        };
        let before = stream.viewer_count(kind);
        let client = self.connect(path).await;
        eventually("the viewer to register", || stream.viewer_count(kind) > before).await;
        client
    }

    /// POST a partial settings update the way the settings panel does.
    pub async fn post_settings(&self, body: serde_json::Value) -> reqwest_like::Response {
        reqwest_like::post_json(self.addr, "/api/settings", body).await
    }

    /// Render one frame through the real render task, carrying the live settings — the
    /// snapshot the capture loop attaches — with the cosmetic stages off.
    pub async fn render(&self, size: (usize, usize), fill: f32) {
        self.render_frame(Frame::filled(size.0, size.1, 3, fill).unwrap()).await;
    }

    /// As [`Self::render`], with pixel noise so LZ4 cannot compress the payload away.
    pub async fn render_noisy(&self, size: (usize, usize), seed: u32) {
        let mut frame = Frame::zeros(size.0, size.1, 3).unwrap();
        let mut x = seed.max(1);
        for sample in frame.data_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *sample = (x % 1000) as f32 / 1000.0;
        }
        self.render_frame(frame).await;
    }

    /// As [`Self::render`], carrying `snapshot` instead of the live settings: the frame of
    /// an exposure that started before a settings change.
    pub async fn render_with_snapshot(
        &self,
        size: (usize, usize),
        fill: f32,
        snapshot: crate::server::state::CaptureSettings,
    ) {
        let frame = Frame::filled(size.0, size.1, 3, fill).unwrap();
        self.render_frame_with(frame, snapshot).await;
    }

    async fn render_frame(&self, frame: Frame) {
        let settings = self.state.settings.read().await.clone();
        self.render_frame_with(frame, settings).await;
    }

    async fn render_frame_with(&self, frame: Frame, mut settings: crate::server::state::CaptureSettings) {
        settings.auto_stretch = false;
        settings.background_subtraction = false;
        settings.saturation_boost = false;

        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(StackedFrame {
            noise: None,
            display_frame: Arc::new(frame),
            showing_stack: false,
            was_stacked: false,
            frame_number: 1,
            settings,
            stack_depth: 0,
        })
        .unwrap();
        drop(tx);
        let rt = tokio::runtime::Handle::current();
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            crate::server::capture::run_render_task(state, rx, QueueDepth::default(), rt)
        })
        .await
        .unwrap();
    }
}

fn kind_of(path: &str) -> crate::server::state::StreamKind {
    if path.starts_with(LOSSLESS) {
        crate::server::state::StreamKind::Lossless
    } else {
        crate::server::state::StreamKind::Jpeg
    }
}

/// A report in the shape the old per-client frontend sent. The server must ignore it.
pub(super) async fn send_viewport(client: &mut Client, width: u32, height: u32) {
    let json = format!(r#"{{"width":{width},"height":{height}}}"#);
    client.send(Message::text(json)).await.unwrap();
}

/// The next binary frame, skipping text (pongs).
pub(super) async fn next_frame(client: &mut Client) -> Vec<u8> {
    tokio::time::timeout(FRAME_TIMEOUT, async {
        loop {
            match client.next().await {
                Some(Ok(Message::Binary(payload))) => return payload.to_vec(),
                Some(Ok(_)) => continue,
                other => panic!("stream ended while waiting for a frame: {other:?}"),
            }
        }
    })
    .await
    .expect("no frame arrived")
}

/// Asserts no binary frame arrives within `wait`.
pub(super) async fn assert_no_frame(client: &mut Client, wait: Duration, context: &str) {
    let got = tokio::time::timeout(wait, async {
        loop {
            match client.next().await {
                Some(Ok(Message::Binary(payload))) => return dimensions(&payload),
                Some(Ok(_)) => continue,
                other => panic!("stream ended: {other:?}"),
            }
        }
    })
    .await;
    if let Ok((w, h)) = got {
        panic!("{context}: unexpected {w}x{h} frame");
    }
}

pub(super) fn magic(payload: &[u8]) -> u32 {
    u32::from_le_bytes(payload[0..4].try_into().unwrap())
}

/// Width and height out of an SA09 or SA10 header.
pub(super) fn dimensions(payload: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        u32::from_le_bytes(payload[8..12].try_into().unwrap()),
    )
}

/// Asserts a payload is JPEG (SA10) of the given size.
pub(super) fn assert_jpeg(payload: &[u8], size: (u32, u32), context: &str) {
    assert_eq!(magic(payload), crate::server::encoding::JPEG_MAGIC, "{context}: not SA10");
    assert_eq!(dimensions(payload), size, "{context}");
}

/// Asserts a payload is RGB8+LZ4 (SA09) of the given size and decodes completely.
pub(super) fn assert_lossless(payload: &[u8], size: (u32, u32), context: &str) -> Vec<u8> {
    assert_eq!(dimensions(payload), size, "{context}");
    decode_sa09(payload)
}

/// Decompress an SA09 payload into interleaved RGB8.
pub(super) fn decode_sa09(payload: &[u8]) -> Vec<u8> {
    use crate::server::encoding::{SA09_CHUNK_DESCRIPTOR_SIZE, SA09_HEADER_SIZE};
    assert_eq!(magic(payload), crate::server::encoding::RGB8_CHUNKED_MAGIC, "not SA09");
    let chunk_count = u32::from_le_bytes(payload[16..20].try_into().unwrap()) as usize;
    let mut data_offset = SA09_HEADER_SIZE + chunk_count * SA09_CHUNK_DESCRIPTOR_SIZE;
    let mut rgb = Vec::new();
    for i in 0..chunk_count {
        let desc = SA09_HEADER_SIZE + i * SA09_CHUNK_DESCRIPTOR_SIZE;
        let compressed = u32::from_le_bytes(payload[desc..desc + 4].try_into().unwrap()) as usize;
        let decompressed =
            u32::from_le_bytes(payload[desc + 4..desc + 8].try_into().unwrap()) as usize;
        let chunk = &payload[data_offset..data_offset + compressed];
        rgb.extend(lz4_flex::decompress(chunk, decompressed).unwrap());
        data_offset += compressed;
    }
    let (w, h) = dimensions(payload);
    assert_eq!(rgb.len(), w as usize * h as usize * 3, "SA09 payload is short");
    rgb
}

/// Poll until `condition` holds; the server notices joins and leaves asynchronously.
pub(super) async fn eventually(what: &str, condition: impl Fn() -> bool) {
    eventually_within(Duration::from_secs(5), what, condition).await;
}

pub(super) async fn eventually_within(limit: Duration, what: &str, condition: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// A minimal HTTP/1.1 client over the same loopback server, so settings changes go
/// through the real handler without another dev-dependency.
pub(super) mod reqwest_like {
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub struct Response {
        pub status: u16,
        pub body: String,
    }

    pub async fn post_json(addr: SocketAddr, path: &str, body: serde_json::Value) -> Response {
        let body = body.to_string();
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut raw = String::new();
        socket.read_to_string(&mut raw).await.unwrap();
        let status = raw
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("malformed HTTP response");
        let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_owned()).unwrap_or_default();
        Response { status, body }
    }
}
