use crate::frame::Frame;

use crate::server::encoding::format::*;
use crate::server::encoding::fused::*;
use crate::server::encoding::jpeg::*;
use crate::server::encoding::lz4::*;

fn to_ready_frame(frame: &Frame) -> crate::server::state::RenderReadyFrame {
    let mut config = crate::render::RenderPipelineConfig::default();
    config.contrast = false;
    config.auto_stretch = false;
    config.saturation_boost = false;
    crate::server::state::RenderReadyFrame {
        noise: None,
        linear_frame: std::sync::Arc::new(frame.clone()),
        pipeline_config: config,
        stretch_result: None,
    }
}

/// Like `to_ready_frame`, but with `auto_stretch` actually enabled and a real
/// `StretchResult` attached — every fused-kernel test up to this point runs with
/// stretch/saturation/contrast all disabled, so the scale-LUT application branch in
/// `expand_to_rgb8_fused`/`area_downsample_to_rgb8_fused` had no coverage at all.
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
        noise: None,
        linear_frame: std::sync::Arc::new(frame.clone()),
        pipeline_config: config,
        stretch_result: Some(crate::server::state::StretchResult {
            deferred_shadow_floor: None,
            sky_shadow: None,
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
fn jpeg_quality_is_95_below_1440p_and_for_every_denoised_frame() {
    // (width, height, denoised) -> quality; the smaller side decides the size rule.
    let cases = [
        (1920, 1080, false, 95),
        (640, 480, false, 95),
        (1080, 1920, false, 95),
        (2560, 1440, false, 90),
        (3840, 2160, false, 90),
        (2160, 3840, false, 90),
        (1920, 1080, true, 95),
        (2560, 1440, true, 95),
        (3840, 2160, true, 95),
    ];
    for (w, h, denoised, quality) in cases {
        assert_eq!(jpeg_quality(w, h, denoised), quality, "{w}x{h}, denoised {denoised}");
    }
}

#[test]
fn test_jpeg_encode_fits_a_square_frame_into_the_4k_box() {
    let frame = Frame::zeros(5000, 5000, 3).unwrap();
    let encoded = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), 3840, 2160).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    // A 1:1 frame fitted into 3840x2160 is limited by the short edge.
    assert_eq!((width, height), (2160, 2160));
}

