//! Cross-encoder colour-identity tests for the planar [`Frame`] layout. `Frame` is
//! planar; every 8-bit output format is interleaved. Each test pushes a frame with
//! three constant, *distinct* channels through one output path and asserts the order
//! — a path reading planar as interleaved produces three adjacent samples of one
//! channel per output pixel, which every assertion here rejects.
//!
//! Kept as one table of paths on purpose: the failure is systemic, so a new output
//! format means a new row here. Covers `to_rgb8`/`to_rgb8_fast`/`write_rgb8_into`,
//! `render::frame_to_rgb8`, `render_to_rgb8`, the fused encoder (and its downsampling
//! sibling), JPEG/LZ4/PNG, SER `Rgb`/`Bgr`/`Mono`, FITS (both directions),
//! `Frame::downsample`, `warp_frame_into`, and both debayer traversals.

use super::Frame;
use crate::frame::PixelFormat;

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

fn assert_frame_is_tricolour(frame: &Frame, ctx: &str) {
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            let got = (
                frame.get_pixel(x, y, 0),
                frame.get_pixel(x, y, 1),
                frame.get_pixel(x, y, 2),
            );
            assert!(
                (got.0 - R_VAL).abs() < 0.01
                    && (got.1 - G_VAL).abs() < 0.01
                    && (got.2 - B_VAL).abs() < 0.01,
                "{ctx}: pixel ({x}, {y}) is {got:?}, expected ({R_VAL}, {G_VAL}, {B_VAL})"
            );
        }
    }
}

#[test]
fn to_rgb8_fast_is_interleaved() {
    let frame = tricolour_frame(W, H);
    assert_interleaved_rgb8(
        &frame.to_rgb8_fast(),
        W,
        H,
        (R_U8, G_U8, B_U8),
        "to_rgb8_fast",
    );
}

#[test]
fn to_rgb8_is_interleaved() {
    let frame = tricolour_frame(W, H);
    assert_interleaved_rgb8(&frame.to_rgb8(), W, H, (R_U8, G_U8, B_U8), "to_rgb8");
}

/// Mono stays one byte per pixel; callers tag the result as greyscale rather than
/// replicating it. (The old name said "replicates", which is the opposite of what this
/// asserts and of what the function does.)
#[test]
fn to_rgb8_fast_keeps_mono_single_byte() {
    let frame = Frame::filled(W, H, 1, G_VAL).unwrap();
    let rgb8 = frame.to_rgb8_fast();
    assert_eq!(
        rgb8.len(),
        W * H,
        "mono to_rgb8_fast should stay 1 byte per pixel"
    );
    assert!(rgb8.iter().all(|&v| v == G_U8));
}

/// `write_rgb8_into` must produce exactly what `to_rgb8_fast` allocates, since callers
/// with a pooled buffer use it precisely to avoid keeping a second copy of the gather.
#[test]
fn write_rgb8_into_matches_to_rgb8_fast() {
    for frame in [
        tricolour_frame(W, H),
        Frame::filled(W, H, 1, G_VAL).unwrap(),
    ] {
        let want = frame.to_rgb8_fast();
        let mut got = vec![0u8; want.len()];
        frame.write_rgb8_into(&mut got);
        assert_eq!(got, want, "channels = {}", frame.channels());
    }
}

#[test]
fn frame_to_rgb8_is_interleaved() {
    let frame = tricolour_frame(W, H);
    let rgb8 = crate::render::frame_to_rgb8_simple(&frame).unwrap();
    assert_interleaved_rgb8(&rgb8, W, H, (R_U8, G_U8, B_U8), "render::frame_to_rgb8");
}

#[test]
fn render_to_rgb8_is_interleaved() {
    let frame = tricolour_frame(W, H);
    let rgb8 = crate::render::render_to_rgb8(&frame).unwrap();
    assert_interleaved_rgb8(&rgb8, W, H, (R_U8, G_U8, B_U8), "render_to_rgb8");
}

