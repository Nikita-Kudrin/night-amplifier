//! The streaming encoders' rows of the planar-layout table in the core crate's
//! `frame/layout_tests.rs`: `Frame` is planar, the wire is interleaved RGB, and a path
//! reading one as the other collapses three distinct channels into one.

use night_amplifier_core::frame::Frame;

const W: usize = 16;
const H: usize = 8;

const R_VAL: f32 = 0.0;
const G_VAL: f32 = 0.5;
const B_VAL: f32 = 1.0;

/// 8-bit encodings of [`R_VAL`], [`G_VAL`], [`B_VAL`] under `sample_to_u8`.
const R_U8: u8 = 0;
const G_U8: u8 = 128;
const B_U8: u8 = 255;

/// A frame whose channels are constant and mutually distinct.
///
/// Built with `set_pixel` so the fixture cannot encode a layout assumption.
fn tricolour_frame(width: usize, height: usize) -> Frame {
    let mut frame = Frame::zeros(width, height, 3).unwrap();
    for y in 0..height {
        for x in 0..width {
            frame.set_pixel(x, y, 0, R_VAL);
            frame.set_pixel(x, y, 1, G_VAL);
            frame.set_pixel(x, y, 2, B_VAL);
        }
    }
    frame
}

/// Asserts every pixel of an interleaved RGB8 buffer is `(r, g, b)`.
fn assert_interleaved_rgb8(
    rgb8: &[u8],
    width: usize,
    height: usize,
    expect: (u8, u8, u8),
    ctx: &str,
) {
    assert_eq!(rgb8.len(), width * height * 3, "{ctx}: wrong buffer length");
    for i in 0..(width * height) {
        let got = (rgb8[i * 3], rgb8[i * 3 + 1], rgb8[i * 3 + 2]);
        assert_eq!(
            got, expect,
            "{ctx}: pixel {i} is {got:?}, expected {expect:?} — channels are \
             interleaved wrongly (planar buffer read as interleaved?)"
        );
    }
}

/// The `RenderReadyFrame` the streaming encoders take, every optional stage off so the
/// only thing under test is the layout.
fn passthrough_ready(frame: Frame) -> night_amplifier_core::render::display::RenderReadyFrame {
    night_amplifier_core::render::display::testing::to_ready_frame(&frame)
}

/// JPEG (SA10) carries interleaved RGB.
///
/// Tolerance rather than equality: TurboJPEG runs at quality 95 with `Sub2x2` chroma
/// subsampling, so exact bytes are not preserved. Channel *identity* is what this
/// guards — a planar buffer read as interleaved collapses all three channels toward one
/// grey value, which no tolerance this tight would hide.
#[test]
fn jpeg_sa10_payload_is_interleaved() {
    let ready = passthrough_ready(tricolour_frame(64, 32));
    let payload = super::encode_rgb8_jpeg_bounded(&ready, 3840, 2160).unwrap();

    assert_eq!(
        u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        super::JPEG_MAGIC
    );
    let width = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    assert_eq!((width, height), (64, 32));

    let jpeg = &payload[super::SA10_HEADER_SIZE..];
    let decoded: image::RgbImage =
        image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .expect("SA10 payload is not decodable JPEG")
            .to_rgb8();
    assert_eq!(
        (decoded.width() as usize, decoded.height() as usize),
        (width, height)
    );

    for (x, y, px) in decoded.enumerate_pixels() {
        let [r, g, b] = px.0;
        assert!(
            (r as i32 - R_U8 as i32).abs() <= 8
                && (g as i32 - G_U8 as i32).abs() <= 8
                && (b as i32 - B_U8 as i32).abs() <= 8,
            "JPEG pixel ({x}, {y}) is {:?}, expected ~({R_U8}, {G_U8}, {B_U8})",
            px.0
        );
    }
}

/// Chunked LZ4 (SA09) carries interleaved RGB, and every stripe round-trips.
#[test]
fn lz4_sa09_payload_is_interleaved() {
    const CHUNKS: usize = 4;
    let ready = passthrough_ready(tricolour_frame(W, H));
    let payload =
        super::encode_rgb8_lz4_chunked(&ready, CHUNKS, 3840, 2160).unwrap();

    assert_eq!(
        u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        super::RGB8_CHUNKED_MAGIC
    );
    let width = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let chunk_count = u32::from_le_bytes(payload[16..20].try_into().unwrap()) as usize;
    assert_eq!((width, height), (W, H));
    assert_eq!(chunk_count, CHUNKS);

    let desc_size = super::SA09_CHUNK_DESCRIPTOR_SIZE;
    let mut desc = super::SA09_HEADER_SIZE;
    let mut data = desc + chunk_count * desc_size;
    let mut rgb8 = Vec::with_capacity(width * height * 3);

    for _ in 0..chunk_count {
        let compressed_size =
            u32::from_le_bytes(payload[desc..desc + 4].try_into().unwrap()) as usize;
        let decompressed_size =
            u32::from_le_bytes(payload[desc + 4..desc + 8].try_into().unwrap()) as usize;
        desc += desc_size;

        let stripe =
            lz4_flex::decompress(&payload[data..data + compressed_size], decompressed_size)
                .expect("SA09 stripe did not decompress");
        data += compressed_size;
        rgb8.extend_from_slice(&stripe);
    }

    assert_interleaved_rgb8(
        &rgb8,
        width,
        height,
        (R_U8, G_U8, B_U8),
        "encode_rgb8_lz4_chunked",
    );
}
