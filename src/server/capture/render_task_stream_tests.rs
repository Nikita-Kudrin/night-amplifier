//! The render task as the imaging stream's producer: what it publishes, at which size,
//! and for whom.

use std::sync::Arc;

use super::QueueDepth;
use crate::server::state::{
    AppState, CaptureSettings, EyepieceStreamResolution, RenderReadyFrame, Resolution, StreamKind,
    ViewerGuard,
};

/// Settings that make the preview pipeline a no-op, so a test observes only how frames
/// are published and not what the render stages do to them.
fn passthrough_settings() -> CaptureSettings {
    CaptureSettings {
        auto_stretch: false,
        background_subtraction: false,
        saturation_boost: false,
        ..CaptureSettings::default()
    }
}

fn stacked_frame(frame: Arc<crate::frame::Frame>, settings: CaptureSettings) -> super::StackedFrame {
    super::StackedFrame {
        noise: None,
        display_frame: frame,
        showing_stack: false,
        was_stacked: false,
        frame_number: 1,
        settings,
        stack_depth: 0,
    }
}

/// Run frames through `run_render_task`, one at a time, until the channel closes.
async fn render(state: &Arc<AppState>, frames: Vec<super::StackedFrame>) {
    let (tx, rx) = std::sync::mpsc::channel();
    for frame in frames {
        tx.send(frame).unwrap();
    }
    drop(tx);
    let rt = tokio::runtime::Handle::current();
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || super::run_render_task(state, rx, QueueDepth::default(), rt))
        .await
        .unwrap();
}

/// Render frames one render-task run each, so draining never merges them.
async fn render_each(state: &Arc<AppState>, frames: Vec<super::StackedFrame>) {
    for frame in frames {
        render(state, vec![frame]).await;
    }
}

fn filled(width: usize, height: usize) -> Arc<crate::frame::Frame> {
    Arc::new(crate::frame::Frame::filled(width, height, 3, 0.25).unwrap())
}

/// Width and height out of an SA09 or SA10 header.
fn dimensions(payload: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        u32::from_le_bytes(payload[8..12].try_into().unwrap()),
    )
}

fn current_size(state: &AppState, kind: StreamKind) -> (u32, u32) {
    let counter = state.main_stream.frame_counter();
    let payload = state
        .main_stream
        .payload(kind, counter)
        .unwrap_or_else(|| panic!("no {kind:?} payload for frame {counter}"));
    dimensions(&payload)
}

/// Run one frame through the render task, returning the pixel buffer address it came in
/// at and the one the published raw frame ended up at.
async fn render_and_report_buffer_addr(
    frame: crate::frame::Frame,
    keep_extra_handle: bool,
) -> (usize, usize) {
    let state = Arc::new(AppState::new_for_testing().0);
    let shared = Arc::new(frame);
    let addr_in = shared.data().as_ptr() as usize;
    let extra_handle = keep_extra_handle.then(|| Arc::clone(&shared));

    render(&state, vec![stacked_frame(shared, passthrough_settings())]).await;
    drop(extra_handle);

    let addr_out = state
        .main_stream
        .get_latest_raw_frame()
        .await
        .expect("render task published a frame")
        .linear_frame
        .data()
        .as_ptr() as usize;
    (addr_in, addr_out)
}

/// The whole point of `StackedFrame` carrying an `Arc`: when the render task holds the
/// only handle it must reuse the buffer, not copy 50 MB.
#[tokio::test]
async fn test_render_task_reuses_uniquely_held_frame_buffer() {
    let (addr_in, addr_out) =
        render_and_report_buffer_addr(crate::frame::Frame::zeros(64, 48, 3).unwrap(), false).await;
    assert_eq!(addr_in, addr_out, "uniquely-held frame was copied instead of moved");
}

/// When another stage still holds the frame, the render task must copy rather than
/// mutate a buffer someone else is reading.
#[tokio::test]
async fn test_render_task_copies_frame_still_held_elsewhere() {
    let (addr_in, addr_out) =
        render_and_report_buffer_addr(crate::frame::Frame::zeros(64, 48, 3).unwrap(), true).await;
    assert_ne!(addr_in, addr_out, "shared frame must be copied before the pipeline mutates it");
}

/// With nobody watching, the frame is still published (a client arriving later needs it
/// and the counter must advance) but neither family is encoded.
#[tokio::test]
async fn an_unwatched_stream_publishes_the_frame_but_encodes_nothing() {
    let state = Arc::new(AppState::new_for_testing().0);

    render(&state, vec![stacked_frame(filled(10, 10), passthrough_settings())]).await;

    let counter = state.main_stream.frame_counter();
    assert_eq!(counter, 1, "the frame counter did not advance with no viewers");
    assert!(state.main_stream.get_latest_raw_frame().await.is_some());
    for kind in StreamKind::all() {
        assert!(state.main_stream.payload(kind, counter).is_none(), "{kind:?} encoded for nobody");
    }
}

/// Each family comes out at its own setting, read from the live settings.
#[tokio::test]
async fn each_family_is_published_at_its_setting() {
    let state = Arc::new(AppState::new_for_testing().0);
    let _jpeg = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
    let _lossless = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);

    for (streaming, eyepiece, jpeg_size, lossless_size) in [
        (Resolution::Qhd1440, EyepieceStreamResolution::Qhd1440, (1440, 1440), (1440, 1440)),
        (Resolution::Hd1080, EyepieceStreamResolution::Uhd2160, (1080, 1080), (2160, 2160)),
        (Resolution::Native, EyepieceStreamResolution::Native, (3008, 3008), (3008, 3008)),
        (Resolution::Uhd2160, EyepieceStreamResolution::Qhd1440, (2160, 2160), (1440, 1440)),
    ] {
        set_resolutions(&state, streaming, eyepiece).await;

        render(&state, vec![stacked_frame(filled(3008, 3008), passthrough_settings())]).await;

        assert_eq!(current_size(&state, StreamKind::Jpeg), jpeg_size, "{streaming:?}");
        assert_eq!(current_size(&state, StreamKind::Lossless), lossless_size, "{eyepiece:?}");
    }
}