#[test]
fn expand_to_rgb8_fused_is_interleaved() {
    let frame = tricolour_frame(W, H);
    let mut config = crate::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = false;
    config.saturation_boost = false;
    let ready = crate::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config: config,
        stretch_result: None,
    };
    let (rgb8, w, h) =
        crate::server::encoding::frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();
    assert_interleaved_rgb8(
        &rgb8,
        w as usize,
        h as usize,
        (R_U8, G_U8, B_U8),
        "expand_to_rgb8_fused",
    );
}

/// The *downsampling* half of the streaming encoder — the Hd1080 and Qhd1440 tiers,
/// i.e. what most clients actually receive.
///
/// `expand_to_rgb8_fused_is_interleaved` above only reaches `expand_to_rgb8_fused`,
/// because its fixture already fits the bounding box. `box_downsample_to_rgb8_fused`
/// is a separate traversal with its own planar indexing (`plane_size + idx`), and the
/// only integration test that touched it asserted on a uniform grey frame, which is
/// layout-invariant by construction. A source larger than the box is what makes the
/// downsampling branch run at all.
#[test]
fn box_downsample_to_rgb8_fused_is_interleaved() {
    let ready = passthrough_ready(tricolour_frame(64, 32));
    let (rgb8, w, h) = crate::server::encoding::frame_to_rgb8_downsampled(&ready, 32, 16).unwrap();

    assert!(
        (w as usize) < 64,
        "fixture did not take the downsampling branch: got {w}x{h}"
    );
    // Every source pixel is the same colour, so box-averaging must reproduce it exactly.
    assert_interleaved_rgb8(
        &rgb8,
        w as usize,
        h as usize,
        (R_U8, G_U8, B_U8),
        "box_downsample_to_rgb8_fused",
    );
}

/// Builds the `RenderReadyFrame` the streaming encoders take, with every optional
/// stage off so the only thing under test is the layout.
fn passthrough_ready(frame: Frame) -> crate::server::state::RenderReadyFrame {
    let mut config = crate::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = false;
    config.saturation_boost = false;
    crate::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config: config,
        stretch_result: None,
    }
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
    let payload = crate::server::encoding::encode_rgb8_jpeg_bounded(&ready, 3840, 2160).unwrap();

    assert_eq!(
        u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        crate::server::encoding::JPEG_MAGIC
    );
    let width = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    assert_eq!((width, height), (64, 32));

    let jpeg = &payload[crate::server::encoding::SA10_HEADER_SIZE..];
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
        crate::server::encoding::encode_rgb8_lz4_chunked(&ready, CHUNKS, 3840, 2160).unwrap();

    assert_eq!(
        u32::from_le_bytes(payload[0..4].try_into().unwrap()),
        crate::server::encoding::RGB8_CHUNKED_MAGIC
    );
    let width = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
    let chunk_count = u32::from_le_bytes(payload[16..20].try_into().unwrap()) as usize;
    assert_eq!((width, height), (W, H));
    assert_eq!(chunk_count, CHUNKS);

    let desc_size = crate::server::encoding::SA09_CHUNK_DESCRIPTOR_SIZE;
    let mut desc = crate::server::encoding::SA09_HEADER_SIZE;
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

#[test]
fn png_preview_is_interleaved() {
    let frame = tricolour_frame(W, H);
    let rgb8 = frame.to_rgb8_fast();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.png");
    crate::disk_writer::write_rgb8_png(&rgb8, W as u32, H as u32, &path).unwrap();

    let file = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_interleaved_rgb8(
        &buf[..info.buffer_size()],
        W,
        H,
        (R_U8, G_U8, B_U8),
        "write_rgb8_png",
    );
}

/// SER frame data starts after a fixed 178-byte header.
const SER_HEADER_SIZE: usize = 178;

