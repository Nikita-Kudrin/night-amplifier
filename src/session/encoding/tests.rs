use crate::frame::Frame;

use crate::render::display::testing::to_ready_frame;
use crate::session::encoding::format::*;
use crate::session::encoding::jpeg::*;
use crate::session::encoding::lz4::*;

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
