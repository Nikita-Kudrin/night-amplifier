use crate::frame::Frame;

use crate::server::encoding::format::*;
use crate::server::encoding::jpeg::calculate_dynamic_jpeg_quality;
use crate::server::encoding::jpeg::*;
use crate::server::encoding::lz4::*;
use crate::server::state::CaptureSettings;
use crate::server::StretchResult;
use std::sync::Arc;

fn to_ready_frame(frame: &Frame) -> crate::server::state::RenderReadyFrame {
    let mut config = crate::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = false;
    config.saturation_boost = false;
    crate::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame.clone()),
        pipeline_config: config,
        stretch_result: None,
    }
}

/// Like `to_ready_frame`, but with `auto_stretch` actually enabled and a real
/// `StretchResult` attached — every fused-kernel test up to this point runs with
/// stretch/saturation/contrast all disabled, so the scale-LUT application branch in
/// `expand_to_rgb8_fused`/`box_downsample_to_rgb8_fused` had no coverage at all.
fn to_ready_frame_with_stretch(
    frame: &Frame,
    black_point: f32,
    scale_lut: std::sync::Arc<Vec<f32>>,
) -> crate::server::state::RenderReadyFrame {
    let mut config = crate::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = true;
    config.saturation_boost = false;
    crate::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame.clone()),
        pipeline_config: config,
        stretch_result: Some(crate::server::state::StretchResult {
            black_point,
            scale_lut,
            color_intensity: 1.0,
        }),
    }
}

use super::*;

#[test]
fn test_rgb8_lz4_encode_header_format() {
    let frame = Frame::filled(2, 2, 3, 0.5).unwrap();
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();

    assert!(encoded.len() >= 16);
    let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
    assert_eq!(magic, RGB8_MAGIC);
    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    assert_eq!(width, 2);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert_eq!(height, 2);
    let compressed_size = u32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]);
    assert_eq!(compressed_size as usize, encoded.len() - 16);
}

#[test]
fn test_rgb8_lz4_encode_decode_roundtrip() {
    use lz4_flex::decompress_size_prepended;
    let mut frame = Frame::zeros(4, 4, 3).unwrap();
    frame.set_pixel(0, 0, 0, 1.0);
    frame.set_pixel(1, 1, 1, 0.5);
    frame.set_pixel(2, 2, 2, 0.25);

    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();
    let compressed_data = &encoded[16..];
    let decompressed = decompress_size_prepended(compressed_data).unwrap();

    // 4x4 pixels * 3 bytes per pixel
    assert_eq!(decompressed.len(), 4 * 4 * 3);

    // Pixel (0,0): R=255, G=0, B=0
    assert_eq!(decompressed[0], 255); // R
    assert_eq!(decompressed[1], 0); // G
    assert_eq!(decompressed[2], 0); // B

    // Pixel (1,1) offset = (1*4 + 1) * 3 = 15
    let offset_1_1 = (1 * 4 + 1) * 3;
    assert_eq!(decompressed[offset_1_1], 0); // R
                                             // G should be ~128 (0.5 * 255 + 0.5 = 128)
    assert!((decompressed[offset_1_1 + 1] as i32 - 128).abs() <= 1);
    assert_eq!(decompressed[offset_1_1 + 2], 0); // B

    // Pixel (2,2) offset = (2*4 + 2) * 3 = 30
    let offset_2_2 = (2 * 4 + 2) * 3;
    assert_eq!(decompressed[offset_2_2], 0); // R
    assert_eq!(decompressed[offset_2_2 + 1], 0); // G
                                                 // B should be ~64 (0.25 * 255 + 0.5 = 64)
    assert!((decompressed[offset_2_2 + 2] as i32 - 64).abs() <= 1);
}

#[test]
fn test_rgb8_lz4_compression_ratio() {
    let frame = Frame::filled(100, 100, 3, 0.01).unwrap();
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();

    let raw_size = 100 * 100 * 3;
    let compressed_size = encoded.len() - 16;
    assert!(compressed_size < raw_size / 2);
}

#[test]
fn test_rgb8_lz4_various_frame_sizes() {
    let test_cases = [(1, 1), (10, 10), (100, 50), (1920, 1080)];
    for (width, height) in test_cases {
        let frame = Frame::zeros(width, height, 3).unwrap();
        let encoded = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();
        let enc_width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        let enc_height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        assert_eq!(enc_width, width as u32);
        assert_eq!(enc_height, height as u32);
    }
}

