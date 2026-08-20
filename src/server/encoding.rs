//! Image encoding utilities for streaming
//!
//! This module provides encoding functions for streaming image data
//! to WebSocket clients in various formats.

use crate::frame::sample_to_u8;

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
pub fn frame_to_rgb8_downsampled(
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_width: u32,
    max_height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let frame = &ready_frame.linear_frame;
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
        return Ok((expand_to_rgb8_fused(ready_frame), width as u32, height as u32));
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

    let rgb8 = box_downsample_to_rgb8_fused(ready_frame, target_width, target_height);
    Ok((rgb8, target_width as u32, target_height as u32))
}

/// Encode RGB8 data with LZ4 compression for high-speed streaming (legacy SA08 format)
pub fn encode_rgb8_lz4(ready_frame: &crate::server::state::RenderReadyFrame) -> Result<Vec<u8>, String> {
    use lz4_flex::block::{compress_into, get_maximum_output_size};

    let (rgb8_data, width, height) = frame_to_rgb8_downsampled(ready_frame, 3840, 2160)?;

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
pub fn encode_rgb8_lz4_chunked(ready_frame: &crate::server::state::RenderReadyFrame, chunk_count: usize) -> Result<Vec<u8>, String> {
    use rayon::prelude::*;

    let chunk_count = chunk_count.max(1);
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8_downsampled(ready_frame, 3840, 2160)?
    };

    encode_rgb8_lz4_chunked_from_u8(&rgb8_data, width, height, chunk_count)
}

/// Encode already-converted RGB8 data with parallel chunked LZ4 compression (SA09 format)
pub fn encode_rgb8_lz4_chunked_from_u8(rgb8_data: &[u8], width: u32, height: u32, chunk_count: usize) -> Result<Vec<u8>, String> {
    use rayon::prelude::*;

    let chunk_count = chunk_count.max(1);

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
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8_downsampled(ready_frame, max_w, max_h)?
    };

    encode_rgb8_jpeg_bounded_from_u8(&rgb8_data, width, height)
}

/// Encode already-converted RGB8 data as JPEG (SA10 format)
pub fn encode_rgb8_jpeg_bounded_from_u8(
    rgb8_data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {

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
    ready_frame: &crate::server::state::RenderReadyFrame,
    req_w: Option<u32>,
    req_h: Option<u32>,
) -> Result<Vec<u8>, String> {
    let (max_w, max_h) = clamp_client_resolution(req_w, req_h);
    encode_rgb8_jpeg_bounded(ready_frame, max_w, max_h)
}


#[cfg(test)]
mod tests {
    use crate::frame::Frame;

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
        stretch_result: Some(crate::server::state::StretchResult { black_point, scale_lut, color_intensity: 1.0 }),
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
        let encoded = encode_rgb8_jpeg_dynamic(&to_ready_frame(&frame), Some(5000), Some(5000)).unwrap();
        
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
        assert_eq!(clamp_client_resolution(Some(2560), Some(1440)), (2560, 1440));
        assert_eq!(clamp_client_resolution(Some(5000), Some(3000)), (3840, 2160));
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
            let (got, gw, gh) = frame_to_rgb8_downsampled(&to_ready_frame(&frame), box_w, box_h).unwrap();
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
                .install(|| frame_to_rgb8_downsampled(&to_ready_frame(&frame), 96, 54).unwrap().0)
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
        assert!(frame_to_rgb8_downsampled(&to_ready_frame(&small), 1920, 1080).is_err(), "native path");

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
                let (rgb, w, h) = frame_to_rgb8_downsampled(&to_ready_frame(&frame), box_w, box_h).unwrap();
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
}

pub fn expand_to_rgb8_fused(ready_frame: &crate::server::state::RenderReadyFrame) -> Vec<u8> {
    use rayon::prelude::*;
    use std::cell::RefCell;

    thread_local! {
        static ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    let frame = &ready_frame.linear_frame;
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let channels = frame.channels();
    let src_data = frame.data();

    let config = &ready_frame.pipeline_config;
    let has_stretch = config.auto_stretch && ready_frame.stretch_result.is_some();
    let has_saturate = config.saturation_boost;
    let has_contrast = config.contrast;

    let (black_point, scale_lut) = if has_stretch {
        let sr = ready_frame.stretch_result.as_ref().unwrap();
        (sr.black_point, sr.scale_lut.clone())
    } else {
        (0.0, std::sync::Arc::new(vec![]))
    };

    let row_len = width * 3;
    let mut output = vec![0u8; width * height * 3];

    output
        .par_chunks_mut(row_len)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, row_out)| {
            ROW_BUF.with(|cell| {
                let mut f32_row = cell.borrow_mut();
                f32_row.resize(row_len, 0.0);

                let row_start = y * width * channels;

                for x in 0..width {
                    let out_idx = x * 3;
                    if channels == 1 {
                        let val = src_data[row_start + x];
                        f32_row[out_idx] = val;
                        f32_row[out_idx + 1] = val;
                        f32_row[out_idx + 2] = val;
                    } else {
                        let in_idx = row_start + x * channels;
                        f32_row[out_idx] = src_data[in_idx];
                        f32_row[out_idx + 1] = src_data[in_idx + 1];
                        f32_row[out_idx + 2] = src_data[in_idx + 2];
                    }
                }

                if has_stretch {
                    crate::render::simd::apply_luminance_scale_lut_simd(&mut f32_row, black_point, &scale_lut, config.stretch_config.color_intensity);
                }
                if has_saturate {
                    if let Some(plugin) = crate::license::pro_plugin(&crate::render::stretch::saturation::SATURATION_PLUGIN) {
                        plugin.apply_boost_slice(&mut f32_row, &config.saturation_config);
                    }
                }
                if has_contrast {
                    crate::render::output::apply_contrast_slice(&mut f32_row, &config.contrast_config);
                }

                for i in 0..row_len {
                    row_out[i] = sample_to_u8(f32_row[i]);
                }
            });
        });

    output
}

