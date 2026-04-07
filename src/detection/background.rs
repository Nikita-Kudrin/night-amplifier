/// Background statistics for the image
#[derive(Debug, Clone, Copy)]
pub struct BackgroundStats {
    /// Estimated background level (median)
    pub median: f32,
    /// Estimated noise level (MAD-based sigma)
    pub sigma: f32,
    /// Detection threshold (median + n*sigma)
    pub threshold: f32,
}

impl BackgroundStats {
    /// Estimates background level and noise using robust statistics
    ///
    /// Uses median for background (robust to stars) and MAD for noise
    /// (Median Absolute Deviation, scaled to match Gaussian sigma)
    pub fn estimate(data: &[f32], sigma_threshold: f32) -> Self {
        let sample_size = 100_000.min(data.len());
        let step = data.len() / sample_size;

        let mut sample: Vec<f32> = data.iter().step_by(step.max(1)).copied().collect();
        sample.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let median = compute_median(&sample);
        let sigma = compute_mad_sigma(&sample, median);
        let threshold = median + sigma_threshold * sigma;

        Self {
            median,
            sigma,
            threshold,
        }
    }
}

fn compute_median(sorted_sample: &[f32]) -> f32 {
    let len = sorted_sample.len();
    if len % 2 == 0 {
        (sorted_sample[len / 2 - 1] + sorted_sample[len / 2]) / 2.0
    } else {
        sorted_sample[len / 2]
    }
}

fn compute_mad_sigma(sorted_sample: &[f32], median: f32) -> f32 {
    let mut deviations: Vec<f32> = sorted_sample.iter().map(|&v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mad = compute_median(&deviations);
    (mad * 1.4826).max(1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_estimation() {
        let data = vec![0.1f32; 10000];
        let stats = BackgroundStats::estimate(&data, 5.0);

        assert!((stats.median - 0.1).abs() < 0.01);
        assert!(stats.sigma < 0.01);
    }
}
