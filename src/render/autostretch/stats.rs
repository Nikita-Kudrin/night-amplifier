

/// Result of the autostretch solver
#[derive(Debug, Clone, Copy)]
pub struct AutoStretchResult {
    pub stretch_factor: f32,
    pub black_point: f32,
    pub original_median: f32,
    pub adjusted_median: f32,
    pub iterations: u32,
    pub converged: bool,
}

/// Estimate the fraction of pixels that likely contain signal (stars/nebulae)
pub fn estimate_signal_fraction(histogram: &[u32], background_mode: f32, sigma: f32) -> f32 {
    let threshold = background_mode + 2.0 * sigma;
    let threshold_bin = (threshold * (histogram.len() - 1) as f32).clamp(0.0, histogram.len() as f32) as usize;

    let mut signal_count = 0u64;
    let mut total_count = 0u64;

    for (i, &count) in histogram.iter().enumerate() {
        total_count += count as u64;
        if i > threshold_bin {
            signal_count += count as u64;
        }
    }

    if total_count == 0 {
        return 0.0;
    }

    signal_count as f32 / total_count as f32
}
