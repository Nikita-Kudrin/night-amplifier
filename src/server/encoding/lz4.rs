use lz4_flex::compress_prepend_size;

use crate::server::encoding::format::*;
use crate::server::encoding::fused::frame_to_rgb8_downsampled;

pub fn encode_rgb8_lz4(
    ready_frame: &crate::server::state::RenderReadyFrame,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    use lz4_flex::block::{compress_into, get_maximum_output_size};

    let (rgb8_data, width, height) = frame_to_rgb8_downsampled(ready_frame, max_w, max_h)?;

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
pub fn encode_rgb8_lz4_chunked(
    ready_frame: &crate::server::state::RenderReadyFrame,
    chunk_count: usize,
    max_w: u32,
    max_h: u32,
) -> Result<Vec<u8>, String> {
    use rayon::prelude::*;

    let chunk_count = chunk_count.max(1);
    let (rgb8_data, width, height) = {
        let _span = tracing::info_span!("frame_to_rgb8").entered();
        frame_to_rgb8_downsampled(ready_frame, max_w, max_h)?
    };

    encode_rgb8_lz4_chunked_from_u8(&rgb8_data, width, height, chunk_count)
}

/// Encode already-converted RGB8 data with parallel chunked LZ4 compression (SA09 format)
pub fn encode_rgb8_lz4_chunked_from_u8(
    rgb8_data: &[u8],
    width: u32,
    height: u32,
    chunk_count: usize,
) -> Result<Vec<u8>, String> {
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