/// Writes one tricolour frame at `bit_depth` and returns the payload's samples,
/// widened to `u32` so 8-bit and 16-bit share one set of assertions. Asserts on bytes,
/// not a read-back round trip — writer and reader can be wrong in mutually inverse
/// ways a round trip can't see, and SER's on-disk layout *is* the contract (consumed
/// by AutoStakkert, PIPP, Registax). Parameterised over bit depth because
/// `encode_8bit`/`encode_16bit` are separate traversals with their own plane gathers;
/// only 16-bit used to be covered, though 8-bit is reachable (`disk_writer::worker`
/// picks it for a session's first `Raw8`/`Rgb24` frame).
fn write_ser_and_read_samples(
    color_id: crate::ser::SerColorId,
    bit_depth: u32,
    name: &str,
) -> (Vec<u32>, Frame) {
    use crate::ser::{SerHeader, SerReader, SerWriter};

    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);

    let mut writer = SerWriter::create(
        &path,
        SerHeader::new(W as u32, H as u32, color_id, bit_depth),
    )
    .unwrap();
    writer.write_frame(&frame, None).unwrap();
    writer.finalize().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let channels = color_id.channels();
    let samples = read_ser_samples(&bytes, W * H * channels, bit_depth);

    let mut reader = SerReader::open(&path).unwrap();
    let round_tripped = reader.read_frame(0).unwrap();
    (samples, round_tripped)
}

/// Decodes `count` payload samples of `bit_depth` bits from a SER file's bytes.
fn read_ser_samples(file: &[u8], count: usize, bit_depth: u32) -> Vec<u32> {
    let bytes_per_sample = if bit_depth <= 8 { 1 } else { 2 };
    let payload = &file[SER_HEADER_SIZE..SER_HEADER_SIZE + count * bytes_per_sample];
    if bytes_per_sample == 1 {
        return payload.iter().map(|&b| b as u32).collect();
    }
    payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
        .collect()
}

/// The expected on-disk sample for a normalised value at a given bit depth.
///
/// 8-bit goes through the canonical `sample_to_u8` rounding — the same one PNG, JPEG and
/// LZ4 use — because SER now shares it rather than truncating on its own.
fn as_ser_sample(v: f32, bit_depth: u32) -> u32 {
    if bit_depth <= 8 {
        return crate::frame::sample_to_u8(v) as u32;
    }
    (v.clamp(0.0, 1.0) * 65535.0) as u32
}

/// SER `Mono` from a colour source is a Rec. 709 combine across the three planes, so it
/// reads all three and is exactly as exposed to the layout as `Rgb`/`Bgr` are.
#[test]
fn ser_mono_payload_is_per_pixel_luminance() {
    for bit_depth in [8u32, 16] {
        let (samples, _) = write_ser_and_read_samples(
            crate::ser::SerColorId::Mono,
            bit_depth,
            &format!("mono{bit_depth}.ser"),
        );

        // Derived from the fixture rather than hardcoded, so the expectation stays
        // legible: one pixel's own three channels, not three neighbouring samples of one
        // plane.
        let want = as_ser_sample(0.2126 * R_VAL + 0.7152 * G_VAL + 0.0722 * B_VAL, bit_depth);
        assert_eq!(samples.len(), W * H, "{bit_depth}-bit sample count");
        for (i, &got) in samples.iter().enumerate() {
            assert!(
                got.abs_diff(want) <= 1,
                "SER Mono {bit_depth}-bit sample {i} is {got}, expected ~{want}"
            );
        }
    }
}

#[test]
fn ser_rgb_payload_is_interleaved_and_round_trips() {
    for bit_depth in [8u32, 16] {
        let (samples, back) = write_ser_and_read_samples(
            crate::ser::SerColorId::Rgb,
            bit_depth,
            &format!("rgb{bit_depth}.ser"),
        );

        let want = (
            as_ser_sample(R_VAL, bit_depth),
            as_ser_sample(G_VAL, bit_depth),
            as_ser_sample(B_VAL, bit_depth),
        );
        for i in 0..(W * H) {
            let got = (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2]);
            assert_eq!(
                got, want,
                "SER Rgb {bit_depth}-bit on-disk pixel {i} is {got:?} — payload must be interleaved RGB"
            );
        }

        assert_frame_is_tricolour(&back, &format!("SER Rgb {bit_depth}-bit round trip"));
    }
}

