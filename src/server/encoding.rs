//! Image encoding utilities for streaming
//!
//! This module provides encoding functions for streaming image data
//! to WebSocket clients in various formats.

use crate::frame::{sample_to_u8, Frame};

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

/// Binary header magic number for JPEG stream format (SA10)
///
/// Header (16 bytes):
/// - bytes 0-3:    Magic "SA10" (0x53413130)
/// - bytes 4-7:    Width (u32 LE)
/// - bytes 8-11:   Height (u32 LE)
/// - bytes 12-15:  Payload size (u32 LE)
/// Followed by raw JPEG bytes
pub const JPEG_MAGIC: u32 = 0x53413130; // "SA10" in little-endian

const SA09_HEADER_SIZE: usize = 20;
const SA09_CHUNK_DESCRIPTOR_SIZE: usize = 8;
const SA10_HEADER_SIZE: usize = 16;

/// Smallest bounding box a JPEG client may ask for.
pub const JPEG_MIN_BOUNDING_BOX: (u32, u32) = (1920, 1080);
/// Largest bounding box a JPEG client may ask for.
pub const JPEG_MAX_BOUNDING_BOX: (u32, u32) = (3840, 2160);

/// Clamp a client-requested viewport to the streamable JPEG range.
///
/// Single source of truth for the bounds: resolution tiers and the encoder
/// both derive from it, so a request always maps to the tier that is actually
/// encoded.
pub fn clamp_client_resolution(req_w: Option<u32>, req_h: Option<u32>) -> (u32, u32) {
    let (min_w, min_h) = JPEG_MIN_BOUNDING_BOX;
    let (max_w, max_h) = JPEG_MAX_BOUNDING_BOX;
    (
        req_w.unwrap_or(min_w).clamp(min_w, max_w),
        req_h.unwrap_or(min_h).clamp(min_h, max_h),
    )
}

/// Convert a Frame to RGB8 data, box-averaging down to a bounding box if needed.
///
/// # Why there is no debayering here
///
/// A 1-channel frame reaching this function is genuine monochrome, never a raw
/// CFA mosaic: every provider debayers colour sensors at capture — `from_bayer`
/// in `camera/{zwo,qhy,playerone,svbony,touptek}` and `SerColorId::is_bayer` in
/// `ser/reader.rs` — while mono sensors take the `from_raw` path at
/// `channels = 1`, and the simulator only builds a `Debayerer` for
/// `SensorType::Color`. Nothing between capture and here changes the channel
/// count.
///
/// So mono channels are replicated across RGB. The previous code instead ran
/// `detect_cfa_pattern` (which never errors for a 1-channel frame ≥ 4x4, and
/// whose confidence was discarded) and debayered unconditionally, which cost a
/// full-resolution f32 RGB frame — 3x the mono source, ~196 MB on an
/// ASI1600MM — per tier per frame, and put colour fringing on grey data.
fn frame_to_rgb8(
    frame: &Frame,
    max_width: u32,
    max_height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    if channels != 1 && channels != 3 {
        return Err(format!(
            "Unsupported channel count for RGB8 conversion: {}",
            channels
        ));
    }

    if width <= max_width as usize && height <= max_height as usize {
        return Ok((expand_to_rgb8(frame), width as u32, height as u32));
    }

    let aspect_ratio = width as f32 / height as f32;
    let (target_width, target_height) =
        if width as f32 / max_width as f32 > height as f32 / max_height as f32 {
            (
                max_width as usize,
                (max_width as f32 / aspect_ratio) as usize,
            )
        } else {
            (
                (max_height as f32 * aspect_ratio) as usize,
                max_height as usize,
            )
        };
    let target_width = target_width.max(1);
    let target_height = target_height.max(1);

    let rgb8 = box_downsample_to_rgb8(frame, target_width, target_height);
    Ok((rgb8, target_width as u32, target_height as u32))
}

/// Convert a frame to RGB8 at its native size, replicating a mono channel.
fn expand_to_rgb8(frame: &Frame) -> Vec<u8> {
    use rayon::prelude::*;

    if frame.channels() == 1 {
        frame
            .data()
            .par_iter()
            .flat_map_iter(|&v| {
                let val = sample_to_u8(v);
                [val, val, val]
            })
            .collect()
    } else {
        frame.to_rgb8_fast()
    }
}