/// A changed setting applies from the next frame on — never to a frame already published.
#[tokio::test]
async fn a_changed_setting_applies_from_the_next_frame() {
    let state = Arc::new(AppState::new_for_testing().0);
    let _jpeg = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
    let _lossless = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);

    render(&state, vec![stacked_frame(filled(3008, 3008), passthrough_settings())]).await;
    let first_jpeg = state.main_stream.payload(StreamKind::Jpeg, 1).unwrap();

    set_resolutions(&state, Resolution::Hd1080, EyepieceStreamResolution::Uhd2160).await;
    render_each(&state, vec![stacked_frame(filled(3008, 3008), passthrough_settings())]).await;

    assert_eq!(dimensions(&first_jpeg), (1440, 1440), "the published frame changed size");
    assert_eq!(current_size(&state, StreamKind::Jpeg), (1080, 1080));
    assert_eq!(current_size(&state, StreamKind::Lossless), (2160, 2160));
}

/// The frame's snapshot was taken when its exposure started, so a change made during the
/// exposure is only in the live settings — and that is what the encoders must follow.
#[tokio::test]
async fn the_live_setting_wins_over_the_frames_snapshot() {
    let state = Arc::new(AppState::new_for_testing().0);
    let _jpeg = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
    let _lossless = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);

    let mut snapshot = passthrough_settings();
    snapshot.streaming_resolution = Resolution::Native;
    snapshot.eyepiece.stream_resolution = EyepieceStreamResolution::Native;
    set_resolutions(&state, Resolution::Hd1080, EyepieceStreamResolution::Qhd1440).await;

    render(&state, vec![stacked_frame(filled(3008, 3008), snapshot)]).await;

    assert_eq!(current_size(&state, StreamKind::Jpeg), (1080, 1080));
    assert_eq!(current_size(&state, StreamKind::Lossless), (1440, 1440));
}

async fn set_resolutions(state: &AppState, streaming: Resolution, eyepiece: EyepieceStreamResolution) {
    let mut settings = state.settings.write().await;
    settings.streaming_resolution = streaming;
    settings.eyepiece.stream_resolution = eyepiece;
}

/// A viewer leaving mid-session stops its family being encoded on the very next frame,
/// while the other family carries on.
#[tokio::test]
async fn a_family_whose_last_viewer_left_is_no_longer_encoded() {
    let state = Arc::new(AppState::new_for_testing().0);
    let jpeg = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
    let _lossless = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);

    render(&state, vec![stacked_frame(filled(1600, 1200), passthrough_settings())]).await;
    assert!(state.main_stream.payload(StreamKind::Jpeg, 1).is_some());

    drop(jpeg);
    render(&state, vec![stacked_frame(filled(1600, 1200), passthrough_settings())]).await;
    assert!(state.main_stream.payload(StreamKind::Jpeg, 2).is_none());
    assert!(state.main_stream.payload(StreamKind::Lossless, 2).is_some());
}

/// A failed encode is a display problem: it reaches the UI as an error event, and never
/// counts towards the session's rejection rate — the signal that decides whether the
/// camera still responds. Failing again on the next frames is reported once, not per frame
/// (the UI re-raises every `error` event), and a recovery makes the next failure news again.
#[tokio::test]
async fn an_encode_failure_is_reported_once_and_is_not_a_camera_rejection() {
    let state = Arc::new(AppState::new_for_testing().0);
    let _jpeg = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Jpeg);
    let _lossless = ViewerGuard::new(Arc::clone(&state.main_stream), StreamKind::Lossless);
    let mut events = state.subscribe_events();
    let (frames_before, rejected_before) = {
        let session = state.session.read().await;
        (session.frame_count, session.rejected_count)
    };

    let unsupported = Arc::new(RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(crate::frame::Frame::filled(64, 64, 2, 0.25).unwrap()),
        pipeline_config: crate::render::RenderPipelineConfig::default(),
        stretch_result: None,
    });
    let supported = Arc::new(RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(crate::frame::Frame::filled(64, 64, 3, 0.25).unwrap()),
        pipeline_config: crate::render::RenderPipelineConfig {
            contrast: false,
            auto_stretch: false,
            saturation_boost: false,
            ..Default::default()
        },
        stretch_result: None,
    });
    let task_state = Arc::clone(&state);
    tokio::task::spawn_blocking(move || {
        let mut conversions = super::ConversionCache::default();
        let mut failures = super::FailureReports::default();
        let resolutions = super::StreamResolutions::of(&passthrough_settings());
        let frames = [&unsupported, &unsupported, &unsupported, &supported, &unsupported];
        for (counter, frame) in (1..).zip(frames) {
            conversions.begin_frame();
            super::encode_payloads(
                &task_state,
                frame,
                counter,
                resolutions,
                &mut conversions,
                &mut failures,
                1,
            );
        }
    })
    .await
    .unwrap();

    let session = state.session.read().await;
    assert_eq!(session.frame_count, frames_before);
    assert_eq!(session.rejected_count, rejected_before);
    assert!(!session.rejection_rate_exceeded());
    drop(session);

    let mut errors = Vec::new();
    while let Ok(event) = events.try_recv() {
        errors.push(event.to_json());
    }
    assert_eq!(
        errors.len(),
        4,
        "expected one error per family for the first failure and again after the recovery: {errors:#?}"
    );
    assert!(errors.iter().all(|e| e.contains("error")), "{errors:#?}");
}