#[test]
fn test_rgb8_lz4_grayscale_to_rgb_conversion() {
    use lz4_flex::decompress_size_prepended;
    let frame = Frame::filled(8, 8, 1, 0.5).unwrap();
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert_eq!(width, 8);
    assert_eq!(height, 8);

    let compressed_data = &encoded[16..];
    let decompressed = decompress_size_prepended(compressed_data).unwrap();
    assert_eq!(decompressed.len(), 8 * 8 * 3);

    // Center pixel (4,4) offset = (4*8 + 4) * 3 = 108
    let center_offset = (4 * 8 + 4) * 3;
    let r = decompressed[center_offset];
    let g = decompressed[center_offset + 1];
    let b = decompressed[center_offset + 2];

    // 0.5 * 255 + 0.5 = 128
    let expected_value: i32 = 128;
    assert!((r as i32 - expected_value).abs() <= 1);
    assert_eq!(r, g);
    assert_eq!(g, b);
}

// --- SA09 Chunked Format Tests ---

/// Decode a SA09 chunked message back to raw RGB8 for test verification
fn decode_sa09(encoded: &[u8]) -> (u32, u32, Vec<u8>) {
    assert!(encoded.len() >= SA09_HEADER_SIZE);
    let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
    assert_eq!(magic, RGB8_CHUNKED_MAGIC);

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    let chunk_count =
        u32::from_le_bytes([encoded[16], encoded[17], encoded[18], encoded[19]]) as usize;

    let descriptors_size = chunk_count * SA09_CHUNK_DESCRIPTOR_SIZE;
    let mut decompressed = Vec::new();
    let mut data_offset = SA09_HEADER_SIZE + descriptors_size;

    for i in 0..chunk_count {
        let desc_offset = SA09_HEADER_SIZE + i * SA09_CHUNK_DESCRIPTOR_SIZE;
        let compressed_size = u32::from_le_bytes([
            encoded[desc_offset],
            encoded[desc_offset + 1],
            encoded[desc_offset + 2],
            encoded[desc_offset + 3],
        ]) as usize;
        let decompressed_size = u32::from_le_bytes([
            encoded[desc_offset + 4],
            encoded[desc_offset + 5],
            encoded[desc_offset + 6],
            encoded[desc_offset + 7],
        ]) as usize;

        let chunk_data = &encoded[data_offset..data_offset + compressed_size];
        let mut chunk_out = vec![0u8; decompressed_size];
        lz4_flex::decompress_into(chunk_data, &mut chunk_out).unwrap();
        decompressed.extend_from_slice(&chunk_out);
        data_offset += compressed_size;
    }

    (width, height, decompressed)
}

#[test]
fn test_sa09_header_format() {
    let frame = Frame::filled(4, 4, 3, 0.5).unwrap();
    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 2).unwrap();

    assert!(encoded.len() >= SA09_HEADER_SIZE);
    let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
    assert_eq!(magic, RGB8_CHUNKED_MAGIC);

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    assert_eq!(width, 4);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert_eq!(height, 4);

    let chunk_count = u32::from_le_bytes([encoded[16], encoded[17], encoded[18], encoded[19]]);
    assert_eq!(chunk_count, 2);
}

#[test]
fn test_sa09_roundtrip() {
    let mut frame = Frame::zeros(8, 8, 3).unwrap();
    frame.set_pixel(0, 0, 0, 1.0);
    frame.set_pixel(3, 3, 1, 0.5);
    frame.set_pixel(7, 7, 2, 0.25);

    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 4).unwrap();
    let (width, height, decompressed) = decode_sa09(&encoded);

    assert_eq!(width, 8);
    assert_eq!(height, 8);
    assert_eq!(decompressed.len(), 8 * 8 * 3);

    // Pixel (0,0): R=255
    assert_eq!(decompressed[0], 255);
    assert_eq!(decompressed[1], 0);
    assert_eq!(decompressed[2], 0);

    // Pixel (3,3): G~128
    let offset_3_3 = (3 * 8 + 3) * 3;
    assert!((decompressed[offset_3_3 + 1] as i32 - 128).abs() <= 1);

    // Pixel (7,7): B~64
    let offset_7_7 = (7 * 8 + 7) * 3;
    assert!((decompressed[offset_7_7 + 2] as i32 - 64).abs() <= 1);
}