#[test]
fn ser_bgr_payload_is_interleaved_and_round_trips() {
    for bit_depth in [8u32, 16] {
        let (samples, back) = write_ser_and_read_samples(
            crate::ser::SerColorId::Bgr,
            bit_depth,
            &format!("bgr{bit_depth}.ser"),
        );

        let want = (
            as_ser_sample(B_VAL, bit_depth),
            as_ser_sample(G_VAL, bit_depth),
            as_ser_sample(R_VAL, bit_depth),
        );
        for i in 0..(W * H) {
            let got = (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2]);
            assert_eq!(
                got, want,
                "SER Bgr {bit_depth}-bit on-disk pixel {i} is {got:?} — payload must be interleaved BGR"
            );
        }

        assert_frame_is_tricolour(&back, &format!("SER Bgr {bit_depth}-bit round trip"));
    }
}

/// The 8-bit SER payload must be byte-identical to what every other 8-bit output writes
/// for the same frame. It used to truncate while the rest rounded, so a mid-grey channel
/// came out one LSB darker in SER than in the PNG beside it.
#[test]
fn ser_8bit_samples_match_the_canonical_8bit_conversion() {
    let (samples, _) = write_ser_and_read_samples(crate::ser::SerColorId::Rgb, 8, "rounding.ser");

    let frame = tricolour_frame(W, H);
    let png_bytes = frame.to_rgb8_fast();

    assert_eq!(samples.len(), png_bytes.len());
    for (i, (&ser, &png)) in samples.iter().zip(png_bytes.iter()).enumerate() {
        assert_eq!(
            ser, png as u32,
            "sample {i}: SER wrote {ser}, the shared 8-bit conversion writes {png}"
        );
    }
    // G_VAL = 0.5 is the value that separates the two conversions: 127 truncated, 128
    // rounded. Assert it explicitly so the test cannot pass on a fixture that happens to
    // avoid the boundary.
    assert_eq!(
        samples[1], G_U8 as u32,
        "0.5 must round to {G_U8}, not truncate"
    );
}

#[test]
fn fits_f32_round_trip_preserves_channels() {
    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stack.fits");

    crate::fits::write_fits(&frame, &path, None).unwrap();

    // FITS stores colour as NAXIS3=3, i.e. plane-major — the same order as `Frame`.
    // `write_rgb_fits` appends the image as an extension HDU, so the primary is empty.
    let mut f = fitsio::FitsFile::open(path.to_str().unwrap()).unwrap();
    let hdu = f.hdu(1).unwrap();
    let data: Vec<f32> = hdu.read_image(&mut f).unwrap();
    assert_eq!(data.len(), W * H * 3);

    let area = W * H;
    for i in 0..area {
        assert!(
            (data[i] - R_VAL).abs() < 1e-4,
            "FITS f32 R plane at {i}: {}",
            data[i]
        );
        assert!(
            (data[area + i] - G_VAL).abs() < 1e-4,
            "FITS f32 G plane at {i}: {}",
            data[area + i]
        );
        assert!(
            (data[2 * area + i] - B_VAL).abs() < 1e-4,
            "FITS f32 B plane at {i}: {}",
            data[2 * area + i]
        );
    }
}

#[test]
fn fits_u16_round_trip_preserves_channels() {
    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("raw.fits");

    crate::fits::write_fits_u16(&frame, &path, None).unwrap();

    let mut f = fitsio::FitsFile::open(path.to_str().unwrap()).unwrap();
    let hdu = f.primary_hdu().unwrap();
    let data: Vec<u16> = hdu.read_image(&mut f).unwrap();
    assert_eq!(data.len(), W * H * 3);

    let area = W * H;
    let expect = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
    for i in 0..area {
        assert_eq!(data[i], expect(R_VAL), "FITS u16 R plane at {i}");
        assert_eq!(data[area + i], expect(G_VAL), "FITS u16 G plane at {i}");
        assert_eq!(data[2 * area + i], expect(B_VAL), "FITS u16 B plane at {i}");
    }
}

