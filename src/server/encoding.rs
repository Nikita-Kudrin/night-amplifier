//! Image encoding utilities for streaming
//!
//! This module provides encoding functions for streaming image data
//! to WebSocket clients in various formats.

use crate::frame::Frame;

/// Binary header magic number for RGB8+LZ4 stream format (legacy single-block)
///
/// Header layout (16 bytes):
/// - bytes 0-3:   Magic number "SA08" (0x53413038)
/// - bytes 4-7:   Width (u32, little-endian)
/// - bytes 8-11:  Height (u32, little-endian)
/// - bytes 12-15: Compressed size (u32, little-endian)
///
/// Followed by LZ4-compressed RGB8 pixel data (3 bytes per pixel)
pub const RGB8_MAGIC: u32 = 0x53413038; // "SA08" in little-endian

/// Binary header magic number for chunked RGB8+LZ4 stream format
///
/// Header (20 bytes):
/// - bytes 0-3:    Magic "SA09" (0x53413039)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Total payload size (u32 LE) — everything after header
/// - bytes 16-19:  Chunk count (u32 LE)
///
/// Per-chunk descriptor (8 bytes each, chunk_count entries):
/// - bytes 0-3:    Compressed size of this chunk (u32 LE)
/// - bytes 4-7:    Decompressed size of this chunk (u32 LE)
///
/// Followed by concatenated compressed chunk data
pub const RGB8_CHUNKED_MAGIC: u32 = 0x53413039; // "SA09" in little-endian

const SA09_HEADER_SIZE: usize = 20;
const SA09_CHUNK_DESCRIPTOR_SIZE: usize = 8;

