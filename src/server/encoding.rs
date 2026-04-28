//! Image encoding utilities for streaming
//!
//! This module provides encoding functions for streaming image data
//! to WebSocket clients in various formats.

use crate::frame::Frame;

/// Binary header magic number for RGB8+LZ4 stream format
///
/// Header layout (16 bytes):
/// - bytes 0-3:   Magic number "SA08" (0x53413038)
/// - bytes 4-7:   Width (u32, little-endian)
/// - bytes 8-11:  Height (u32, little-endian)
/// - bytes 12-15: Compressed size (u32, little-endian)
///
/// Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
pub const RGB8_MAGIC: u32 = 0x53413038; // "SA08" in little-endian

/// Encode RGB8 data with LZ4 compression for high-speed streaming
pub fn encode_rgb8_lz4(frame: &Frame) -> Result<Vec<u8>, String> {
    use lz4_flex::block::{compress_into, get_maximum_output_size};
    use rayon::prelude::*;

    // Check if downsampling is needed (max 4K)
    let (process_frame, width, height) = if frame.width() > 3840 || frame.height() > 2160 {
        let aspect_ratio = frame.width() as f32 / frame.height() as f32;
        let (target_width, target_height) = if frame.width() > frame.height() {
            (3840, (3840.0 / aspect_ratio) as usize)
        } else {
            ((2160.0 * aspect_ratio) as usize, 2160)
        };

        let mut binned = Frame::zeros(target_width, target_height, frame.channels())
            .map_err(|e| e.to_string())?;

        let x_scale = frame.width() as f32 / target_width as f32;
        let y_scale = frame.height() as f32 / target_height as f32;

        binned
            .data_mut()
            .par_chunks_mut(target_width * frame.channels())
            .enumerate()
            .for_each(|(y, row)| {
                let src_y = ((y as f32 + 0.5) * y_scale) as usize;
                for x in 0..target_width {
                    let src_x = ((x as f32 + 0.5) * x_scale) as usize;
                    for c in 0..frame.channels() {
                        let idx = x * frame.channels() + c;
                        row[idx] = frame.get_pixel(src_x, src_y, c);
                    }
                }
            });
        (std::borrow::Cow::Owned(binned), target_width as u32, target_height as u32)
    } else {
        (std::borrow::Cow::Borrowed(frame), frame.width() as u32, frame.height() as u32)
    };

    let frame_ref = &*process_frame;

    let rgb8_data = if frame_ref.channels() == 1 {
        // 1. Try to debayer
        match crate::debayer::debayer_auto_with_algorithm(
            frame_ref,
            crate::debayer::DebayerAlgorithm::Bilinear,
        ) {
            Ok((rgb_frame, _)) => rgb_frame.to_rgb8_fast(),
            Err(_) => {
                // 2. Fallback: Duplicate mono data to standard RGB8
                let gray_data = frame_ref.data();
                let mut out = Vec::with_capacity(gray_data.len() * 3);
                
                // Using map instead of zip/for_each avoids zero-initialization of vec
                let rgb_flat: Vec<u8> = gray_data
                    .par_iter()
                    .flat_map_iter(|&v| {
                        let val = (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        [val, val, val]
                    })
                    .collect();
                out.extend_from_slice(&rgb_flat);
                out
            }
        }
    } else {
        frame_ref.to_rgb8_fast()
    };

    let uncompressed_len = rgb8_data.len() as u32;
    let max_compressed_len = get_maximum_output_size(rgb8_data.len());

    let mut output = vec![0u8; 16 + 4 + max_compressed_len];

    // Write header
    output[0..4].copy_from_slice(&RGB8_MAGIC.to_le_bytes());
    output[4..8].copy_from_slice(&width.to_le_bytes());
    output[8..12].copy_from_slice(&height.to_le_bytes());
    output[16..20].copy_from_slice(&uncompressed_len.to_le_bytes());

    let compressed_len = compress_into(&rgb8_data, &mut output[20..])
        .map_err(|e| format!("LZ4 compression error: {:?}", e))?;

    let final_payload_size = 4 + compressed_len;
    output.truncate(16 + final_payload_size);
    output[12..16].copy_from_slice(&(final_payload_size as u32).to_le_bytes());

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb8_lz4_encode_header_format() {
        let frame = Frame::filled(2, 2, 3, 0.5).unwrap();
        let encoded = encode_rgb8_lz4(&frame).unwrap();

        assert!(encoded.len() >= 16);
        let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(magic, RGB8_MAGIC);
        let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        assert_eq!(width, 2);
        let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        assert_eq!(height, 2);
        let compressed_size =
            u32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]);
        assert_eq!(compressed_size as usize, encoded.len() - 16);
    }

    #[test]
    fn test_rgb8_lz4_encode_decode_roundtrip() {
        use lz4_flex::decompress_size_prepended;
        let mut frame = Frame::zeros(4, 4, 3).unwrap();
        frame.set_pixel(0, 0, 0, 1.0);
        frame.set_pixel(1, 1, 1, 0.5);
        frame.set_pixel(2, 2, 2, 0.25);

        let encoded = encode_rgb8_lz4(&frame).unwrap();
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
        let encoded = encode_rgb8_lz4(&frame).unwrap();

        let raw_size = 100 * 100 * 3;
        let compressed_size = encoded.len() - 16;
        assert!(compressed_size < raw_size / 2);
    }

    #[test]
    fn test_rgb8_lz4_various_frame_sizes() {
        let test_cases = [(1, 1), (10, 10), (100, 50), (1920, 1080)];
        for (width, height) in test_cases {
            let frame = Frame::zeros(width, height, 3).unwrap();
            let encoded = encode_rgb8_lz4(&frame).unwrap();
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
        let encoded = encode_rgb8_lz4(&frame).unwrap();

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
}
