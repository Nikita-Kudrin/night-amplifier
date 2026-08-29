use crate::frame::Frame;

use crate::server::encoding::format::*;
use crate::server::encoding::fused::*;
use crate::server::encoding::jpeg::calculate_dynamic_jpeg_quality;
use crate::server::encoding::jpeg::*;
use crate::server::encoding::lz4::*;

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

#[test]
fn test_rgb8_lz4_encode_header_format() {
    let frame = Frame::filled(2, 2, 3, 0.5).unwrap();
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();

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

    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();
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
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();

    let raw_size = 100 * 100 * 3;
    let compressed_size = encoded.len() - 16;
    assert!(compressed_size < raw_size / 2);
}

#[test]
fn test_rgb8_lz4_various_frame_sizes() {
    let test_cases = [(1, 1), (10, 10), (100, 50), (1920, 1080)];
    for (width, height) in test_cases {
        let frame = Frame::zeros(width, height, 3).unwrap();
        let encoded = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();
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
    let encoded = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();

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
    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 2, 3840, 2160).unwrap();

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

    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 4, 3840, 2160).unwrap();
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
    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 1, 3840, 2160).unwrap();

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
        let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), chunks, 3840, 2160).unwrap();
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

    let sa08 = encode_rgb8_lz4(&to_ready_frame(&frame), 3840, 2160).unwrap();
    let sa08_pixels = decompress_size_prepended(&sa08[16..]).unwrap();

    let sa09 = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 4, 3840, 2160).unwrap();
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
    let area = width * height;
    for y in 0..height {
        for x in 0..width {
            for c in 0..channels {
                let idx = c * area + y * width + x;
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
        0.2, 0.05, 0.9, 0.0, //
        0.2, 0.05, 0.9, 0.0, //
        0.2, 0.05, 0.9, 0.0,
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
        0.0, 0.0, 0.8, 0.8, // R
        0.0, 0.0, 0.8, 0.8, // G
        0.0, 0.0, 0.8, 0.8, // B
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

// ---------------------------------------------------------------------------
// Display transform (black floor + ordered dither) through the fused kernels
// ---------------------------------------------------------------------------

/// Ready frame carrying a display transform, with every render stage off so a
/// test observes only what the 8-bit conversion did.
fn to_ready_frame_with_display(
    frame: &Frame,
    display: crate::render::DisplayOutput,
) -> crate::server::state::RenderReadyFrame {
    let mut ready = to_ready_frame(frame);
    ready.pipeline_config.display = display;
    ready
}

/// Frame large enough that `frame_to_rgb8_downsampled` takes the box-downsample
/// traversal rather than the expand one.
const OVERSIZE: (usize, usize) = (3900, 2200);

/// The dark blocks this was built to remove. A sky at zero must not reach an
/// OLED as an off pixel — through the traversal that streams a frame small
/// enough to send at native size.
#[test]
fn display_pedestal_lifts_black_off_zero_in_the_expand_kernel() {
    let frame = Frame::zeros(64, 48, 3).unwrap();
    let display = crate::render::DisplayOutput::default().with_pedestal(0.04);

    let (bytes, w, h) = frame_to_rgb8_downsampled(&to_ready_frame_with_display(&frame, display), 3840, 2160)
        .expect("encode failed");
    assert_eq!((w, h), (64, 48), "frame should not have been downsampled");
    assert!(
        bytes.iter().all(|&b| b > 0),
        "pedestal did not reach the expand kernel"
    );

    // And without it the same frame is all zeros, so the assertion above is
    // actually detecting the transform rather than something else.
    let (plain, _, _) = frame_to_rgb8_downsampled(
        &to_ready_frame_with_display(&frame, crate::render::DisplayOutput::PLAIN),
        3840,
        2160,
    )
    .unwrap();
    assert!(plain.iter().all(|&b| b == 0));
}

/// The same property through the *other* fused traversal. `AGENTS.md` asks for
/// each traversal to be covered separately: the two kernels gather planes
/// independently, so a gap in one does not show up via the other.
#[test]
fn display_pedestal_lifts_black_off_zero_in_the_downsample_kernel() {
    let (width, height) = OVERSIZE;
    let frame = Frame::zeros(width, height, 3).unwrap();
    let display = crate::render::DisplayOutput::default().with_pedestal(0.04);

    let (bytes, w, h) = frame_to_rgb8_downsampled(&to_ready_frame_with_display(&frame, display), 1920, 1080)
        .expect("encode failed");
    assert!(w < width as u32 && h < height as u32, "expected a downsample");
    assert!(
        bytes.iter().all(|&b| b > 0),
        "pedestal did not reach the downsample kernel"
    );
}

/// A plain transform must leave both kernels byte-identical to what they
/// produced before they carried one, so enabling the feature is the only thing
/// that can change a streamed frame.
#[test]
fn plain_display_transform_leaves_both_kernels_unchanged() {
    let mut frame = Frame::zeros(200, 120, 3).unwrap();
    let mut seed = 0x1234_5678u32;
    for y in 0..120 {
        for x in 0..200 {
            for c in 0..3 {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                frame.set_pixel(x, y, c, (seed >> 8) as f32 / 16_777_216.0);
            }
        }
    }

    // Expand traversal: identical to the canonical whole-frame conversion.
    let (expanded, _, _) = frame_to_rgb8_downsampled(
        &to_ready_frame_with_display(&frame, crate::render::DisplayOutput::PLAIN),
        3840,
        2160,
    )
    .unwrap();
    assert_eq!(expanded, frame.to_rgb8_fast());

    // Downsample traversal: stable across runs and unaffected by the field.
    let default_cfg = frame_to_rgb8_downsampled(&to_ready_frame(&frame), 100, 60).unwrap();
    let plain_cfg = frame_to_rgb8_downsampled(
        &to_ready_frame_with_display(&frame, crate::render::DisplayOutput::PLAIN),
        100,
        60,
    )
    .unwrap();
    assert_eq!(default_cfg, plain_cfg);
}

/// Dither must be indexed in *output* coordinates. If a kernel indexed the
/// source pixel instead, the pattern would survive at the source's period
/// rather than the output's — so assert the tile repeats every 8 output pixels
/// after a downsample that is not a multiple of 8.
#[test]
fn dither_tiles_in_output_coordinates_after_downsampling() {
    let (width, height) = OVERSIZE;
    // A flat mid-grey between two 8-bit levels, so only the dither varies.
    let frame = Frame::filled(width, height, 3, 40.5 / 255.0).unwrap();
    let display = crate::render::DisplayOutput::default().with_dither(true);

    let (bytes, w, _h) =
        frame_to_rgb8_downsampled(&to_ready_frame_with_display(&frame, display), 1920, 1080)
            .expect("encode failed");

    let row = &bytes[..w as usize * 3];
    for x in 0..16usize {
        assert_eq!(
            row[x * 3],
            row[(x + 8) * 3],
            "output column {x} and {} differ; dither is not tiling in output space",
            x + 8
        );
    }
    // A flat input between levels must produce more than one output level, or
    // the dither is not doing anything.
    let distinct: std::collections::HashSet<u8> = row.iter().step_by(3).copied().collect();
    assert!(
        distinct.len() > 1,
        "dithered flat field collapsed to a single level"
    );
}

/// The reason the dither exists: a flat field sitting between two 8-bit levels
/// must average to that value across a tile instead of snapping to one level.
#[test]
fn dither_preserves_sub_lsb_level_through_the_streaming_kernel() {
    let value = 40.25 / 255.0;
    let frame = Frame::filled(64, 64, 3, value).unwrap();

    let (dithered, w, h) = frame_to_rgb8_downsampled(
        &to_ready_frame_with_display(
            &frame,
            crate::render::DisplayOutput::default().with_dither(true),
        ),
        3840,
        2160,
    )
    .unwrap();
    let mean = dithered.iter().step_by(3).map(|&v| v as f64).sum::<f64>()
        / (w as f64 * h as f64);
    assert!(
        (mean - 40.25).abs() < 0.1,
        "dithered mean {mean} should track 40.25"
    );

    let (plain, _, _) = frame_to_rgb8_downsampled(
        &to_ready_frame_with_display(&frame, crate::render::DisplayOutput::PLAIN),
        3840,
        2160,
    )
    .unwrap();
    assert!(
        plain.iter().step_by(3).all(|&v| v == 40),
        "undithered conversion should snap the whole field to one level"
    );
}

/// The lossless encoder must honour the box it is handed, since that box is now
/// the client's viewport rather than a hardcoded 4K cap.
#[test]
fn lz4_encodes_into_the_requested_box() {
    let frame = Frame::filled(3008, 3008, 3, 0.25).unwrap();

    let encoded = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 2, 2560, 1440).unwrap();
    let width = u32::from_le_bytes(encoded[4..8].try_into().unwrap());
    let height = u32::from_le_bytes(encoded[8..12].try_into().unwrap());
    assert_eq!(
        (width, height),
        (1440, 1440),
        "3008x3008 fitted into a 2560x1440 box should be 1440x1440"
    );

    // The old behaviour, still reachable by passing the cap.
    let native = encode_rgb8_lz4_chunked(&to_ready_frame(&frame), 2, 3840, 2160).unwrap();
    let native_h = u32::from_le_bytes(native[8..12].try_into().unwrap());
    assert_eq!(native_h, 2160);
    assert!(
        encoded.len() < native.len(),
        "a smaller box must produce a smaller payload"
    );
}

// ---------------------------------------------------------------------------
// Spatial denoising through the fused kernels (Tier 2)
// ---------------------------------------------------------------------------

/// A frame carrying deterministic noise, so a denoiser has something to remove
/// and the result is reproducible.
fn noisy_frame(width: usize, height: usize, base: f32, amplitude: f32) -> Frame {
    let mut frame = Frame::filled(width, height, 3, base).unwrap();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for y in 0..height {
        for x in 0..width {
            for c in 0..3 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let n = ((state >> 40) as f32 / 16777216.0) - 0.5;
                frame.set_pixel(x, y, c, base + n * amplitude + c as f32 * 0.02);
            }
        }
    }
    frame
}

fn ready_with_denoise(
    frame: &Frame,
    denoise: crate::render::DenoiseConfig,
) -> crate::server::state::RenderReadyFrame {
    let mut ready = to_ready_frame(frame);
    ready.pipeline_config.denoise = denoise;
    ready
}

/// Every spelling of "off" must take the fused traversal, not a staged one that
/// happens to compute the same thing. `is_enabled` is what routes between them,
/// so its contract is pinned at the byte level on both kernels.
#[test]
fn every_disabled_denoise_config_is_byte_identical_through_both_kernels() {
    let variants = [
        crate::render::DenoiseConfig::OFF,
        crate::render::DenoiseConfig {
            luma: crate::render::LumaDenoiseConfig {
                enabled: false,
                ..Default::default()
            },
            chroma: crate::render::ChromaDenoiseConfig {
                enabled: false,
                ..Default::default()
            },
        },
        crate::render::DenoiseConfig {
            luma: crate::render::LumaDenoiseConfig {
                strength: 0.0,
                ..Default::default()
            },
            chroma: crate::render::ChromaDenoiseConfig {
                strength: 0.0,
                ..Default::default()
            },
        },
        crate::render::DenoiseConfig {
            luma: crate::render::LumaDenoiseConfig {
                k: [0.0; 4],
                ..Default::default()
            },
            chroma: crate::render::ChromaDenoiseConfig {
                radius: 0,
                ..Default::default()
            },
        },
    ];

    let small = noisy_frame(96, 72, 0.2, 0.05);
    let (expand_baseline, _, _) =
        frame_to_rgb8_downsampled(&to_ready_frame(&small), 3840, 2160).unwrap();
    let big = noisy_frame(200, 150, 0.2, 0.05);
    let (reduce_baseline, _, _) =
        frame_to_rgb8_downsampled(&to_ready_frame(&big), 100, 75).unwrap();

    for (i, denoise) in variants.into_iter().enumerate() {
        let (off, _, _) =
            frame_to_rgb8_downsampled(&ready_with_denoise(&small, denoise), 3840, 2160).unwrap();
        assert_eq!(expand_baseline, off, "expand kernel changed for variant {i}");

        let (off, _, _) =
            frame_to_rgb8_downsampled(&ready_with_denoise(&big, denoise), 100, 75).unwrap();
        assert_eq!(reduce_baseline, off, "downsample kernel changed for variant {i}");
    }
}

/// The staged path must actually filter, on both traversals — a config that is
/// wired but never reaches the kernels would pass every layout test in the repo.
#[test]
fn denoising_reduces_sky_sigma_through_both_kernels() {
    let denoise = crate::render::DenoiseConfig {
        luma: crate::render::LumaDenoiseConfig {
            k: [1.0, 3.0, 2.0, 1.0],
            ..Default::default()
        },
        chroma: crate::render::ChromaDenoiseConfig::default(),
    };

    let sigma = |bytes: &[u8]| {
        let vals: Vec<f64> = bytes.iter().skip(1).step_by(3).map(|&v| v as f64).collect();
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        (vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64).sqrt()
    };

    let small = noisy_frame(128, 128, 0.3, 0.1);
    let (plain, _, _) = frame_to_rgb8_downsampled(&to_ready_frame(&small), 3840, 2160).unwrap();
    let (filtered, _, _) =
        frame_to_rgb8_downsampled(&ready_with_denoise(&small, denoise), 3840, 2160).unwrap();
    assert!(
        sigma(&filtered) < sigma(&plain) * 0.6,
        "expand kernel: sigma only fell from {:.2} to {:.2}",
        sigma(&plain),
        sigma(&filtered)
    );

    let big = noisy_frame(256, 256, 0.3, 0.1);
    let (plain, _, _) = frame_to_rgb8_downsampled(&to_ready_frame(&big), 128, 128).unwrap();
    let (filtered, _, _) =
        frame_to_rgb8_downsampled(&ready_with_denoise(&big, denoise), 128, 128).unwrap();
    assert!(
        sigma(&filtered) < sigma(&plain) * 0.6,
        "downsample kernel: sigma only fell from {:.2} to {:.2}",
        sigma(&plain),
        sigma(&filtered)
    );
}

/// The staged path still has to run the tone curve, and in the same order: the
/// denoisers sit between the resample and the stretch, not after it. A staged
/// buffer that skipped or reordered the tail would produce a visibly different
/// image while passing every layout and sigma assertion above.
#[test]
fn the_staged_path_still_applies_the_stretch_before_quantizing() {
    let frame = Frame::filled(32, 24, 3, 0.1).unwrap();
    let lut: std::sync::Arc<Vec<f32>> =
        std::sync::Arc::new((0..1024).map(|i| 1.0 + i as f32 / 1024.0 * 4.0).collect());

    let mut ready = to_ready_frame_with_stretch(&frame, 0.02, lut);
    let (unfiltered, _, _) = frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();

    ready.pipeline_config.denoise = crate::render::DenoiseConfig {
        luma: crate::render::LumaDenoiseConfig::default(),
        chroma: crate::render::ChromaDenoiseConfig::default(),
    };
    let (staged, _, _) = frame_to_rgb8_downsampled(&ready, 3840, 2160).unwrap();

    // A constant frame has nothing for either filter to remove, so the staged
    // path must reproduce the fused one exactly — including the stretch.
    assert_eq!(
        unfiltered, staged,
        "staged path disagrees with the fused one on a frame neither filter can change"
    );
    assert!(
        unfiltered.iter().any(|&b| b > 26),
        "stretch did not run: 0.1 should be lifted well above its linear byte"
    );
}
