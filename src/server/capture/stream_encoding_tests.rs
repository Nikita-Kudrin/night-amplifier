use super::*;
use crate::server::state::{EyepieceStreamResolution, ViewerGuard};

fn ready_frame(width: usize, height: usize, channels: usize) -> RenderReadyFrame {
    RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(crate::frame::Frame::filled(width, height, channels, 0.25).unwrap()),
        pipeline_config: crate::render::RenderPipelineConfig {
            contrast: false,
            auto_stretch: false,
            saturation_boost: false,
            ..Default::default()
        },
        stretch_result: None,
    }
}

fn watch(stream: &Arc<FrameStream>, kind: StreamKind) -> ViewerGuard {
    ViewerGuard::new(Arc::clone(stream), kind)
}

/// Width and height out of an SA09 or SA10 header.
fn dimensions(payload: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        u32::from_le_bytes(payload[8..12].try_into().unwrap()),
    )
}

/// The DC step of an SA10 payload's first quantisation table, the luminance one.
fn luma_dc_step(payload: &[u8]) -> u8 {
    let jpeg = &payload[crate::server::encoding::SA10_HEADER_SIZE..];
    let dqt = jpeg.windows(2).position(|m| m == [0xFF, 0xDB]).expect("no DQT marker");
    // Marker, two length bytes, precision/table id, then the table in zigzag order.
    jpeg[dqt + 5]
}

/// libjpeg's scaling of the standard luminance DC step (16) for `quality` of 50 and up.
fn libjpeg_dc_step(quality: i32) -> u8 {
    ((16 * (200 - 2 * quality) + 50) / 100).clamp(1, 255) as u8
}

fn encode_both(
    stream: &FrameStream,
    frame: &RenderReadyFrame,
    counter: u64,
    conversions: &mut ConversionCache,
    streaming: Resolution,
    eyepiece: Resolution,
) {
    encode_lossless(stream, frame, counter, conversions, eyepiece, 2).unwrap();
    encode_jpeg(stream, frame, counter, conversions, streaming).unwrap();
}

/// The two settings are independent: each family comes out at its own size.
#[test]
fn each_family_is_encoded_at_its_own_resolution() {
    let stream = Arc::new(FrameStream::default());
    let _jpeg = watch(&stream, StreamKind::Jpeg);
    let _lossless = watch(&stream, StreamKind::Lossless);
    let frame = ready_frame(3008, 3008, 3);
    let mut conversions = ConversionCache::default();

    encode_both(&stream, &frame, 1, &mut conversions, Resolution::Hd1080, Resolution::Uhd2160);

    let jpeg = stream.payload(StreamKind::Jpeg, 1).unwrap();
    let lossless = stream.payload(StreamKind::Lossless, 1).unwrap();
    assert_eq!(dimensions(&jpeg), (1080, 1080));
    assert_eq!(dimensions(&lossless), (2160, 2160));
    assert_eq!(conversions.len(), 2);
}

/// The denoised conversion is ~5x the encode, so both families at one output size must
/// share it — including two different settings that resolve to the same size.
#[test]
fn families_at_the_same_output_size_share_one_conversion() {
    for (streaming, eyepiece, frame_size) in [
        (Resolution::Qhd1440, Resolution::Qhd1440, (3008, 3008)),
        // IMX464 fits inside the 4K box, so 4K and Native are the same size.
        (Resolution::Uhd2160, Resolution::Native, (2712, 1538)),
    ] {
        let stream = Arc::new(FrameStream::default());
        let _jpeg = watch(&stream, StreamKind::Jpeg);
        let _lossless = watch(&stream, StreamKind::Lossless);
        let frame = ready_frame(frame_size.0, frame_size.1, 3);
        let mut conversions = ConversionCache::default();

        encode_both(&stream, &frame, 1, &mut conversions, streaming, eyepiece);

        assert_eq!(conversions.len(), 1, "{streaming:?} + {eyepiece:?} converted twice");
        assert_eq!(
            dimensions(&stream.payload(StreamKind::Jpeg, 1).unwrap()),
            dimensions(&stream.payload(StreamKind::Lossless, 1).unwrap())
        );
    }
}