/// Box-average `frame` to `target_width` x `target_height`, emitting RGB8 directly.
///
/// Folding the f32 → u8 conversion into the accumulation pass keeps the
/// intermediate downsampled frame from ever materialising, which is the point of
/// the exercise: it was a 24.7 MB write plus a 24.7 MB read per frame.
///
/// Caller guarantees `channels` is 1 or 3, and that both target dimensions are
/// non-zero and no larger than the source, so every source range is non-empty.
fn box_downsample_to_rgb8(frame: &Frame, target_width: usize, target_height: usize) -> Vec<u8> {
    use rayon::prelude::*;

    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let src_data = frame.data();
    let src_stride = width * channels;

    let x_scale = width as f32 / target_width as f32;
    let y_scale = height as f32 / target_height as f32;

    // Column ranges are identical for every row, so resolve them once per frame
    // rather than once per output sample. The reciprocal is stored instead of the
    // count so the per-pixel area factor is a multiply rather than a divide —
    // that is ~2 M divides saved per 1080p tier per frame.
    let col_ranges: Vec<(usize, usize, f32)> = (0..target_width)
        .map(|x| {
            let src_x0 = (x as f32 * x_scale) as usize;
            let src_x1 = (((x + 1) as f32 * x_scale) as usize).min(width);
            (src_x0, src_x1, 1.0 / (src_x1 - src_x0).max(1) as f32)
        })
        .collect();

    let mut output = vec![0u8; target_width * target_height * 3];

    output
        .par_chunks_mut(target_width * 3)
        .enumerate()
        .for_each(|(y, row_out)| {
            let src_y0 = (y as f32 * y_scale) as usize;
            let src_y1 = (((y + 1) as f32 * y_scale) as usize).min(height);
            let inv_y_count = 1.0 / (src_y1 - src_y0).max(1) as f32;

            for (x, &(src_x0, src_x1, inv_x_count)) in col_ranges.iter().enumerate() {
                let inv_area = inv_y_count * inv_x_count;
                let out_idx = x * 3;

                if channels == 1 {
                    let mut sum = 0.0f32;
                    for sy in src_y0..src_y1 {
                        let row_start = sy * src_stride;
                        for sx in src_x0..src_x1 {
                            sum += src_data[row_start + sx];
                        }
                    }
                    let val = sample_to_u8(sum * inv_area);
                    row_out[out_idx] = val;
                    row_out[out_idx + 1] = val;
                    row_out[out_idx + 2] = val;
                } else {
                    // channels is exactly 3 — see the caller's guarantee
                    let mut sum_r = 0.0f32;
                    let mut sum_g = 0.0f32;
                    let mut sum_b = 0.0f32;
                    for sy in src_y0..src_y1 {
                        let row_start = sy * src_stride;
                        for sx in src_x0..src_x1 {
                            let pixel_start = row_start + sx * 3;
                            sum_r += src_data[pixel_start];
                            sum_g += src_data[pixel_start + 1];
                            sum_b += src_data[pixel_start + 2];
                        }
                    }
                    row_out[out_idx] = sample_to_u8(sum_r * inv_area);
                    row_out[out_idx + 1] = sample_to_u8(sum_g * inv_area);
                    row_out[out_idx + 2] = sample_to_u8(sum_b * inv_area);
                }
            }
        });

    output
}