/// Convert a Frame to RGB8 data, handling downsampling and debayering
fn frame_to_rgb8(frame: &Frame) -> Result<(Vec<u8>, u32, u32), String> {
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
        match crate::debayer::detect_cfa_pattern(frame_ref) {
            Ok(detection) => {
                crate::debayer::debayer_bilinear_to_rgb8_fast(frame_ref, detection.pattern)
                    .unwrap_or_else(|_| {
                        let gray_data = frame_ref.data();
                        gray_data
                            .par_iter()
                            .flat_map_iter(|&v| {
                                let val = (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                                [val, val, val]
                            })
                            .collect()
                    })
            }
            Err(_) => {
                let gray_data = frame_ref.data();
                gray_data
                    .par_iter()
                    .flat_map_iter(|&v| {
                        let val = (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        [val, val, val]
                    })
                    .collect()
            }
        }
    } else {
        frame_ref.to_rgb8_fast()
    };

    Ok((rgb8_data, width, height))
}

/// Encode RGB8 data with LZ4 compression for high-speed streaming (legacy SA08 format)
pub fn encode_rgb8_lz4(frame: &Frame) -> Result<Vec<u8>, String> {
    use lz4_flex::block::{compress_into, get_maximum_output_size};

    let (rgb8_data, width, height) = frame_to_rgb8(frame)?;

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

/// Encode RGB8 data with parallel chunked LZ4 compression (SA09 format)
///
/// Splits the image into `chunk_count` horizontal row-stripes and compresses
/// each independently via Rayon. When `chunk_count == 1`, produces a single
/// chunk (sequential, yields CPU to other tasks like stacking).
pub fn encode_rgb8_lz4_chunked(frame: &Frame, chunk_count: usize) -> Result<Vec<u8>, String> {
    use rayon::prelude::*;

    let chunk_count = chunk_count.max(1);
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8(frame)?
    };

    let row_bytes = width as usize * 3;
    let total_rows = height as usize;

    // Split into row-stripes
    let rows_per_chunk = total_rows / chunk_count;
    let remainder_rows = total_rows % chunk_count;

    // Compute stripe boundaries (some chunks get one extra row to handle remainder)
    let mut stripe_ranges: Vec<(usize, usize)> = Vec::with_capacity(chunk_count);
    let mut row_offset = 0;
    for i in 0..chunk_count {
        let rows = rows_per_chunk + if i < remainder_rows { 1 } else { 0 };
        let byte_start = row_offset * row_bytes;
        let byte_end = (row_offset + rows) * row_bytes;
        stripe_ranges.push((byte_start, byte_end));
        row_offset += rows;
    }

    // Compress each stripe in parallel
    let compressed_chunks: Vec<Vec<u8>> = {
        let _span = tracing::info_span!("lz4_compress_parallel", chunk_count).entered();
        stripe_ranges
            .par_iter()
            .map(|&(start, end)| {
                let stripe = &rgb8_data[start..end];
                lz4_flex::compress(stripe)
            })
            .collect()
    };

    // Compute output size
    let descriptors_size = chunk_count * SA09_CHUNK_DESCRIPTOR_SIZE;
    let compressed_total: usize = compressed_chunks.iter().map(|c| c.len()).sum();
    let payload_size = descriptors_size + compressed_total;
    let total_size = SA09_HEADER_SIZE + payload_size;

    let mut output = vec![0u8; total_size];

    // Write header
    output[0..4].copy_from_slice(&RGB8_CHUNKED_MAGIC.to_le_bytes());
    output[4..8].copy_from_slice(&width.to_le_bytes());
    output[8..12].copy_from_slice(&height.to_le_bytes());
    output[12..16].copy_from_slice(&(payload_size as u32).to_le_bytes());
    output[16..20].copy_from_slice(&(chunk_count as u32).to_le_bytes());

    // Write chunk descriptors and data
    let mut desc_offset = SA09_HEADER_SIZE;
    let mut data_offset = SA09_HEADER_SIZE + descriptors_size;

    for (i, compressed) in compressed_chunks.iter().enumerate() {
        let (start, end) = stripe_ranges[i];
        let decompressed_size = (end - start) as u32;
        let compressed_size = compressed.len() as u32;

        // Descriptor
        output[desc_offset..desc_offset + 4].copy_from_slice(&compressed_size.to_le_bytes());
        output[desc_offset + 4..desc_offset + 8].copy_from_slice(&decompressed_size.to_le_bytes());
        desc_offset += SA09_CHUNK_DESCRIPTOR_SIZE;

        // Data
        output[data_offset..data_offset + compressed.len()].copy_from_slice(compressed);
        data_offset += compressed.len();
    }

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

    // --- SA09 Chunked Format Tests ---

    /// Decode a SA09 chunked message back to raw RGB8 for test verification
    fn decode_sa09(encoded: &[u8]) -> (u32, u32, Vec<u8>) {
        assert!(encoded.len() >= SA09_HEADER_SIZE);
        let magic = u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(magic, RGB8_CHUNKED_MAGIC);

        let width = u32::from_le_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
        let height = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        let chunk_count = u32::from_le_bytes([encoded[16], encoded[17], encoded[18], encoded[19]]) as usize;

        let descriptors_size = chunk_count * SA09_CHUNK_DESCRIPTOR_SIZE;
        let mut decompressed = Vec::new();
        let mut data_offset = SA09_HEADER_SIZE + descriptors_size;

        for i in 0..chunk_count {
            let desc_offset = SA09_HEADER_SIZE + i * SA09_CHUNK_DESCRIPTOR_SIZE;
            let compressed_size = u32::from_le_bytes([
                encoded[desc_offset], encoded[desc_offset + 1],
                encoded[desc_offset + 2], encoded[desc_offset + 3],
            ]) as usize;
            let decompressed_size = u32::from_le_bytes([
                encoded[desc_offset + 4], encoded[desc_offset + 5],
                encoded[desc_offset + 6], encoded[desc_offset + 7],
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
        let encoded = encode_rgb8_lz4_chunked(&frame, 2).unwrap();

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

        let encoded = encode_rgb8_lz4_chunked(&frame, 4).unwrap();
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
        let encoded = encode_rgb8_lz4_chunked(&frame, 1).unwrap();

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
            let encoded = encode_rgb8_lz4_chunked(&frame, chunks).unwrap();
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

        let sa08 = encode_rgb8_lz4(&frame).unwrap();
        let sa08_pixels = decompress_size_prepended(&sa08[16..]).unwrap();

        let sa09 = encode_rgb8_lz4_chunked(&frame, 4).unwrap();
        let (_, _, sa09_pixels) = decode_sa09(&sa09);

        assert_eq!(sa08_pixels, sa09_pixels);
    }
}