/// A family nobody watches costs nothing: no conversion, no payload.
#[test]
fn an_unwatched_family_is_neither_converted_nor_stored() {
    let stream = Arc::new(FrameStream::default());
    let frame = ready_frame(3008, 3008, 3);
    let mut conversions = ConversionCache::default();

    encode_both(&stream, &frame, 1, &mut conversions, Resolution::Qhd1440, Resolution::Qhd1440);
    assert_eq!(conversions.len(), 0);
    assert!(stream.payload(StreamKind::Jpeg, 1).is_none());
    assert!(stream.payload(StreamKind::Lossless, 1).is_none());

    let _lossless = watch(&stream, StreamKind::Lossless);
    encode_both(&stream, &frame, 2, &mut conversions, Resolution::Qhd1440, Resolution::Qhd1440);
    assert!(stream.payload(StreamKind::Jpeg, 2).is_none());
    assert!(stream.payload(StreamKind::Lossless, 2).is_some());
}

/// A resolution larger than the frame never upscales — e.g. Processing Resolution
/// binned a 3008² sensor to 1504² and the stream asks for 4K or Native.
#[test]
fn a_resolution_above_the_frame_sends_the_frame_as_is() {
    for resolution in [Resolution::Uhd2160, Resolution::Native, Resolution::Qhd1440] {
        let stream = Arc::new(FrameStream::default());
        let _jpeg = watch(&stream, StreamKind::Jpeg);
        let frame = ready_frame(1504, 1504, 3);

        encode_jpeg(&stream, &frame, 1, &mut ConversionCache::default(), resolution).unwrap();

        let expected = if resolution == Resolution::Qhd1440 { (1440, 1440) } else { (1504, 1504) };
        assert_eq!(dimensions(&stream.payload(StreamKind::Jpeg, 1).unwrap()), expected);
    }
}

/// Native means native for the lossless family too — no hidden 4K cap.
#[test]
fn native_lossless_is_not_capped() {
    let stream = Arc::new(FrameStream::default());
    let _lossless = watch(&stream, StreamKind::Lossless);
    let frame = ready_frame(4000, 3000, 3);

    encode_lossless(
        &stream,
        &frame,
        1,
        &mut ConversionCache::default(),
        EyepieceStreamResolution::Native.resolution(),
        4,
    )
    .unwrap();

    assert_eq!(dimensions(&stream.payload(StreamKind::Lossless, 1).unwrap()), (4000, 3000));
}

/// A failed conversion is reported with the family and resolution, and stores nothing.
#[test]
fn a_failed_conversion_is_reported_and_publishes_nothing() {
    let stream = Arc::new(FrameStream::default());
    let _jpeg = watch(&stream, StreamKind::Jpeg);
    let _lossless = watch(&stream, StreamKind::Lossless);
    let unsupported = ready_frame(64, 64, 2);
    let mut conversions = ConversionCache::default();

    let lossless = encode_lossless(&stream, &unsupported, 1, &mut conversions, Resolution::Qhd1440, 1)
        .expect_err("a failed lossless conversion was not reported");
    assert!(lossless.contains("lossless") && lossless.contains("1440p"), "{lossless}");
    let jpeg = encode_jpeg(&stream, &unsupported, 1, &mut conversions, Resolution::Native)
        .expect_err("a failed JPEG conversion was not reported");
    assert!(jpeg.contains("jpeg") && jpeg.contains("Native"), "{jpeg}");

    assert!(stream.payload(StreamKind::Lossless, 1).is_none());
    assert!(stream.payload(StreamKind::Jpeg, 1).is_none());
}