/// Box-average `frame` to `target_width` x `target_height` in **linear light**, then apply
/// the tone-curve stretch (+ saturation/contrast) to the averaged result.
///
/// # Why stretch happens after downsampling, not before
///
/// This resamples before applying the non-linear, shadow-boosting tone curve, rather than
/// the reverse (which is what the pre-fusion pipeline did: stretch once at full resolution,
/// then downsample the already-stretched result). Because the stretch curves used here
/// (asinh, MTF) are concave, Jensen's inequality guarantees `curve(average(pixels)) >=
/// average(curve(pixels))` for any box of source pixels — so this order can only preserve
/// or brighten faint detail in a downsampled tier relative to the old order, never dim it.
/// See `test_downsample_then_stretch_is_at_least_as_bright_as_stretch_then_downsample` for a
/// pinned numerical example.
pub fn box_downsample_to_rgb8_fused(ready_frame: &crate::server::state::RenderReadyFrame, target_width: usize, target_height: usize) -> Vec<u8> {
    use rayon::prelude::*;
    use std::cell::RefCell;

    thread_local! {
        static DS_ROW_BUF: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    }

    let frame = &ready_frame.linear_frame;
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let src_data = frame.data();
    let src_stride = width * channels;

    let config = &ready_frame.pipeline_config;
    let has_stretch = config.auto_stretch && ready_frame.stretch_result.is_some();
    let has_saturate = config.saturation_boost;
    let has_contrast = config.contrast;

    let (black_point, scale_lut) = if has_stretch {
        let sr = ready_frame.stretch_result.as_ref().unwrap();
        (sr.black_point, sr.scale_lut.clone())
    } else {
        (0.0, std::sync::Arc::new(vec![]))
    };

    let x_scale = width as f32 / target_width as f32;
    let y_scale = height as f32 / target_height as f32;

    let col_ranges: Vec<(usize, usize, f32)> = (0..target_width)
        .map(|x| {
            let src_x0 = (x as f32 * x_scale) as usize;
            let src_x1 = (((x + 1) as f32 * x_scale) as usize).min(width);
            (src_x0, src_x1, 1.0 / (src_x1 - src_x0).max(1) as f32)
        })
        .collect();

    let row_len = target_width * 3;
    let mut output = vec![0u8; target_width * target_height * 3];

    output
        .par_chunks_mut(row_len)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y, row_out)| {
            DS_ROW_BUF.with(|cell| {
                let mut f32_row = cell.borrow_mut();
                f32_row.resize(row_len, 0.0);

                let src_y0 = (y as f32 * y_scale) as usize;
                let src_y1 = (((y + 1) as f32 * y_scale) as usize).min(height);
                let row_inv_area = 1.0 / (src_y1 - src_y0).max(1) as f32;

                for tgt_x in 0..target_width {
                    let (src_x0, src_x1, col_inv_area) = col_ranges[tgt_x];
                    let inv_area = row_inv_area * col_inv_area;

                    let mut acc = [0.0f32; 3];

                    for src_y in src_y0..src_y1 {
                        let row_start = src_y * src_stride;
                        for src_x in src_x0..src_x1 {
                            let src_idx = row_start + src_x * channels;
                            if channels == 1 {
                                acc[0] += src_data[src_idx];
                            } else {
                                acc[0] += src_data[src_idx];
                                acc[1] += src_data[src_idx + 1];
                                acc[2] += src_data[src_idx + 2];
                            }
                        }
                    }

                    let out_idx = tgt_x * 3;
                    if channels == 1 {
                        let val = acc[0] * inv_area;
                        f32_row[out_idx] = val;
                        f32_row[out_idx + 1] = val;
                        f32_row[out_idx + 2] = val;
                    } else {
                        f32_row[out_idx] = acc[0] * inv_area;
                        f32_row[out_idx + 1] = acc[1] * inv_area;
                        f32_row[out_idx + 2] = acc[2] * inv_area;
                    }
                }

                if has_stretch {
                    crate::render::simd::apply_luminance_scale_lut_simd(&mut f32_row, black_point, &scale_lut, config.stretch_config.color_intensity);
                }
                if has_saturate {
                    if let Some(plugin) = crate::license::pro_plugin(&crate::render::stretch::saturation::SATURATION_PLUGIN) {
                        plugin.apply_boost_slice(&mut f32_row, &config.saturation_config);
                    }
                }
                if has_contrast {
                    crate::render::output::apply_contrast_slice(&mut f32_row, &config.contrast_config);
                }

                for i in 0..row_len {
                    row_out[i] = sample_to_u8(f32_row[i]);
                }
            });
        });

    output
}

