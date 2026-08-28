//! Cross-encoder colour-identity tests for the planar [`Frame`] layout.
//!
//! `Frame` is planar (`channel * width * height + y * width + x`); every 8-bit
//! output format is interleaved. Each test below pushes a frame whose three
//! channels are constant and *distinct* through one output path and asserts the
//! channels come out in the right order. A path that reads the planar buffer as
//! interleaved produces three adjacent samples of one channel per output pixel,
//! which every assertion here rejects.
//!
//! Kept as one table of paths on purpose: the failure these catch is systemic, so
//! adding a new output format should mean adding a row here. Rows currently cover
//! `to_rgb8`/`to_rgb8_fast`/`write_rgb8_into`, `render::frame_to_rgb8`,
//! `render_to_rgb8`, the fused encoder expansion *and* its downsampling sibling,
//! JPEG (SA10), chunked LZ4 (SA09), PNG, SER `Rgb`/`Bgr`/`Mono`, FITS f32/u16 (both
//! directions), `Frame::downsample`, `warp_frame_into` and the debayer-to-RGB8 path.

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
    let payload = crate::server::encoding::encode_rgb8_lz4_chunked(&ready, CHUNKS).unwrap();

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
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preview.png");
    crate::disk_writer::write_png(&frame, &path).unwrap();

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
        "write_png",
    );
}

/// SER frame data starts after a fixed 178-byte header.
const SER_HEADER_SIZE: usize = 178;

/// Writes one tricolour frame and returns the raw 16-bit samples of the frame payload.
///
/// Asserting on the bytes rather than on a read-back round trip is deliberate: the
/// writer and reader can be wrong in mutually inverse ways, which a round trip
/// cannot see. SER is consumed by third-party tools (AutoStakkert, PIPP, Registax),
/// so the on-disk layout *is* the contract.
fn write_ser_and_read_samples(color_id: crate::ser::SerColorId, name: &str) -> (Vec<u16>, Frame) {
    use crate::ser::{SerHeader, SerReader, SerWriter};

    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);

    let mut writer =
        SerWriter::create(&path, SerHeader::new(W as u32, H as u32, color_id, 16)).unwrap();
    writer.write_frame(&frame, None).unwrap();
    writer.finalize().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let payload = &bytes[SER_HEADER_SIZE..SER_HEADER_SIZE + W * H * 3 * 2];
    let samples: Vec<u16> = payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut reader = SerReader::open(&path).unwrap();
    let round_tripped = reader.read_frame(0).unwrap();
    (samples, round_tripped)
}

/// SER `Mono` from a colour source is a Rec. 709 combine across the three planes, so it
/// reads all three and is exactly as exposed to the layout as `Rgb`/`Bgr` are — but it
/// had no row here.
#[test]
fn ser_mono_payload_is_per_pixel_luminance() {
    use crate::ser::{SerColorId, SerHeader, SerWriter};

    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mono.ser");

    let mut writer = SerWriter::create(
        &path,
        SerHeader::new(W as u32, H as u32, SerColorId::Mono, 16),
    )
    .unwrap();
    writer.write_frame(&frame, None).unwrap();
    writer.finalize().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let payload = &bytes[SER_HEADER_SIZE..SER_HEADER_SIZE + W * H * 2];
    let samples: Vec<u16> = payload
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Derived from the fixture rather than hardcoded, so the expectation stays legible:
    // one pixel's own three channels, not three neighbouring samples of one plane.
    let want = as_u16(0.2126 * R_VAL + 0.7152 * G_VAL + 0.0722 * B_VAL);
    assert_eq!(samples.len(), W * H);
    for (i, &got) in samples.iter().enumerate() {
        assert!(
            got.abs_diff(want) <= 1,
            "SER Mono sample {i} is {got}, expected ~{want}"
        );
    }
}

fn as_u16(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 65535.0) as u16
}

#[test]
fn ser_rgb_payload_is_interleaved_and_round_trips() {
    let (samples, back) = write_ser_and_read_samples(crate::ser::SerColorId::Rgb, "rgb.ser");

    for i in 0..(W * H) {
        let got = (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2]);
        assert_eq!(
            got,
            (as_u16(R_VAL), as_u16(G_VAL), as_u16(B_VAL)),
            "SER Rgb on-disk pixel {i} is {got:?} — payload must be interleaved RGB"
        );
    }

    assert_frame_is_tricolour(&back, "SER Rgb round trip");
}

#[test]
fn ser_bgr_payload_is_interleaved_and_round_trips() {
    let (samples, back) = write_ser_and_read_samples(crate::ser::SerColorId::Bgr, "bgr.ser");

    for i in 0..(W * H) {
        let got = (samples[i * 3], samples[i * 3 + 1], samples[i * 3 + 2]);
        assert_eq!(
            got,
            (as_u16(B_VAL), as_u16(G_VAL), as_u16(R_VAL)),
            "SER Bgr on-disk pixel {i} is {got:?} — payload must be interleaved BGR"
        );
    }

    assert_frame_is_tricolour(&back, "SER Bgr round trip");
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
    let back = crate::camera::simulated::loaders::fits::load_fits(&path).unwrap();

    assert_eq!(back.channels(), 3, "loader lost the colour planes");
    assert_frame_is_tricolour(&back, "write_fits_u16 -> production load_fits");
}

#[test]
fn production_fits_loader_round_trips_f32() {
    let frame = tricolour_frame(W, H);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip_f32.fits");

    crate::fits::write_fits(&frame, &path, None).unwrap();
    let back = crate::camera::simulated::loaders::fits::load_fits(&path).unwrap();

    assert_eq!(back.channels(), 3, "loader lost the colour planes");
    assert_frame_is_tricolour(&back, "write_fits -> production load_fits");
}