/// Equivalent boxes are one conversion; a box that genuinely shrinks the frame is
/// another, and asking for it again is free.
#[test]
fn conversion_cache_shares_one_buffer_across_equivalent_boxes() {
    let frame = ready_frame(2712, 1538, 3);
    let mut cache = ConversionCache::default();

    let uhd = cache.get(&frame, 3840, 2160).expect("conversion");
    let native = cache.get(&frame, u32::MAX, u32::MAX).expect("conversion");
    assert_eq!(cache.len(), 1, "equivalent boxes converted twice");
    assert!(Arc::ptr_eq(&uhd, &native));

    let hd = cache.get(&frame, 1920, 1080).expect("conversion");
    assert_eq!(cache.len(), 2);
    assert_ne!((hd.1, hd.2), (uhd.1, uhd.2));

    cache.get(&frame, 1920, 1080).expect("conversion");
    assert_eq!(cache.len(), 2);
}

/// A failure repeating frame after frame is reported once per family; success or a
/// different failure makes the next one worth reporting.
#[test]
fn a_repeated_failure_is_reported_once_until_it_changes_or_clears() {
    let mut failures = FailureReports::default();
    let broken = || Err::<(), _>("conversion failed".to_owned());

    assert_eq!(failures.to_report(StreamKind::Jpeg, broken()).as_deref(), Some("conversion failed"));
    assert_eq!(failures.to_report(StreamKind::Jpeg, broken()), None);
    assert_eq!(
        failures.to_report(StreamKind::Lossless, broken()).as_deref(),
        Some("conversion failed"),
        "families share a report"
    );

    let other = Err("encoding failed".to_owned());
    assert_eq!(failures.to_report(StreamKind::Jpeg, other).as_deref(), Some("encoding failed"));

    assert_eq!(failures.to_report(StreamKind::Jpeg, Ok(())), None);
    assert_eq!(failures.to_report(StreamKind::Jpeg, broken()).as_deref(), Some("conversion failed"));
}

/// The quality follows the denoisers on both ways a JPEG leaves the server — the render
/// task's encode and a first-frame encode for a client joining an unwatched family — or
/// two clients of one family would be sent different bytes.
#[test]
fn a_denoised_frame_is_streamed_at_the_denoised_quality_on_both_paths() {
    let (box_w, box_h) = Resolution::Qhd1440.bounding_box();
    assert_ne!(
        libjpeg_dc_step(jpeg_quality_at_1440(false)),
        libjpeg_dc_step(jpeg_quality_at_1440(true)),
        "the two qualities must be told apart at 1440p, or this test checks nothing"
    );
    for denoised in [false, true] {
        let mut frame = ready_frame(3008, 3008, 3);
        if denoised {
            frame.pipeline_config.denoise.chroma = crate::render::ChromaDenoiseConfig {
                enabled: true,
                radius: 8,
                strength: 1.0,
                ..crate::render::ChromaDenoiseConfig::OFF
            };
        }
        assert_eq!(frame.pipeline_config.denoise.is_enabled(), denoised);
        let stream = Arc::new(FrameStream::default());
        let _jpeg = watch(&stream, StreamKind::Jpeg);
        encode_jpeg(&stream, &frame, 1, &mut ConversionCache::default(), Resolution::Qhd1440).unwrap();
        let streamed = stream.payload(StreamKind::Jpeg, 1).unwrap();
        let on_demand = crate::server::encoding::encode_rgb8_jpeg_bounded(&frame, box_w, box_h).unwrap();

        let expected = libjpeg_dc_step(jpeg_quality_at_1440(denoised));
        assert_eq!(dimensions(&streamed), (1440, 1440));
        assert_eq!(luma_dc_step(&streamed), expected, "render task, denoised {denoised}");
        assert_eq!(luma_dc_step(&on_demand), expected, "first-frame encode, denoised {denoised}");
    }
}

fn jpeg_quality_at_1440(denoised: bool) -> i32 {
    crate::server::encoding::jpeg_quality(1440, 1440, denoised)
}
