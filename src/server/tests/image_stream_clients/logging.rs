//! What the log says about stream clients and resolution changes.

use std::sync::Mutex;

use serde_json::json;

use super::*;

/// A `MakeWriter` that appends every log line to a shared buffer.
#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLog {
    fn lines_containing(&self, needle: &str) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .filter(|line| line.contains(needle))
            .map(str::to_owned)
            .collect()
    }
}

/// Logs to `CapturedLog` on this thread while the guards live. See the next test for why
/// a second dispatcher is held.
fn capture_log() -> (CapturedLog, (tracing::Dispatch, tracing::subscriber::DefaultGuard)) {
    let ask_every_dispatcher = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    (log, (ask_every_dispatcher, tracing::subscriber::set_default(subscriber)))
}

async fn server_showing_a_frame() -> TestServer {
    let server = start_server().await;
    server
        .state
        .main_stream
        .set_latest_raw_frame(Arc::new(crate::render::display::RenderReadyFrame {
            noise: None,
            linear_frame: Arc::new(crate::frame::Frame::filled(IMX533.0, IMX533.1, 3, 0.25).unwrap()),
            pipeline_config: crate::render::RenderPipelineConfig {
                contrast: false,
                auto_stretch: false,
                saturation_boost: false,
                ..Default::default()
            },
            stretch_result: None,
        }));
    server.state.main_stream.begin_frame();
    server.state.main_stream.publish_frame();
    server
}

/// One `info` line per connection naming the page, peer, resolution and output size;
/// nothing for client messages; one line per actual resolution change.
///
/// Current-thread runtime: the capturing subscriber is a thread-local default, so the
/// server's tasks must run on this thread to be heard.
///
/// While it is the only live dispatcher, tracing-core takes a new callsite's interest
/// from the default of whichever thread hits it *first* — a parallel settings test then
/// caches "never" for the resolution-change line and this test hears nothing (failed
/// under coverage). A second live dispatcher makes every registration ask all of them.
#[tokio::test(flavor = "current_thread")]
async fn connections_and_resolution_changes_are_logged_once_each() {
    let (log, _guards) = capture_log();
    let server = server_showing_a_frame().await;

    let mut eyepiece = server.connect(LOSSLESS).await;
    next_frame(&mut eyepiece).await;
    send_viewport(&mut eyepiece, 3840, 2160).await;
    let mut live = server.connect(LIVE_VIEW).await;
    next_frame(&mut live).await;

    let connected = log.lines_containing("Image stream client connected");
    assert_eq!(connected.len(), 2, "{connected:#?}");
    for expected in [
        "page=\"/eyepiece_quality\"",
        "resolution=\"1440p\"",
        "output=1440x1440 (frame 3008x3008)",
        "peer=127.0.0.1:",
    ] {
        assert!(connected[0].contains(expected), "missing {expected:?} in {}", connected[0]);
    }
    for expected in ["page=\"/\"", "/ws/stream", "resolution=\"1440p\""] {
        assert!(connected[1].contains(expected), "missing {expected:?} in {}", connected[1]);
    }

    // A real change logs once with both ends; re-sending the same value logs nothing.
    server.post_settings(json!({"streaming_resolution": "uhd2160"})).await;
    server.post_settings(json!({"streaming_resolution": "uhd2160"})).await;
    let changed = log.lines_containing("Streaming resolution changed");
    assert_eq!(changed.len(), 1, "{changed:#?}");
    assert!(changed[0].contains("from=\"1440p\"") && changed[0].contains("to=\"4K\""), "{}", changed[0]);

    let mut eyepiece_settings = serde_json::to_value(&server.state.settings.snapshot().eyepiece).unwrap();
    eyepiece_settings["intensity"] = json!(0.5);
    server.post_settings(json!({"eyepiece": eyepiece_settings.clone()})).await;
    assert!(log.lines_containing("Eyepiece streaming resolution changed").is_empty());
    eyepiece_settings["stream_resolution"] = json!("native");
    server.post_settings(json!({"eyepiece": eyepiece_settings})).await;
    let changed = log.lines_containing("Eyepiece streaming resolution changed");
    assert_eq!(changed.len(), 1, "{changed:#?}");
    assert!(changed[0].contains("to=\"Native\""), "{}", changed[0]);
}

/// A page whose ping went unanswered drops the socket with code 4000. On a link too slow
/// for one frame per liveness window that repeats forever, so the server says so: each
/// slow frame (every frame here, the test threshold being zero) and the drop, naming the
/// frame that was in flight.
#[tokio::test(flavor = "current_thread")]
async fn a_slow_frame_and_the_page_dropping_the_socket_for_it_are_logged() {
    use tokio_tungstenite::tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
    let (log, _guards) = capture_log();
    let server = server_showing_a_frame().await;

    let mut eyepiece = server.connect(LOSSLESS).await;
    let frame = next_frame(&mut eyepiece).await;
    let slow = log.lines_containing("Image stream frame was slow to send");
    assert_eq!(slow.len(), 1, "{slow:#?}");
    assert!(slow[0].contains(&format!("bytes={}", frame.len())) && slow[0].contains("delivered=true"), "{}", slow[0]);

    let close = CloseFrame { code: CloseCode::from(4000), reason: "No answer to ping".into() };
    eyepiece.send(Message::Close(Some(close))).await.unwrap();
    eventually("the drop to be logged", || !log.lines_containing("dropped the image stream as unresponsive").is_empty()).await;

    let dropped = log.lines_containing("dropped the image stream as unresponsive");
    for expected in ["page=\"/eyepiece_quality\"", "reason=No answer to ping", &format!("last_frame_bytes=Some({})", frame.len())] {
        assert!(dropped[0].contains(expected), "missing {expected:?} in {}", dropped[0]);
    }
}

/// A page leaving for its own reasons is no warning.
#[tokio::test(flavor = "current_thread")]
async fn an_ordinary_close_is_not_called_unresponsive() {
    let (log, _guards) = capture_log();
    let server = server_showing_a_frame().await;
    let stream = Arc::clone(&server.state.main_stream);

    let mut live = server.connect_registered(LIVE_VIEW).await;
    next_frame(&mut live).await;
    live.close(None).await.unwrap();
    eventually("the viewer to leave", || stream.viewer_count(crate::session::state::StreamKind::Jpeg) == 0).await;

    assert!(log.lines_containing("unresponsive").is_empty());
}
