use super::channel::ChannelStats;
use super::ops::{compute_mad_in_place_simd, fast_median, min_max_simd};
use crate::frame::Frame;

/// Compute statistics for a single channel with SIMD optimization
pub(crate) fn compute_channel_stats(frame: &Frame, channel: usize, step: usize) -> ChannelStats {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let data = frame.data();
    let total_pixels = width * height;

    // Estimate sample count and pre-allocate
    let estimated_samples = total_pixels / step + 1;
    let mut samples = Vec::with_capacity(estimated_samples);

    // For step=1 (full sampling), use optimized contiguous access
    if step == 1 && channels == 1 {
        // Monochrome with full sampling: data is contiguous
        samples.extend_from_slice(data);
        let (min_val, max_val) = min_max_simd(&samples);
        let median = fast_median(&mut samples);
        compute_mad_in_place_simd(&mut samples, median);
        let mad = fast_median(&mut samples);
        return ChannelStats::new(median, mad, min_val, max_val);
    }

    // For step=1 with multiple channels, use strided but optimized access
    if step == 1 {
        // Process in chunks for better cache utilization
        let chunk_size = 1024;
        for chunk_start in (0..total_pixels).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size).min(total_pixels);

            for pixel_idx in chunk_start..chunk_end {
                let data_idx = pixel_idx * channels + channel;
                let value = data[data_idx];
                samples.push(value);
            }
        }

        // Compute min/max using SIMD on the collected samples
        let (min_val, max_val) = min_max_simd(&samples);

        let median = fast_median(&mut samples);
        compute_mad_in_place_simd(&mut samples, median);
        let mad = fast_median(&mut samples);

        return ChannelStats::new(median, mad, min_val, max_val);
    }

    // For sampling (step > 1), use batch collection
    let batch_size = 256;

    // Pre-compute all pixel indices to sample
    let sample_indices: Vec<usize> = (0..total_pixels).step_by(step).collect();

    // Process in batches
    for batch in sample_indices.chunks(batch_size) {
        for &pixel_idx in batch {
            let data_idx = pixel_idx * channels + channel;
            let value = data[data_idx];
            samples.push(value);
        }
    }

    if samples.is_empty() {
        return ChannelStats::new(0.0, 0.0, 0.0, 0.0);
    }

    // Compute min/max using SIMD
    let (min_val, max_val) = min_max_simd(&samples);

    // Compute median using partial sort
    let median = fast_median(&mut samples);

    // Compute MAD in-place using SIMD for absolute deviations
    compute_mad_in_place_simd(&mut samples, median);
    let mad = fast_median(&mut samples);

    ChannelStats::new(median, mad, min_val, max_val)
}