#[test]
fn test_sa09_single_chunk() {
    let frame = Frame::filled(10, 10, 3, 0.3).unwrap();
    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 1).unwrap();

    let chunk_count = u32::from_le_bytes([encoded[16], encoded[17], encoded[18], encoded[19]]);
    assert_eq!(chunk_count, 1);

    let (_, _, decompressed) = decode_sa09(&encoded);
    assert_eq!(decompressed.len(), 10 * 10 * 3);

    let expected = (0.3_f32 * 255.0 + 0.5) as u8;
    assert!((decompressed[0] as i32 - expected as i32).abs() <= 1);
}

#[test]
fn test_sa09_various_chunk_counts() {
    let frame = Frame::filled(100, 100, 3, 0.42).unwrap();

    for chunks in [1, 2, 3, 4, 7, 8] {
        let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), chunks).unwrap();
        let (w, h, decompressed) = decode_sa09(&encoded);
        assert_eq!(w, 100);
        assert_eq!(h, 100);
        assert_eq!(decompressed.len(), 100 * 100 * 3);

        let expected = (0.42_f32 * 255.0 + 0.5) as u8;
        assert!((decompressed[0] as i32 - expected as i32).abs() <= 1);
    }
}

#[test]
fn test_sa09_matches_sa08_pixel_data() {
    use lz4_flex::decompress_size_prepended;

    let frame = Frame::filled(20, 20, 3, 0.7).unwrap();

    let sa08 = encode_rgb8_lz4(&to_ready_frame(&frame)).unwrap();
    let sa08_pixels = decompress_size_prepended(&sa08[16..]).unwrap();

    let sa09 = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 4).unwrap();
    let (_, _, sa09_pixels) = decode_sa09(&sa09);

    assert_eq!(sa08_pixels, sa09_pixels);
}

#[test]
fn test_calculate_dynamic_jpeg_quality() {
    // 1080p (smallest side 1080) -> < 1440, should be 95
    assert_eq!(calculate_dynamic_jpeg_quality(1920, 1080), 95);
    // Small (640x480) -> < 1440, should be 95
    assert_eq!(calculate_dynamic_jpeg_quality(640, 480), 95);
    // 1440p (2560x1440) -> >= 1440, should be 90
    assert_eq!(calculate_dynamic_jpeg_quality(2560, 1440), 90);
    // 4K (3840x2160) -> >= 1440, should be 90
    assert_eq!(calculate_dynamic_jpeg_quality(3840, 2160), 90);
    // Odd portrait orientation
    assert_eq!(calculate_dynamic_jpeg_quality(1080, 1920), 95);
    assert_eq!(calculate_dynamic_jpeg_quality(2160, 3840), 90);
}

#[test]
fn test_jpeg_encode_4k_clamp() {
    // Mock a massive 5000x5000 frame
    let frame = Frame::zeros(5000, 5000, 3).unwrap();
    // Request 5000x5000
    let encoded =
        encode_rgb8_jpeg_dynamic(&to_ready_frame(&frame), Some(5000), Some(5000)).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);

    // It should clamp to the 4K bounding box (3840x2160).
    // Since aspect ratio is 1:1, fitting into 3840x2160 means 2160x2160.
    assert_eq!(width, 2160);
    assert_eq!(height, 2160);
}

#[test]
fn test_jpeg_encode_1080p_clamp() {
    // Mock a 2000x2000 frame
    let frame = Frame::zeros(2000, 2000, 3).unwrap();
    // Request a tiny 640x480 stream
    let encoded = encode_rgb8_jpeg_dynamic(&to_ready_frame(&frame), Some(640), Some(480)).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);

    // It should clamp up to the 1080p bounding box (1920x1080).
    // Since aspect ratio is 1:1, fitting into 1920x1080 means 1080x1080.
    assert_eq!(width, 1080);
    assert_eq!(height, 1080);
}

#[test]
fn test_clamp_client_resolution() {
    assert_eq!(clamp_client_resolution(None, None), (1920, 1080));
    assert_eq!(clamp_client_resolution(Some(1280), Some(720)), (1920, 1080));
    assert_eq!(
        clamp_client_resolution(Some(2560), Some(1440)),
        (2560, 1440)
    );
    assert_eq!(
        clamp_client_resolution(Some(5000), Some(3000)),
        (3840, 2160)
    );
}

#[test]
fn test_jpeg_encode_bounded_keeps_native_resolution() {
    let frame = Frame::zeros(200, 120, 3).unwrap();
    let encoded = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), u32::MAX, u32::MAX).unwrap();

    let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
    assert_eq!(magic, JPEG_MAGIC);
    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert_eq!(width, 200);
    assert_eq!(height, 120);

    let payload_size = u32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]);
    assert_eq!(payload_size as usize, encoded.len() - SA10_HEADER_SIZE);
}

