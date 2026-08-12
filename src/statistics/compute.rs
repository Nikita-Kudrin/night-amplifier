use super::channel::ChannelStats;
use super::ops::{compute_mad_in_place_simd, fast_median, min_max_simd};
use crate::frame::Frame;
use rayon::prelude::*;

/// Compute statistics for a single channel with SIMD optimization
pub(crate) fn compute_channel_stats(frame: &Frame, channel: usize, step: usize) -> ChannelStats {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let data = frame.data();
    let total_pixels = width * height;

    // For step=1 (full sampling), use optimized contiguous access
    if step == 1 && channels == 1 {
        // Monochrome with full sampling: data is contiguous
        let mut samples = data.to_vec();
        let (min_val, max_val) = min_max_simd(&samples);
        let median = fast_median(&mut samples);
        compute_mad_in_place_simd(&mut samples, median);
        let mad = fast_median(&mut samples);
        return ChannelStats::new(median, mad, min_val, max_val);
    }

    // Collect samples in parallel. The gather is strided, so every read is a cache miss
    // regardless; splitting it across cores hides that latency.
    let mut samples: Vec<f32> = (0..total_pixels)
        .into_par_iter()
        .step_by(step)
        .map(|pixel_idx| data[pixel_idx * channels + channel])
        .collect();

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