/// Encode RGB8 data with LZ4 compression for high-speed streaming (legacy SA08 format)
pub fn encode_rgb8_lz4(frame: &Frame) -> Result<Vec<u8>, String> {
    use lz4_flex::block::{compress_into, get_maximum_output_size};

    let (rgb8_data, width, height) = frame_to_rgb8(frame, 3840, 2160)?;

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
        frame_to_rgb8(frame, 3840, 2160)?
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

fn calculate_dynamic_jpeg_quality(width: u32, height: u32) -> i32 {
    let smallest_side = width.min(height);
    // If resolution is lower than 2K (1440p), use 95% quality.
    // Otherwise, default to 90% quality.
    if smallest_side < 1440 {
        95
    } else {
        90
    }
}

thread_local! {
    /// Reused TurboJPEG compressor. The render task encodes one payload per
    /// active resolution tier per frame, so keeping the compressor alive avoids
    /// re-allocating libjpeg-turbo's internal buffers on every encode.
    static JPEG_COMPRESSOR: std::cell::RefCell<Option<turbojpeg::Compressor>> =
        const { std::cell::RefCell::new(None) };
}

fn configure_compressor(
    compressor: &mut turbojpeg::Compressor,
    quality: i32,
) -> Result<(), String> {
    compressor
        .set_quality(quality)
        .map_err(|e| format!("TurboJPEG set_quality failed: {}", e))?;
    compressor
        .set_subsamp(turbojpeg::Subsamp::Sub2x2)
        .map_err(|e| format!("TurboJPEG set_subsamp failed: {}", e))
}

fn compress_rgb8_to_jpeg(rgb8_data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let quality = calculate_dynamic_jpeg_quality(width, height);
    let image = turbojpeg::Image {
        pixels: rgb8_data,
        width: width as usize,
        pitch: 3 * width as usize,
        height: height as usize,
        format: turbojpeg::PixelFormat::RGB,
    };

    JPEG_COMPRESSOR.with(|slot| {
        // A re-entrant call would find the slot already borrowed; fall back to a
        // throwaway compressor instead of panicking.
        let Ok(mut borrowed) = slot.try_borrow_mut() else {
            let mut compressor = turbojpeg::Compressor::new().map_err(|e| e.to_string())?;
            configure_compressor(&mut compressor, quality)?;
            return compressor.compress_to_vec(image).map_err(|e| e.to_string());
        };

        if borrowed.is_none() {
            *borrowed = Some(turbojpeg::Compressor::new().map_err(|e| e.to_string())?);
        }
        let Some(compressor) = borrowed.as_mut() else {
            return Err("TurboJPEG compressor unavailable".to_string());
        };
        configure_compressor(compressor, quality)?;
        compressor.compress_to_vec(image).map_err(|e| e.to_string())
    })
}

/// Encode a frame as JPEG (SA10 format) fitted into an exact bounding box.
///
/// The box is used verbatim, which lets the `Original` resolution tier stream a
/// frame at its native size. Clients go through [`encode_rgb8_jpeg_dynamic`],
/// which clamps the request first.
pub fn encode_rgb8_jpeg_bounded(
    frame: &Frame,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8(frame, max_w, max_h)?
    };

    let compressed = {
        let _span = tracing::info_span!("jpeg_compress").entered();
        compress_rgb8_to_jpeg(&rgb8_data, width, height)?
    };

    let payload_size = compressed.len() as u32;
    let mut output = Vec::with_capacity(SA10_HEADER_SIZE + compressed.len());
    output.extend_from_slice(&JPEG_MAGIC.to_le_bytes());
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&payload_size.to_le_bytes());
    output.extend_from_slice(&compressed);

    Ok(output)
}

/// Encode frame as JPEG at a client-requested resolution (SA10 format)
pub fn encode_rgb8_jpeg_dynamic(
    frame: &Frame,
    req_w: Option<u32>,
    req_h: Option<u32>,
) -> Result<Vec<u8>, String> {
    let (max_w, max_h) = clamp_client_resolution(req_w, req_h);
    encode_rgb8_jpeg_bounded(frame, max_w, max_h)
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
        let encoded = encode_rgb8_jpeg_dynamic(&frame, Some(5000), Some(5000)).unwrap();
        
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
        let encoded = encode_rgb8_jpeg_dynamic(&frame, Some(640), Some(480)).unwrap();

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
        assert_eq!(clamp_client_resolution(Some(2560), Some(1440)), (2560, 1440));
        assert_eq!(clamp_client_resolution(Some(5000), Some(3000)), (3840, 2160));
    }

    #[test]
    fn test_jpeg_encode_bounded_keeps_native_resolution() {
        let frame = Frame::zeros(200, 120, 3).unwrap();
        let encoded = encode_rgb8_jpeg_bounded(&frame, u32::MAX, u32::MAX).unwrap();

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
        let encoded = encode_rgb8_jpeg_bounded(&frame, 1920, 1080).unwrap();

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

        let boxed = encode_rgb8_jpeg_bounded(&frame, 1920, 1080).unwrap();
        let native = encode_rgb8_jpeg_bounded(&frame, u32::MAX, u32::MAX).unwrap();
        let boxed_again = encode_rgb8_jpeg_bounded(&frame, 1920, 1080).unwrap();

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
            let (got, gw, gh) = frame_to_rgb8(&frame, box_w, box_h).unwrap();
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
        let (rgb, w, h) = frame_to_rgb8(&frame, 137, 111).unwrap();

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
        let (rgb, w, h) = frame_to_rgb8(&frame, 1920, 1080).unwrap();

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
                .install(|| frame_to_rgb8(&frame, 96, 54).unwrap().0)
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
        assert!(frame_to_rgb8(&small, 1920, 1080).is_err(), "native path");

        let large = Frame::zeros(4000, 3000, 4).unwrap();
        assert!(
            frame_to_rgb8(&large, 1920, 1080).is_err(),
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
                let (rgb, w, h) = frame_to_rgb8(&frame, box_w, box_h).unwrap();
                assert_eq!(
                    rgb.len(),
                    w as usize * h as usize * 3,
                    "channels={channels} box={box_w}x{box_h}"
                );
                assert!(w <= box_w.max(271) && h <= box_h.max(153));
            }
        }
    }
}