#[test]
fn test_jpeg_encode_bounded_downsamples_to_box() {
    let frame = Frame::zeros(2712, 1538, 3).unwrap();
    let encoded = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), 1920, 1080).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert!(width <= 1920 && height <= 1080);
    assert_eq!(height, 1080);
}

/// The thread-local compressor is reused across calls, so settings from one
/// encode must not leak into the next. 1500x1500 native encodes at quality
/// 90 while the 1080p-boxed encode uses 95.
#[test]
fn test_jpeg_encode_reused_compressor_does_not_leak_quality() {
    let frame = Frame::filled(1500, 1500, 3, 0.4).unwrap();

    let boxed = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), 1920, 1080).unwrap();
    let native = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), u32::MAX, u32::MAX).unwrap();
    let boxed_again = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), 1920, 1080).unwrap();

    assert_eq!(boxed, boxed_again);
    assert_ne!(boxed.len(), native.len());
}

// ==================================================================
// frame_to_rgb8 — the fused box downsample
//
// The pre-existing tests here feed `Frame::zeros` or a uniform fill, which
// average to themselves whatever the weights are, so none of them can see
// an arithmetic error. These use non-uniform data and an independent
// reference.
// ==================================================================

/// Distinct value per (x, y, channel), non-separable so a row/column swap or
/// a channel mix-up cannot survive. Values stay inside [0, 1].
fn gradient_frame(width: usize, height: usize, channels: usize) -> Frame {
    let mut data = vec![0.0f32; width * height * channels];
    for y in 0..height {
        for x in 0..width {
            for c in 0..channels {
                let idx = (y * width + x) * channels + c;
                let v = (x * 7 + y * 13 + c * 71) % 251;
                data[idx] = v as f32 / 250.0;
            }
        }
    }
    Frame::from_f32_vec(data, width, height, channels).unwrap()
}