/// `warp_frame_into` writes into a caller-owned buffer and is the variant the stacker
/// actually runs per frame; `warp_frame` is the one that had a per-channel test. They
/// are separate functions with separate plane dispatch, so both need a row.
///
/// An identity transform means every output pixel interpolates from its own source
/// pixel, so a constant-plane fixture must come back untouched — including the borders,
/// which take the `border_value` path only when the source coordinate leaves the frame.
#[test]
fn warp_frame_into_preserves_channel_identity() {
    use crate::registration::AffineTransform;

    let frame = tricolour_frame(32, 24);
    let mut output = Frame::zeros(32, 24, 3).unwrap();
    crate::stacking::warp_frame_into(&frame, &AffineTransform::identity(), &mut output, 0.0)
        .unwrap();

    // The warp treats a source coordinate as in-bounds only while `sx < width - 2`
    // (bilinear needs `x0 + 1` to exist), so the last two columns and rows take the
    // border-value path by design and are excluded here rather than asserted on.
    let (w, h) = (output.width(), output.height());
    for y in 1..h - 2 {
        for x in 1..w - 2 {
            let got = (
                output.get_pixel(x, y, 0),
                output.get_pixel(x, y, 1),
                output.get_pixel(x, y, 2),
            );
            assert!(
                (got.0 - R_VAL).abs() < 1e-4
                    && (got.1 - G_VAL).abs() < 1e-4
                    && (got.2 - B_VAL).abs() < 1e-4,
                "warp_frame_into: pixel ({x}, {y}) is {got:?}, expected \
                 ({R_VAL}, {G_VAL}, {B_VAL})"
            );
        }
    }
}

/// The debayer-straight-to-8-bit path used by the streaming encoder for undebayered
/// frames. Its f32 sibling writes three planes; this one writes interleaved bytes, so
/// the two cannot share a row.
///
/// A constant-plane CFA source: every site carries its own channel's level, so bilinear
/// interpolation must reproduce the three constants exactly across the interior.
#[test]
fn debayer_to_rgb8_is_interleaved() {
    use crate::debayer::CfaPattern;

    let (w, h) = (32usize, 16usize);
    for pattern in CfaPattern::all() {
        let mut cfa = Frame::zeros(w, h, 1).unwrap();
        for y in 0..h {
            for x in 0..w {
                let level = match pattern.color_at(x, y) {
                    0 => R_VAL,
                    2 => B_VAL,
                    _ => G_VAL,
                };
                cfa.set_pixel(x, y, 0, level);
            }
        }

        let rgb8 = crate::debayer::debayer_bilinear_to_rgb8_fast(&cfa, pattern).unwrap();
        assert_eq!(rgb8.len(), w * h * 3);

        // Interior only: border pixels clamp their neighbour fetches, which biases the
        // interpolation at the edge even on a constant source.
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let i = (y * w + x) * 3;
                let got = (rgb8[i], rgb8[i + 1], rgb8[i + 2]);
                assert_eq!(
                    got,
                    (R_U8, G_U8, B_U8),
                    "{pattern:?} at ({x}, {y}) is {got:?}, expected \
                     ({R_U8}, {G_U8}, {B_U8})"
                );
            }
        }
    }
}

/// Superpixel debayering is a separate traversal from the two interpolating
/// kernels: it walks quads rather than rows of neighbours, gathers four samples
/// per output pixel, and writes into a half-size planar frame. A gap in its
/// coverage does not show up through the bilinear row.
#[test]
fn superpixel_debayer_lands_in_planes() {
    use crate::debayer::{CfaPattern, DebayerAlgorithm, DebayerConfig, Debayerer};

    let (w, h) = (32usize, 16usize);
    for pattern in CfaPattern::all() {
        let mut cfa = Frame::zeros(w, h, 1).unwrap();
        for y in 0..h {
            for x in 0..w {
                let level = match pattern.color_at(x, y) {
                    0 => R_VAL,
                    2 => B_VAL,
                    _ => G_VAL,
                };
                cfa.set_pixel(x, y, 0, level);
            }
        }

        let out = Debayerer::new(
            DebayerConfig::new(pattern).with_algorithm(DebayerAlgorithm::Superpixel),
        )
        .debayer(&cfa)
        .unwrap();

        assert_eq!(
            (out.width(), out.height(), out.channels()),
            (w / 2, h / 2, 3)
        );
        // No interpolation and no border case: every output pixel is exact.
        assert_frame_is_tricolour(&out, &format!("superpixel {pattern:?}"));
    }
}

