use super::*;
use crate::frame::Frame;
use crate::server::state::{EyepieceStreamResolution, RenderReadyFrame};

/// A frame the encoder can run on without a stretch solve behind it.
fn ready_frame(width: usize, height: usize) -> Arc<RenderReadyFrame> {
    let config = crate::render::RenderPipelineConfig {
        contrast: false,
        auto_stretch: false,
        saturation_boost: false,
        ..Default::default()
    };
    Arc::new(RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(Frame::filled(width, height, 3, 0.25).unwrap()),
        pipeline_config: config,
        stretch_result: None,
    })
}

/// Width and height out of an SA09 or SA10 header; both put them at the same offsets.
fn dimensions(payload: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        u32::from_le_bytes(payload[8..12].try_into().unwrap()),
    )
}

fn magic(payload: &[u8]) -> u32 {
    u32::from_le_bytes(payload[0..4].try_into().unwrap())
}

fn test_state() -> Arc<AppState> {
    Arc::new(AppState::new_for_testing().0)
}

async fn with_frame(state: &AppState, width: usize, height: usize) -> u64 {
    state.main_stream.set_latest_raw_frame(ready_frame(width, height)).await;
    let counter = state.main_stream.begin_frame();
    state.main_stream.publish_frame();
    counter
}

/// The producer skips a family nobody watches, so the first client of a family has
/// nothing to show. Without an on-demand encode the view stays black until the next
/// exposure — a minute, at 60 s subs.
#[tokio::test]
async fn a_first_client_is_served_from_the_raw_frame_at_the_configured_size() {
    let state = test_state();
    let counter = with_frame(&state, 3008, 3008).await;
    let stream = Arc::clone(&state.main_stream);

    for (kind, expected_magic) in [
        (StreamKind::Jpeg, crate::server::encoding::JPEG_MAGIC),
        (StreamKind::Lossless, crate::server::encoding::RGB8_CHUNKED_MAGIC),
    ] {
        assert!(stream.payload(kind, counter).is_none());
        let (tag, payload) = payload_for_client(&state, &stream, kind)
            .await
            .unwrap_or_else(|| panic!("a connecting {kind:?} client was left with nothing"));

        assert_eq!(tag, counter);
        assert_eq!(magic(&payload), expected_magic);
        // Both settings default to 1440p.
        assert_eq!(dimensions(&payload), (1440, 1440));
        assert!(
            stream.payload(kind, counter).is_some(),
            "the on-demand {kind:?} payload was not stored for the next client"
        );
    }
}

/// Each family follows its own setting, read when the client arrives.
#[tokio::test]
async fn on_demand_encodes_follow_each_familys_setting() {
    let state = test_state();
    {
        let mut settings = state.settings.write().await;
        settings.streaming_resolution = Resolution::Hd1080;
        settings.eyepiece.stream_resolution = EyepieceStreamResolution::Native;
    }
    with_frame(&state, 3008, 3008).await;
    let stream = Arc::clone(&state.main_stream);

    let (_, jpeg) = payload_for_client(&state, &stream, StreamKind::Jpeg).await.unwrap();
    let (_, lossless) = payload_for_client(&state, &stream, StreamKind::Lossless).await.unwrap();
    assert_eq!(dimensions(&jpeg), (1080, 1080));
    assert_eq!(dimensions(&lossless), (3008, 3008));
}

/// A client that connects after capture stopped still gets the last frame.
#[tokio::test]
async fn a_client_arriving_after_capture_stopped_still_gets_the_last_frame() {
    let state = test_state();
    with_frame(&state, 320, 240).await;
    for kind in StreamKind::all() {
        assert!(payload_for_client(&state, &state.main_stream, kind).await.is_some());
    }
}

/// A payload tagged with an older counter is from a previous frame or session, and
/// serving it as current would show last night's target.
#[tokio::test]
async fn a_stale_payload_is_re_encoded_rather_than_served() {
    let state = test_state();
    let stale = state.main_stream.begin_frame();
    state.main_stream.set_payload(StreamKind::Lossless, stale, vec![0xde; 64]);
    let current = with_frame(&state, 400, 300).await;

    let (tag, payload) = payload_for_client(&state, &state.main_stream, StreamKind::Lossless)
        .await
        .expect("no payload");

    assert_eq!(tag, current, "the stale payload's counter was served as current");
    assert_ne!(payload.as_ref(), &[0xde; 64][..]);
    assert_eq!(dimensions(&payload), (400, 300));
}

