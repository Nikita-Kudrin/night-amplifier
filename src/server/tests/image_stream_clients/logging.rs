//! What the log says about stream clients and resolution changes.

use std::sync::Mutex;

use serde_json::json;
use serial_test::serial;

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

/// One `info` line per connection naming the page, peer, resolution and output size;
/// nothing for client messages; one line per actual resolution change.
///
/// Current-thread runtime: the capturing subscriber is a thread-local default, so the
/// server's tasks must run on this thread to be heard. Serial with the other stream
/// tests: a callsite first hit on another thread while this subscriber registers can
/// cache "never" interest and drop the line.
#[tokio::test(flavor = "current_thread")]
#[serial(image_stream_log)]
async fn connections_and_resolution_changes_are_logged_once_each() {
    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = start_server().await;
    server
        .state
        .main_stream
        .set_latest_raw_frame(Arc::new(crate::server::state::RenderReadyFrame {
            noise: None,
            linear_frame: Arc::new(crate::frame::Frame::filled(IMX533.0, IMX533.1, 3, 0.25).unwrap()),
            pipeline_config: crate::render::RenderPipelineConfig {
                contrast: false,
                auto_stretch: false,
                saturation_boost: false,
                ..Default::default()
            },
            stretch_result: None,
        }))
        .await;
    server.state.main_stream.begin_frame();
    server.state.main_stream.publish_frame();

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

    let mut eyepiece_settings = serde_json::to_value(&server.state.settings.read().await.eyepiece).unwrap();
    eyepiece_settings["intensity"] = json!(0.5);
    server.post_settings(json!({"eyepiece": eyepiece_settings.clone()})).await;
    assert!(log.lines_containing("Eyepiece streaming resolution changed").is_empty());
    eyepiece_settings["stream_resolution"] = json!("native");
    server.post_settings(json!({"eyepiece": eyepiece_settings})).await;
    let changed = log.lines_containing("Eyepiece streaming resolution changed");
    assert_eq!(changed.len(), 1, "{changed:#?}");
    assert!(changed[0].contains("to=\"Native\""), "{}", changed[0]);
}