#[test]
fn from_raw_interleaved_lands_in_planes() {
    // Rgb8: an interleaved input buffer must be split into planes.
    let raw: Vec<u8> = (0..(W * H)).flat_map(|_| [R_U8, G_U8, B_U8]).collect();
    let frame = Frame::from_raw(&raw, W, H, 3, PixelFormat::Rgb8).unwrap();
    assert_frame_is_tricolour(&frame, "from_raw Rgb8");

    // Rgb16 little-endian.
    let raw16: Vec<u8> = (0..(W * H))
        .flat_map(|_| {
            [
                ((R_VAL * 65535.0) as u16).to_le_bytes(),
                ((G_VAL * 65535.0) as u16).to_le_bytes(),
                ((B_VAL * 65535.0) as u16).to_le_bytes(),
            ]
        })
        .flatten()
        .collect();
    let frame = Frame::from_raw(&raw16, W, H, 3, PixelFormat::Rgb16).unwrap();
    assert_frame_is_tricolour(&frame, "from_raw Rgb16");

    // Rgb16 big-endian.
    let raw16be: Vec<u8> = (0..(W * H))
        .flat_map(|_| {
            [
                ((R_VAL * 65535.0) as u16).to_be_bytes(),
                ((G_VAL * 65535.0) as u16).to_be_bytes(),
                ((B_VAL * 65535.0) as u16).to_be_bytes(),
            ]
        })
        .flatten()
        .collect();
    let frame = Frame::from_raw(&raw16be, W, H, 3, PixelFormat::Rgb16Be).unwrap();
    assert_frame_is_tricolour(&frame, "from_raw Rgb16Be");
}

#[test]
fn downsample_preserves_channel_identity() {
    let frame = tricolour_frame(W, H);
    let small = frame.downsample(2).unwrap();
    assert_eq!((small.width(), small.height()), (W / 2, H / 2));
    assert_frame_is_tricolour(&small, "downsample(2)");
}

/// The production FITS loader must read back what the FITS writer produced.
///
/// Verifies the claim that `interleave_planar` in the integer arms is a redundant
/// planar -> interleaved -> planar round trip rather than a corruption: `from_raw`
/// de-interleaves, so the two conversions cancel. If they ever stop cancelling, this
/// is the test that says so.
#[test]
fn production_fits_loader_round_trips_u16() {
    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip_u16.fits");

    crate::fits::write_fits_u16(&frame, &path, None).unwrap();
    let back = crate::fits::read_frame(&path).unwrap();

    assert_eq!(back.channels(), 3, "loader lost the colour planes");
    assert_frame_is_tricolour(&back, "write_fits_u16 -> production load_fits");
}

#[test]
fn production_fits_loader_round_trips_f32() {
    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip_f32.fits");

    crate::fits::write_fits(&frame, &path, None).unwrap();
    let back = crate::fits::read_frame(&path).unwrap();

    assert_eq!(back.channels(), 3, "loader lost the colour planes");
    assert_frame_is_tricolour(&back, "write_fits -> production load_fits");
}