#[test]
fn test_jpeg_encode_fits_a_square_frame_into_the_1080p_box() {
    let frame = Frame::zeros(2000, 2000, 3).unwrap();
    let encoded = encode_rgb8_jpeg_bounded(&to_ready_frame(&frame), 1920, 1080).unwrap();

    let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
    assert_eq!((width, height), (1080, 1080));
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

/// The resampling kernel written out the slow way: every output pixel integrates its
/// footprint `[i*scale, (i+1)*scale)` over linearly interpolated source samples (a
/// 1 px tent each), weights computed in 2D per pixel with f64 and a real division.
/// Independent of `AxisTaps` on purpose: a shared helper would agree with itself.
fn reference_downsample_to_rgb8(frame: &Frame, target_w: usize, target_h: usize) -> Vec<u8> {
    let (w, h, channels) = (frame.width(), frame.height(), frame.channels());
    let tent_integral = |lo: f64, hi: f64, centre: f64| {
        // Midpoint rule at 1/64 px: exact enough for a 1 LSB bound.
        let steps = (((hi - lo) * 64.0).ceil() as usize).max(1);
        let dt = (hi - lo) / steps as f64;
        (0..steps)
            .map(|k| (1.0 - (lo + (k as f64 + 0.5) * dt - centre).abs()).max(0.0) * dt)
            .sum::<f64>()
    };
    // Normalised area-tent weights of one output pixel.
    let footprint = |len: usize, target: usize, i: usize| -> Vec<(usize, f64)> {
        let scale = len as f64 / target as f64;
        let (lo, hi) = (i as f64 * scale, (i + 1) as f64 * scale);
        let near = (lo - 2.0).max(0.0) as usize..((hi + 2.0) as usize).min(len);
        let w: Vec<(usize, f64)> = near
            .map(|j| (j, tent_integral(lo, hi, j as f64 + 0.5)))
            .filter(|&(_, weight)| weight > 0.0)
            .collect();
        let sum: f64 = w.iter().map(|(_, v)| v).sum();
        w.into_iter().map(|(j, v)| (j, v / sum)).collect()
    };
    // The output-grid sharpen `[-a, 1+2a, -a]`, restated: 0.15 up to 1.1x, none from 1.9x.
    let axis = |len: usize, target: usize, i: usize| -> Vec<(usize, f64)> {
        let scale = len as f64 / target as f64;
        let a = 0.15 * ((1.9 - scale) / 0.8).clamp(0.0, 1.0);
        if a == 0.0 || target < 3 {
            return footprint(len, target, i);
        }
        let mut acc = std::collections::BTreeMap::<usize, f64>::new();
        for (k, n) in [(1.0 + 2.0 * a, i), (-a, i.saturating_sub(1)), (-a, (i + 1).min(target - 1))] {
            for (j, v) in footprint(len, target, n) {
                *acc.entry(j).or_default() += k * v;
            }
        }
        acc.into_iter().collect()
    };
    let mut out = vec![0u8; target_w * target_h * 3];

    for y in 0..target_h {
        let rows = axis(h, target_h, y);
        for x in 0..target_w {
            let cols = axis(w, target_w, x);
            let mut avg = [0.0f64; 3];
            let mut total = 0.0;
            for &(sy, wy) in &rows {
                for &(sx, wx) in &cols {
                    total += wy * wx;
                    for (c, a) in avg.iter_mut().enumerate().take(channels) {
                        *a += wy * wx * frame.get_pixel(sx, sy, c) as f64;
                    }
                }
            }
            for a in avg.iter_mut() {
                *a /= total;
            }
            if channels == 1 {
                avg[1] = avg[0];
                avg[2] = avg[0];
            }

            let idx = (y * target_w + x) * 3;
            for c in 0..3 {
                out[idx + c] = (avg[c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

/// The fused kernel against the slow reference: separable f32 taps against 2D f64
/// weights may differ by a rounding step but must never differ visibly.
#[test]
fn test_downsample_matches_reference_within_1_lsb() {
    // Non-integer scale factors, so the footprint's phase varies along both axes.
    for (w, h, box_w, box_h) in [
        (400, 300, 137, 111),
        (271, 153, 96, 54),
        (300, 400, 111, 137),
        // Near unity (IMX464 at 1440p) and 1.42x, where the sharpen applies.
        (321, 300, 300, 281),
        (284, 200, 200, 141),
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

    let rgb8 = expand_to_rgb8_fused(&ready, &mut Default::default());

    // (0,0): (0.2 - 0.1).max(0) * 2.0 = 0.2 -> u8 51
    assert_eq!(&rgb8[0..3], &[51, 51, 51]);
    // (1,0): (0.05 - 0.1).max(0) = 0.0 -> below black point, clamped to 0
    assert_eq!(&rgb8[3..6], &[0, 0, 0]);
    // (0,1): (0.9 - 0.1).max(0) * 2.0 = 1.6, clamped to 1.0 -> u8 255
    assert_eq!(&rgb8[6..9], &[255, 255, 255]);
    // (1,1): (0.0 - 0.1).max(0) = 0.0 -> 0
    assert_eq!(&rgb8[9..12], &[0, 0, 0]);
}

/// Pins the ordering documented on `area_downsample_to_rgb8_fused`: for a concave
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

    let actual = area_downsample_to_rgb8_fused(&ready, 1, 1, &mut Default::default());

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
// Display transform (black floor + dither) through the fused kernels
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
/// rather than the output's — so assert the tile repeats every 64 output pixels
/// after a downsample that is not a multiple of 64.
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
    for x in 0..128usize {
        assert_eq!(
            row[x * 3],
            row[(x + 64) * 3],
            "output column {x} and {} differ; dither is not tiling in output space",
            x + 64
        );
    }
    // Down a column too: rows are written in parallel chunks, and a chunk-relative row
    // index would restart the mask at every chunk instead of every 64 rows.
    let column: Vec<u8> = bytes.chunks_exact(w as usize * 3).map(|r| r[0]).collect();
    for y in 0..128usize {
        assert_eq!(column[y], column[y + 64], "output rows {y} and {} differ in column 0", y + 64);
    }
    assert_ne!(column[..32], column[32..64], "the mask repeats every 32 rows, not 64");
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
                k: [0.0; crate::render::MAX_WAVELET_LEVELS],
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

/// A non-integer box downsample must average the same source area into every output
/// pixel, or sky noise prints a lattice. 3008 -> 1440 (the eyepiece default on IMX533)
/// is 2.089x: boxes are 2 px wide except every ~11th row/column, which is 3, so those
/// lines are less noisy. Seen in the 2026-09-14 globular session at 18 arcmin per
/// block through a 100 mm eyepiece lens: ~11 % less sky grain on the 3-px lines,
/// 21 % at their crossings. Same 2.089x ratio here, at half the size.
#[test]
fn non_integer_downsample_gives_every_output_pixel_the_same_noise() {
    let (src, dst) = (1504usize, 720u32);
    let mut frame = Frame::zeros(src, src, 1).unwrap();
    let mut seed = 0x9e37_79b9_u32;
    for y in 0..src {
        for x in 0..src {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let uniform = (seed >> 8) as f32 / (1u32 << 24) as f32;
            frame.set_pixel(x, y, 0, 0.3 + 0.4 * uniform);
        }
    }

    let (rgb8, width, height) =
        frame_to_rgb8_downsampled(&to_ready_frame(&frame), dst, dst).unwrap();
    assert_eq!((width, height), (dst, dst));

    let scale = src as f64 / dst as f64;
    let span = |i: usize| ((i + 1) as f64 * scale) as usize - (i as f64 * scale) as usize;
    let mut groups = [(0.0f64, 0.0f64, 0usize); 2];
    for x in 1..dst as usize - 1 {
        let group = usize::from(span(x) != 2);
        for y in (1..dst as usize - 1).filter(|&y| span(y) == 2) {
            let v = rgb8[(y * dst as usize + x) * 3] as f64;
            let g = &mut groups[group];
            g.0 += v;
            g.1 += v * v;
            g.2 += 1;
        }
    }
    let std = |(sum, sq, n): (f64, f64, usize)| (sq / n as f64 - (sum / n as f64).powi(2)).sqrt();
    let (narrow, wide) = (std(groups[0]), std(groups[1]));
    assert!(
        (wide / narrow - 1.0).abs() < 0.05,
        "output columns averaging a wider source box are {:.1} % less noisy ({wide:.2} vs {narrow:.2} levels)",
        (1.0 - wide / narrow) * 100.0
    );
}

/// Near-unity ratios are where the area-tent kernel costs the most sharpness: IMX464 is
/// 1.07x over the 1440p box, where a 2.5 px star kept 83 % of the whole-pixel box's peak
/// before the output-grid sharpen. Planetary live view exists for exactly that detail.
#[test]
fn a_near_unity_downsample_keeps_star_peaks() {
    let (src, dst) = (1538usize, 1440usize);
    let sigma = 2.5 / 2.3548;
    let scale = src as f32 / dst as f32;
    let mut worst = f32::MAX;
    for phase in 0..8 {
        let (cx, cy) = (src as f32 / 2.0 + phase as f32 * 0.133, src as f32 / 2.0);
        let star = |x: usize, y: usize| {
            let r2 = (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2);
            0.9 * (-r2 / (2.0 * sigma * sigma)).exp()
        };
        let mut frame = Frame::zeros(src, src, 1).unwrap();
        for y in src / 2 - 12..src / 2 + 12 {
            for x in src / 2 - 12..src / 2 + 12 {
                frame.set_pixel(x, y, 0, star(x, y));
            }
        }
        let (rgb, _, _) =
            frame_to_rgb8_downsampled(&to_ready_frame(&frame), dst as u32, dst as u32).unwrap();
        let peak = rgb.iter().copied().max().unwrap() as f32;

        // The whole-pixel box this kernel replaced, as the sharpness reference.
        let span = |i: usize| ((i as f32 * scale) as usize, (((i + 1) as f32 * scale) as usize).max((i as f32 * scale) as usize + 1));
        let box_peak = (dst / 2 - 12..dst / 2 + 12)
            .flat_map(|oy| (dst / 2 - 12..dst / 2 + 12).map(move |ox| (ox, oy)))
            .map(|(ox, oy)| {
                let ((x0, x1), (y0, y1)) = (span(ox), span(oy));
                let sum: f32 = (y0..y1).flat_map(|y| (x0..x1).map(move |x| star(x, y))).sum();
                sum / ((x1 - x0) * (y1 - y0)) as f32 * 255.0
            })
            .fold(0.0f32, f32::max);
        worst = worst.min(peak / box_peak);
    }
    assert!(
        worst >= 0.97,
        "a 2.5 px star kept {:.0} % of the whole-pixel box's peak",
        worst * 100.0
    );
}

/// A ready frame with the sky shadow set and an identity tail (no stretch, contrast or
/// saturation), so the expected bytes can be built from the frame's own samples.
fn ready_with_sky_shadow(frame: &Frame, shadow: crate::render::SkyShadow) -> crate::server::state::RenderReadyFrame {
    let mut ready = to_ready_frame(frame);
    ready.stretch_result = Some(crate::server::state::StretchResult {
        black_point: 0.0,
        scale_lut: std::sync::Arc::new(vec![]),
        color_intensity: 1.0,
        deferred_shadow_floor: None,
        sky_shadow: Some(shadow),
    });
    ready
}

/// The denoise-off path streams the sky shadow chunk by chunk; the staged path applies
/// it to the whole image. One setting must produce one image on both — the sky is
/// measured on the same sample rows, each guide row from the same three rows.
///
/// With the dither on too: the stream writes 32-row chunks and the mask tiles every 64
/// rows, so a chunk-relative row index would repeat the mask every 32 rows — invisible
/// under the 8-row Bayer tile it replaced, and to this test with the dither off.
#[test]
fn sky_shadow_streaming_matches_staged() {
    let displays = [
        crate::render::DisplayOutput::default(),
        crate::render::DisplayOutput::default().with_dither(true),
    ];
    for display in displays {
        sky_shadow_streaming_matches_staged_with(display);
    }
}

fn sky_shadow_streaming_matches_staged_with(display: crate::render::DisplayOutput) {
    let sky = 0.052f32;
    for (w, h) in [(71usize, 97usize), (40, 1), (33, 2), (129, 200)] {
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        let mut seed = 0x5eed_u32;
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let u = (seed >> 8) as f32 / (1u32 << 24) as f32;
                let star = if (x * 7 + y * 3) % 53 == 0 { 0.6 } else { 0.0 };
                for c in 0..3 {
                    frame.set_pixel(x, y, c, sky * (0.6 + 0.8 * u) * (1.0 + 0.1 * c as f32) + star);
                }
            }
        }
        let shadow = crate::render::SkyShadow::from_sky(0.7, sky).unwrap();
        let mut ready = ready_with_sky_shadow(&frame, shadow);
        ready.pipeline_config.display = display;
        let (streamed, _, _) = frame_to_rgb8_downsampled(&ready, 4096, 4096).unwrap();

        let mut staged: Vec<f32> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| (0..3).map(move |c| (x, y, c))))
            .map(|(x, y, c)| frame.get_pixel(x, y, c))
            .collect();
        crate::render::output::apply_sky_shadow_interleaved(&mut staged, w, h, shadow, &mut vec![], &mut vec![]);
        let mut expected = vec![0u8; w * h * 3];
        for (y, (out, row)) in expected.chunks_exact_mut(w * 3).zip(staged.chunks_exact(w * 3)).enumerate() {
            crate::render::output::write_row_rgb8(out, row, y, display);
        }
        assert_eq!(streamed, expected, "{w}x{h} {display:?}: streaming and staged sky shadow disagree");
        assert_ne!(streamed, frame_to_rgb8_downsampled(&to_ready_frame(&frame), 4096, 4096).unwrap().0, "{w}x{h}: shadow did nothing");
    }
}

/// The output-grid sharpen has negative lobes, and the black point sits only 2.8 deep-stack
/// sigmas (~3 % of the sky) under the sky, so an undershoot beside a bright star would
/// clip a dark ring. At IMX464's 1.07x none does: deepest +1.1 sigma at FWHM 3 px, -2.9
/// at 1.6 px, the whole-pixel box -3.0.
#[test]
fn a_near_unity_downsample_leaves_no_dark_ring_around_bright_stars() {
    use super::axis_taps::AxisTaps;
    const SKY: f32 = 0.0028;
    const SIGMA: f32 = 3.4e-5;
    const K: f32 = 2.8;
    let (src_len, dst_len) = (1538usize, 1440usize);
    let (n, out_n) = (440usize, 400usize);
    let scale = src_len as f64 / dst_len as f64;

    let mut seed = 0x0bad_5eed_u32;
    let mut gauss = move || {
        let mut u = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 + 1e-7
        };
        let (u1, u2) = (u(), u());
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    };
    let mut src = vec![0.0f32; n * n];
    src.iter_mut().for_each(|v| *v = SKY + SIGMA * gauss());

    // Moffat beta 2.5, FWHM 3 px: saturated and bright-unsaturated stars on a grid.
    let alpha = 3.0 / (2.0 * (2f32.powf(0.4) - 1.0).sqrt());
    let mut stars = Vec::new();
    for (k, (gy, gx)) in (0..9).flat_map(|gy| (0..9).map(move |gx| (gy, gx))).enumerate() {
        let (cx, cy) = (40.0 + gx as f32 * 45.0 + 0.37 * (k % 3) as f32, 40.0 + gy as f32 * 45.0 + 0.29 * (k % 4) as f32);
        let amplitude = if k % 2 == 0 { 20.0 } else { 0.05 };
        for y in (cy as usize - 20)..(cy as usize + 20) {
            for x in (cx as usize - 20)..(cx as usize + 20) {
                let r2 = (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2);
                src[y * n + x] += amplitude * (1.0 + r2 / (alpha * alpha)).powf(-2.5);
            }
        }
        stars.push(((cx as f64 / scale) as f32, (cy as f64 / scale) as f32));
    }
    src.iter_mut().for_each(|v| *v = v.min(1.0));

    let taps = AxisTaps::new(src_len, dst_len);
    let tent = |img: &[f32]| -> Vec<f32> {
        let mut out = vec![0.0f32; out_n * out_n];
        for oy in 0..out_n {
            let (fy, wy) = taps.of(oy);
            for ox in 0..out_n {
                let (fx, wx) = taps.of(ox);
                let mut acc = 0.0f32;
                for (j, &a) in wy.iter().enumerate() {
                    for (i, &b) in wx.iter().enumerate() {
                        acc += a * b * img[(fy + j) * n + fx + i];
                    }
                }
                out[oy * out_n + ox] = acc;
            }
        }
        out
    };
    let whole_pixel_box = |img: &[f32]| -> Vec<f32> {
        let span = |i: usize| ((i as f64 * scale) as usize, (((i + 1) as f64 * scale) as usize).max((i as f64 * scale) as usize + 1));
        let mut out = vec![0.0f32; out_n * out_n];
        for oy in 0..out_n {
            let (y0, y1) = span(oy);
            for ox in 0..out_n {
                let (x0, x1) = span(ox);
                let sum: f32 = (y0..y1).flat_map(|y| (x0..x1).map(move |x| img[y * n + x])).sum();
                out[oy * out_n + ox] = sum / ((y1 - y0) * (x1 - x0)) as f32;
            }
        }
        out
    };

    let black_point = SKY - K * SIGMA;
    let ring_stats = |out: &[f32]| {
        let (mut ring, mut ring_black, mut sky, mut sky_black, mut worst) = (0usize, 0usize, 0usize, 0usize, f32::MAX);
        for oy in 0..out_n {
            for ox in 0..out_n {
                let d = stars
                    .iter()
                    .map(|&(cx, cy)| (ox as f32 + 0.5 - cx).hypot(oy as f32 + 0.5 - cy))
                    .fold(f32::MAX, f32::min);
                let v = out[oy * out_n + ox];
                if (2.0..8.0).contains(&d) {
                    ring += 1;
                    ring_black += usize::from(v < black_point);
                    worst = worst.min((v - SKY) / SIGMA);
                } else if d > 15.0 {
                    sky += 1;
                    sky_black += usize::from(v < black_point);
                }
            }
        }
        (ring_black as f32 / ring as f32, sky_black as f32 / sky as f32, worst)
    };
    let (tent_ring, tent_sky, tent_worst) = ring_stats(&tent(&src));
    let (box_ring, box_sky, box_worst) = ring_stats(&whole_pixel_box(&src));
    println!(
        "below black point: area-tent ring {:.2} % (sky {:.2} %, deepest {tent_worst:.1} sigma), \
         whole-pixel box ring {:.2} % (sky {:.2} %, deepest {box_worst:.1} sigma)",
        tent_ring * 100.0,
        tent_sky * 100.0,
        box_ring * 100.0,
        box_sky * 100.0
    );
    assert!(
        tent_ring <= (box_ring + 0.005).max(tent_sky * 2.0),
        "the sharpen clips {:.1} % of the pixels 2-8 px from bright stars to black \
         (box: {:.1} %, open sky: {:.2} %) — a dark ring round every bright star",
        tent_ring * 100.0,
        box_ring * 100.0,
        tent_sky * 100.0
    );
}

/// The denoise-on path streams its denoised rows through the same sky-shadow driver; it
// ---------------------------------------------------------------------------
// The quadrature contract: how a noise map has to be resampled.
//
// This is the single easiest thing in the noise-map work to get wrong and the hardest
// to see, because nothing downstream reports a number that would show it. Getting it
// wrong makes every threshold built on the map too aggressive by roughly sqrt(k).
// ---------------------------------------------------------------------------

/// Independent Gaussian noise of unit sigma, deterministic across machines and runs.
fn white_noise(len: usize, seed: u32) -> Vec<f32> {
    let mut state = seed | 1;
    let mut u = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / (1u32 << 24) as f32 + 1e-7
    };
    (0..len)
        .map(|_| {
            let (u1, u2) = (u(), u());
            (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
        })
        .collect()
}

/// Separable resample of an f32 plane through the production taps.
fn resample_plane(
    src: &[f32],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<f32> {
    use super::axis_taps::AxisTaps;
    let columns = AxisTaps::cached(src_w, dst_w);
    let rows = AxisTaps::cached(src_h, dst_h);
    let mut out = vec![0.0f32; dst_w * dst_h];
    for oy in 0..dst_h {
        let (first_row, wy) = rows.of(oy);
        for ox in 0..dst_w {
            let (first_col, wx) = columns.of(ox);
            let mut acc = 0.0f32;
            for (j, &a) in wy.iter().enumerate() {
                let row = (first_row + j).min(src_h - 1) * src_w;
                for (i, &b) in wx.iter().enumerate() {
                    acc += a * b * src[row + (first_col + i).min(src_w - 1)];
                }
            }
            out[oy * dst_w + ox] = acc;
        }
    }
    out
}

fn variance(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32
}

/// `sum(w^2)` really is the factor by which the resample scales variance.
#[test]
fn quadrature_resample_predicts_measured_noise() {
    use super::axis_taps::AxisTaps;
    // The shipped eyepiece geometry: IMX533 to a 1440 box, a 2.089x non-integer ratio.
    let (src, dst) = (1504usize, 720usize);
    let plane = white_noise(src * src, 0x51de_5eed);
    let out = resample_plane(&plane, src, src, dst, dst);

    let measured = variance(&out);
    let columns = AxisTaps::cached(src, dst);
    let rows = AxisTaps::cached(src, dst);
    // The source sigma is 1, so the prediction is the mean tap energy of both axes.
    let mean_sum_sq = |t: &AxisTaps| t.sum_sq().iter().sum::<f32>() / t.sum_sq().len() as f32;
    let predicted = variance(&plane) * mean_sum_sq(&columns) * mean_sum_sq(&rows);

    assert!(
        (measured / predicted - 1.0).abs() < 0.03,
        "measured output variance {measured:e} against a predicted {predicted:e}"
    );
}

/// And the guard has to be able to refute the alternative, or it is not guarding the
/// choice: resampling the map like an image — averaging sigmas, which is what
/// `sum(w) = 1` gives — leaves the source sigma untouched and so overstates the output.
#[test]
fn resampling_the_field_like_an_image_does_not() {
    let (src, dst) = (1504usize, 720usize);
    let plane = white_noise(src * src, 0x51de_5eed);
    let out = resample_plane(&plane, src, src, dst, dst);

    let measured = variance(&out);
    // `sum(w) = 1`, so an image-like resample of a flat sigma field returns that sigma.
    let image_like = variance(&plane);
    let overstatement = (image_like / measured).sqrt();
    assert!(
        overstatement > 1.8,
        "an image-like resample overstated output sigma by only {overstatement:.2}x; \
         the trap this guards is worth ~sqrt(k), so either the ratio moved or the \
         measurement is not seeing it"
    );
}

/// End to end through the shipped type: a `NoiseField` carrying the source variance,
/// resampled with the production taps, must land on what the resample actually produces.
#[test]
fn a_resampled_noise_field_matches_the_encoder_it_describes() {
    use super::axis_taps::AxisTaps;
    use crate::frame::{NoiseField, NOISE_REDUCTION};

    let (src, dst) = (1504usize, 720usize);
    let plane = white_noise(src * src, 0x0c0f_fee1);
    let source_variance = variance(&plane);

    let cells = src.div_ceil(NOISE_REDUCTION);
    let field = NoiseField::new(
        vec![source_variance; cells * cells],
        cells,
        cells,
        1,
        src,
        src,
    )
    .unwrap();

    let columns = AxisTaps::cached(src, dst);
    let rows = AxisTaps::cached(src, dst);
    let resampled = field
        .resampled(dst, dst, columns.sum_sq(), rows.sum_sq())
        .unwrap();

    let measured = variance(&resample_plane(&plane, src, src, dst, dst));
    let predicted = resampled.sample(0, dst / 2, dst / 2);
    assert!(
        (predicted / measured - 1.0).abs() < 0.05,
        "the field predicts {predicted:e} where the encoder produces {measured:e}"
    );
}

/// How much the tap energy varies across the output grid decides whether working in
/// *relative* noise is enough on its own — a factor common to every output pixel
/// cancels in a ratio, one that varies does not.
///
/// Two regimes, and they differ by more than the difference between them looks:
///
/// - **At the shipped eyepiece geometry** (3008 -> 1440, 2.089x) the interior varies
///   1.037x in variance, i.e. 1.018x in sigma. There the sharpen is off entirely
///   (`SHARPEN_NONE_FROM` is 1.9x) and the tent alone is very nearly phase-invariant,
///   which is what makes a relative field a ~2 % approximation.
/// - **Near unity** (1538 -> 1440, 1.068x) it reaches 1.28x in variance, 1.13x in sigma.
///   That is the `[-a, 1+2a, -a]` sharpen: its negative lobes raise `sum(w^2)` sharply
///   and by an amount that moves with the phase. A relative field is a ~13 %
///   approximation there, not a ~2 % one, so anything reading the map *absolutely*
///   matters more at IMX464's near-unity ratio than at IMX533's.
///
/// Both bounds are measured, not derived. The first output pixel is excluded: the frame
/// edge renormalises its footprint after dropping out-of-range samples and reads 1.16x
/// the interior on its own, which is real, carried correctly by the resample, and not
/// where anybody reads sky noise.
#[test]
fn the_tap_phase_variation_is_small_where_the_sharpen_is_off() {
    use super::axis_taps::AxisTaps;
    let spread = |source: usize, target: usize| {
        let taps = AxisTaps::cached(source, target);
        let interior = &taps.sum_sq()[8..taps.sum_sq().len() - 8];
        let lo = interior.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = interior.iter().copied().fold(0.0f32, f32::max);
        hi / lo
    };

    let eyepiece = spread(3008, 1440);
    assert!(
        eyepiece < 1.05,
        "at 2.089x the tent alone should be nearly phase-invariant; measured {eyepiece:.3}x"
    );

    let near_unity = spread(1538, 1440);
    assert!(
        (1.15..1.45).contains(&near_unity),
        "at 1.068x the sharpen's negative lobes dominate `sum(w^2)`; measured \
         {near_unity:.3}x, and moving out of this band changes how good an \
         approximation a relative noise field is on IMX464"
    );
}