/// The pre-rewrite algorithm: box-average into f32 with a real division, then
/// convert. This is the "current implementation" the ±1 LSB bound is against.
fn reference_downsample_to_rgb8(frame: &Frame, target_w: usize, target_h: usize) -> Vec<u8> {
    let (w, h, channels) = (frame.width(), frame.height(), frame.channels());
    let x_scale = w as f32 / target_w as f32;
    let y_scale = h as f32 / target_h as f32;
    let mut out = vec![0u8; target_w * target_h * 3];

    for y in 0..target_h {
        let sy0 = (y as f32 * y_scale) as usize;
        let sy1 = (((y + 1) as f32 * y_scale) as usize).min(h);
        let y_count = (sy1 - sy0).max(1);
        for x in 0..target_w {
            let sx0 = (x as f32 * x_scale) as usize;
            let sx1 = (((x + 1) as f32 * x_scale) as usize).min(w);
            let x_count = (sx1 - sx0).max(1);
            let area = (y_count * x_count) as f32;

            let mut avg = [0.0f32; 3];
            for c in 0..channels {
                let mut sum = 0.0f32;
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        sum += frame.get_pixel(sx, sy, c);
                    }
                }
                avg[c] = sum / area;
            }
            if channels == 1 {
                avg[1] = avg[0];
                avg[2] = avg[0];
            }

            let idx = (y * target_w + x) * 3;
            for c in 0..3 {
                out[idx + c] = (avg[c].max(0.0).min(1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// The bound the plan asks for: the fused kernel replaced `sum / area` with
/// `sum * (1/y_count) * (1/x_count)`, so results may differ by a rounding
/// step but must never differ visibly.
#[test]
fn test_downsample_matches_reference_within_1_lsb() {
    // Non-integer scale factors so `x_count`/`y_count` vary across the row and
    // the area divisor is exercised at more than one value.
    for (w, h, box_w, box_h) in [
        (400, 300, 137, 111),
        (271, 153, 96, 54),
        (300, 400, 111, 137),
    ] {
        let frame = gradient_frame(w, h, 3);
        let (got, gw, gh) =
            frame_to_rgb8_downsampled(&to_ready_frame(&frame), box_w, box_h).unwrap();
        let want = reference_downsample_to_rgb8(&frame, gw as usize, gh as usize);

        assert_eq!(got.len(), want.len(), "{w}x{h} -> {box_w}x{box_h}");

        // Guard against a vacuous pass: two all-zero buffers also agree.
        let distinct: std::collections::HashSet<u8> = got.iter().copied().collect();
        assert!(
            distinct.len() > 16,
            "{w}x{h} -> {box_w}x{box_h}: output has only {} distinct values, \
                 the comparison is not exercising anything",
            distinct.len()
        );

        let mut differing = 0usize;
        for (i, (&g, &r)) in got.iter().zip(&want).enumerate() {
            let delta = (g as i32 - r as i32).abs();
            assert!(
                delta <= 1,
                "{w}x{h} -> {box_w}x{box_h}: sample {i} differs by {delta} ({g} vs {r})"
            );
            if delta != 0 {
                differing += 1;
            }
        }
        // Informational: the reciprocal is exact whenever both counts are
        // powers of two, so most samples match bit for bit.
        println!(
            "{w}x{h} -> {gw}x{gh}: {differing}/{} samples differ by 1",
            got.len()
        );
    }
}

/// Regression guard for the mono fix. A 1-channel frame here is genuine
/// monochrome — every provider debayers colour at capture — so it must come
/// out grey. The previous code ran `detect_cfa_pattern` (which never fails)
/// and debayered, which tinted grey data and allocated a full-resolution f32
/// RGB frame to do it.
#[test]
fn test_mono_frame_downsamples_to_grey_not_false_colour() {
    let frame = gradient_frame(400, 300, 1);
    let (rgb, w, h) = frame_to_rgb8_downsampled(&to_ready_frame(&frame), 137, 111).unwrap();

    assert_eq!(rgb.len(), w as usize * h as usize * 3);
    for (i, px) in rgb.chunks_exact(3).enumerate() {
        assert_eq!(
            (px[0], px[1], px[2]),
            (px[0], px[0], px[0]),
            "pixel {i} is not grey: {px:?}"
        );
    }

    let want = reference_downsample_to_rgb8(&frame, w as usize, h as usize);
    for (&g, &r) in rgb.iter().zip(&want) {
        assert!((g as i32 - r as i32).abs() <= 1);
    }
}

/// Same property on the no-downsample path, which had the identical defect.
#[test]
fn test_mono_frame_stays_grey_at_native_size() {
    let frame = gradient_frame(64, 48, 1);
    let (rgb, w, h) = frame_to_rgb8_downsampled(&to_ready_frame(&frame), 1920, 1080).unwrap();

    assert_eq!((w, h), (64, 48));
    assert_eq!(rgb.len(), 64 * 48 * 3);
    for (px, &src) in rgb.chunks_exact(3).zip(frame.data()) {
        let expected = (src.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
        assert_eq!(px, [expected, expected, expected]);
    }
}

/// `par_chunks_mut` over output rows must not make the result depend on how
/// rayon splits the work.
#[test]
fn test_downsample_is_invariant_to_thread_count() {
    let frame = gradient_frame(271, 153, 3);
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                frame_to_rgb8_downsampled(&to_ready_frame(&frame), 96, 54)
                    .unwrap()
                    .0
            })
    };
    let single = run(1);
    assert_eq!(single, run(3));
    assert_eq!(single, run(8));
}

/// The channel guard applies to both paths. It used to sit inside the
/// downsample branch only, so an unsupported count silently produced a
/// wrongly sized buffer at native resolution.
#[test]
fn test_unsupported_channel_count_is_rejected_on_both_paths() {
    let small = Frame::zeros(64, 48, 2).unwrap();
    assert!(
        frame_to_rgb8_downsampled(&to_ready_frame(&small), 1920, 1080).is_err(),
        "native path"
    );

    let large = Frame::zeros(4000, 3000, 4).unwrap();
    assert!(
        frame_to_rgb8_downsampled(&to_ready_frame(&large), 1920, 1080).is_err(),
        "downsample path"
    );
}

/// Output length must always be `w * h * 3` — `encode_rgb8_lz4_chunked`
/// slices it as `width * 3` per row and would silently drop the tail.
#[test]
fn test_output_length_always_matches_reported_dimensions() {
    for channels in [1, 3] {
        for (box_w, box_h) in [(1920, 1080), (u32::MAX, u32::MAX), (37, 29)] {
            let frame = gradient_frame(271, 153, channels);
            let (rgb, w, h) =
                frame_to_rgb8_downsampled(&to_ready_frame(&frame), box_w, box_h).unwrap();
            assert_eq!(
                rgb.len(),
                w as usize * h as usize * 3,
                "channels={channels} box={box_w}x{box_h}"
            );
            assert!(w <= box_w.max(271) && h <= box_h.max(153));
        }
    }
}

/// Every test above this point runs with `auto_stretch = false`, so the scale-LUT
/// branch inside `expand_to_rgb8_fused` (black point subtraction + tone-curve scale)
/// had zero coverage. Uses a flat LUT so the expected output is trivial to hand-verify
/// — the point here is "does the kernel read and apply `stretch_result` at all",
/// not "is the curve math correct" (covered exhaustively by `render::simd`'s own tests).
#[test]
fn test_expand_to_rgb8_fused_applies_stretch_scale_and_black_point() {
    // R=G=B per pixel, so luminance equals the shared channel value regardless of
    // the 0.2126/0.7152/0.0722 weighting, keeping the expected values simple.
    let data = vec![
        0.2, 0.2, 0.2, 0.05, 0.05, 0.05, //
        0.9, 0.9, 0.9, 0.0, 0.0, 0.0,
    ];
    let frame = Frame::from_f32_vec(data, 2, 2, 3).unwrap();
    let scale_lut = std::sync::Arc::new(vec![2.0f32; 8192]); // flat 2x scale
    let ready = to_ready_frame_with_stretch(&frame, 0.1, scale_lut);

    let rgb8 = expand_to_rgb8_fused(&ready);

    // (0,0): (0.2 - 0.1).max(0) * 2.0 = 0.2 -> u8 51
    assert_eq!(&rgb8[0..3], &[51, 51, 51]);
    // (1,0): (0.05 - 0.1).max(0) = 0.0 -> below black point, clamped to 0
    assert_eq!(&rgb8[3..6], &[0, 0, 0]);
    // (0,1): (0.9 - 0.1).max(0) * 2.0 = 1.6, clamped to 1.0 -> u8 255
    assert_eq!(&rgb8[6..9], &[255, 255, 255]);
    // (1,1): (0.0 - 0.1).max(0) = 0.0 -> 0
    assert_eq!(&rgb8[9..12], &[0, 0, 0]);
}

/// Pins the ordering documented on `box_downsample_to_rgb8_fused`: for a concave
/// tone curve, averaging in linear light and *then* stretching must never come out
/// dimmer than stretching each source pixel first and averaging the results
/// afterward — see that function's doc comment for the Jensen's-inequality argument.
/// `curve(l) = sqrt(l)` is used here as a simple, clearly concave stand-in for the
/// real asinh/MTF curves.
#[test]
fn test_downsample_then_stretch_is_at_least_as_bright_as_stretch_then_downsample() {
    const N: usize = 8192;
    // scale_lut(l) * l == sqrt(l), i.e. scale_lut(l) = 1/sqrt(l).
    let scale_lut: Vec<f32> = (0..N)
        .map(|i| {
            let l = (i as f32 / (N - 1) as f32).max(1e-6); // avoid 1/0 at index 0
            1.0 / l.sqrt()
        })
        .collect();
    let scale_lut = std::sync::Arc::new(scale_lut);

    // 2x2 box: two dim (0.0) pixels, two bright (0.8) pixels. R=G=B per pixel.
    let data = vec![
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, //
        0.8, 0.8, 0.8, 0.8, 0.8, 0.8,
    ];
    let frame = Frame::from_f32_vec(data, 2, 2, 3).unwrap();
    let ready = to_ready_frame_with_stretch(&frame, 0.0, scale_lut);

    let actual = box_downsample_to_rgb8_fused(&ready, 1, 1);

    // Production order (downsample-then-stretch): average = 0.4, curve(0.4) =
    // sqrt(0.4) ~= 0.632456 -> u8 ~= 161 (+-1 for LUT interpolation error).
    assert!(
        (actual[0] as i32 - 161).abs() <= 1,
        "expected ~161, got {:?}",
        actual
    );

    // Reference order (stretch-then-downsample, computed by hand, NOT via the
    // kernel): curve(0.0)=0.0 (x2), curve(0.8)=sqrt(0.8)~=0.894427 (x2);
    // average = 0.447214 -> u8 = 114. The gap (47 LSB) comfortably absorbs the
    // ~0.15 LSB interpolation error documented on `scale_lut_lookup`.
    let reference_u8 = 114;
    assert!(
        actual[0] as i32 > reference_u8,
        "downsample-then-stretch ({}) should be brighter than stretch-then-downsample \
             ({reference_u8}) for a concave curve — the ordering guarantee has regressed",
        actual[0]
    );
}