/// The display transform (black floor + ordered dither) rewrote the tail of
/// every 8-bit conversion, so it has to be swept for layout too: a channel swap
/// inside `write_row_rgb8` would be invisible to the transform's own unit tests,
/// which use symmetric grey inputs.
///
/// Both fused traversals are covered separately, per the rule at the top of this
/// file — they gather planes independently, so a gap in one does not show up via
/// the other.
#[test]
fn display_transform_preserves_channel_order_in_both_fused_kernels() {
    let display = crate::render::DisplayOutput::default()
        .with_pedestal(0.04)
        .with_dither(true);

    // Pedestal maps [0, 1] onto [pedestal, 1]; the dither can move a sample by
    // at most one level either way.
    let lift = |v: u8| {
        let x = 0.04 + (v as f32 / 255.0) * 0.96;
        (x * 255.0 + 0.5) as u8
    };
    let expect = (lift(R_U8), lift(G_U8), lift(B_U8));

    let frame = tricolour_frame(W, H);
    let mut ready = passthrough_ready(frame.clone());
    ready.pipeline_config.display = display;

    // Expand traversal: frame fits the box, so it is sent at native size.
    let (expanded, w, h) =
        crate::server::encoding::frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();
    assert_eq!((w as usize, h as usize), (W, H));
    assert_interleaved_rgb8_within(
        &expanded,
        W,
        H,
        expect,
        1,
        "expand_to_rgb8_fused + display transform",
    );

    // Downsample traversal: a constant frame box-averages to the same colour,
    // so the expected values are unchanged and any channel mixing shows up.
    let big = tricolour_frame(W * 4, H * 4);
    let mut ready_big = passthrough_ready(big);
    ready_big.pipeline_config.display = display;
    let (reduced, rw, rh) =
        crate::server::encoding::frame_to_rgb8_downsampled(&ready_big, W as u32, H as u32).unwrap();
    assert_interleaved_rgb8_within(
        &reduced,
        rw as usize,
        rh as usize,
        expect,
        1,
        "box_downsample_to_rgb8_fused + display transform",
    );
}

/// The staged traversal Tier 2's denoisers introduced: with denoising on, both
/// kernels stop transforming a thread-local scratch row and instead resample the
/// whole frame into one interleaved f32 buffer, filter it, then run the tone curve
/// and 8-bit write. A third path across the planar/interleaved boundary — the gather
/// is shared with the fused drivers, but the row the tail/writer see is reached
/// differently, and the denoisers split it into YCbCr and back. A channel swap there
/// is invisible to the filters' own unit tests, which use grey inputs. Both sources
/// get a row, per the rule at the top of this file.
#[test]
fn denoised_staged_traversal_preserves_channel_order_in_both_sources() {
    let denoise = crate::render::DenoiseConfig {
        luma: crate::render::LumaDenoiseConfig::default(),
        chroma: crate::render::ChromaDenoiseConfig::default(),
    };

    // A constant frame has no detail at any scale and no chroma structure, so
    // neither filter may change it — which makes an exact sweep the right
    // assertion and any channel mixing immediately visible.
    let mut ready = passthrough_ready(tricolour_frame(W, H));
    ready.pipeline_config.denoise = denoise;
    let (expanded, w, h) =
        crate::server::encoding::frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();
    assert_eq!((w as usize, h as usize), (W, H));
    assert_interleaved_rgb8_within(
        &expanded,
        W,
        H,
        (R_U8, G_U8, B_U8),
        1,
        "expand source + denoise",
    );

    let mut ready_big = passthrough_ready(tricolour_frame(W * 4, H * 4));
    ready_big.pipeline_config.denoise = denoise;
    let (reduced, rw, rh) =
        crate::server::encoding::frame_to_rgb8_downsampled(&ready_big, W as u32, H as u32).unwrap();
    assert!((rw as usize) < W * 4, "fixture did not downsample");
    assert_interleaved_rgb8_within(
        &reduced,
        rw as usize,
        rh as usize,
        (R_U8, G_U8, B_U8),
        1,
        "downsample source + denoise",
    );
}

/// Like [`assert_interleaved_rgb8`], but with a tolerance: ordered dithering
/// moves individual samples by up to one level by design, so an exact sweep
/// would fail on a correct implementation.
fn assert_interleaved_rgb8_within(
    rgb8: &[u8],
    width: usize,
    height: usize,
    expect: (u8, u8, u8),
    tolerance: i32,
    ctx: &str,
) {
    let close = |got: u8, want: u8| (got as i32 - want as i32).abs() <= tolerance;
    for i in 0..(width * height) {
        let got = (rgb8[i * 3], rgb8[i * 3 + 1], rgb8[i * 3 + 2]);
        assert!(
            close(got.0, expect.0) && close(got.1, expect.1) && close(got.2, expect.2),
            "{ctx}: pixel {i} is {got:?}, expected {expect:?} +/-{tolerance} — channels \
             are interleaved wrongly (planar buffer read as interleaved?)"
        );
    }
}