/// A current payload is reused as is — whatever size it was encoded at. After a setting
/// change that is the old size until the next frame, the same frame every connected
/// client is looking at.
#[tokio::test]
async fn a_current_payload_is_reused_until_the_next_frame() {
    let state = test_state();
    let counter = with_frame(&state, 400, 300).await;
    state.main_stream.set_payload(StreamKind::Jpeg, counter, vec![0xab; 64]);
    state.settings.write().await.streaming_resolution = Resolution::Hd1080;

    let (tag, payload) = payload_for_client(&state, &state.main_stream, StreamKind::Jpeg)
        .await
        .expect("no payload");

    assert_eq!(tag, counter);
    assert_eq!(payload.as_ref(), &[0xab; 64][..]);
}

/// Clients arriving together share one on-demand encode: every one of them gets the very
/// same allocation, not merely equal bytes from separate conversions.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_first_clients_share_one_encode() {
    let state = test_state();
    with_frame(&state, 3008, 3008).await;

    for kind in StreamKind::all() {
        let arrivals = (0..8).map(|_| payload_for_client(&state, &state.main_stream, kind));
        let payloads: Vec<_> = futures_util::future::join_all(arrivals)
            .await
            .into_iter()
            .map(|p| p.expect("an arriving client got nothing").1)
            .collect();

        let first = payloads[0].as_ptr();
        assert!(
            payloads.iter().all(|p| p.as_ptr() == first),
            "simultaneous {kind:?} clients encoded the frame more than once"
        );
    }
}

/// On-demand encodes of the two families do not wait on each other.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn families_encode_on_demand_independently() {
    let state = test_state();
    with_frame(&state, 1200, 900).await;

    let _holding_jpeg = state.main_stream.on_demand_encode_lock(StreamKind::Jpeg).lock().await;
    let lossless = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        payload_for_client(&state, &state.main_stream, StreamKind::Lossless),
    )
    .await
    .expect("a lossless client waited on the JPEG family's encode");
    assert!(lossless.is_some());
}

/// With no rendered frame there is nothing to encode, and the handler must simply wait
/// rather than send an empty message.
#[tokio::test]
async fn nothing_to_encode_yields_nothing() {
    let state = test_state();
    for kind in StreamKind::all() {
        assert!(payload_for_client(&state, &state.main_stream, kind).await.is_none());
    }
}

#[tokio::test]
async fn configured_resolution_reads_each_familys_setting() {
    let state = test_state();
    assert_eq!(configured_resolution(&state, StreamKind::Jpeg).await, Resolution::Qhd1440);
    assert_eq!(configured_resolution(&state, StreamKind::Lossless).await, Resolution::Qhd1440);

    {
        let mut settings = state.settings.write().await;
        settings.streaming_resolution = Resolution::Native;
        settings.eyepiece.stream_resolution = EyepieceStreamResolution::Uhd2160;
    }
    assert_eq!(configured_resolution(&state, StreamKind::Jpeg).await, Resolution::Native);
    assert_eq!(configured_resolution(&state, StreamKind::Lossless).await, Resolution::Uhd2160);
}

#[test]
fn endpoints_map_to_their_page_socket_and_family() {
    use StreamEndpoint::*;
    assert_eq!((LiveView.page(), LiveView.socket_path()), ("/", "/ws/stream"));
    assert_eq!((Eyepiece.page(), Eyepiece.socket_path()), ("/eyepiece", "/ws/eyepiece"));
    assert_eq!(
        (EyepieceQuality.page(), EyepieceQuality.socket_path()),
        ("/eyepiece_quality", "/ws/eyepiece_quality")
    );
    assert_eq!(LiveView.kind(), StreamKind::Jpeg);
    assert_eq!(Eyepiece.kind(), StreamKind::Jpeg);
    assert_eq!(EyepieceQuality.kind(), StreamKind::Lossless);
}

#[tokio::test]
async fn the_log_describes_the_output_size_or_its_absence() {
    let empty = FrameStream::default();
    assert_eq!(describe_output(&empty, Resolution::Qhd1440).await, "no frame yet");

    let state = test_state();
    with_frame(&state, 3008, 3008).await;
    assert_eq!(
        describe_output(&state.main_stream, Resolution::Qhd1440).await,
        "1440x1440 (frame 3008x3008)"
    );
    assert_eq!(
        describe_output(&state.main_stream, Resolution::Native).await,
        "3008x3008 (frame 3008x3008)"
    );
}
