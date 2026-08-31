/// Result of the autostretch solver
#[derive(Debug, Clone, Copy)]
pub struct AutoStretchResult {
    pub stretch_factor: f32,
    /// The background level the solver actually aimed the sky at.
    ///
    /// Not the same as `AutoStretchConfig::target_background`: a frame that is
    /// mostly signal has its target raised, so this is the only place the level
    /// the sky was *really* mapped to is recorded. The shadow floor anchors to
    /// it, which is why it has to be reported rather than recomputed.
    pub target_background: f32,
    pub midtones: [f32; 3],
    pub black_point: f32,
    pub original_median: f32,
    pub adjusted_median: f32,
    pub iterations: u32,
    pub converged: bool,
}

/// Estimate the fraction of pixels that likely contain signal (stars/nebulae)
///
/// Takes the luminance samples already gathered by `estimate_background_mode`, so this
/// costs a pass over ~50k floats instead of a strided walk over the whole frame.
///
/// The comparison is against the exact sample luminance, not a histogram bin. Binning
/// cannot support this test: the 4096-bin histogram has a bin width of ~2.4e-4 while a
/// typical `2 * sigma` is ~1.7e-4, so a whole bin is wider than the threshold offset and
/// the entire background distribution lands in one or two bins. Resolving the threshold
/// matters because the result gates the adaptive black point at 0.2 and 0.4.
pub fn estimate_signal_fraction(
    luminance_samples: &[f32],
    background_mode: f32,
    sigma: f32,
) -> f32 {
    if luminance_samples.is_empty() {
        return 0.0;
    }

    let threshold = background_mode + 2.0 * sigma;
    let signal_count = luminance_samples
        .iter()
        .filter(|&&lum| lum > threshold)
        .count();

    signal_count as f32 / luminance_samples.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_fraction_empty() {
        assert_eq!(estimate_signal_fraction(&[], 0.1, 0.01), 0.0);
    }

    #[test]
    fn test_signal_fraction_counts_above_threshold() {
        // threshold = 0.1 + 2 * 0.01 = 0.12; 2 of 5 samples are above it
        let samples = [0.05, 0.10, 0.119, 0.121, 0.9];
        let fraction = estimate_signal_fraction(&samples, 0.1, 0.01);
        assert!((fraction - 0.4).abs() < 1e-6, "got {fraction}");
    }

    #[test]
    fn test_signal_fraction_resolves_sub_histogram_bin_thresholds() {
        // Regression guard: sigma here is far smaller than a 4096-bin histogram bin
        // (2.4e-4), so a bin-quantised implementation cannot separate these samples and
        // collapses to 0.0 or 1.0. The mode sits at a bin boundary to make that worse.
        let mode = 23.0 / 4095.0;
        let sigma = 8.5e-5;
        let threshold = mode + 2.0 * sigma;
        let samples: Vec<f32> = (0..100)
            .map(|i| {
                if i < 30 {
                    threshold + 1e-6
                } else {
                    threshold - 1e-6
                }
            })
            .collect();

        let fraction = estimate_signal_fraction(&samples, mode, sigma);
        assert!((fraction - 0.30).abs() < 1e-6, "got {fraction}");
    }

    #[test]
    fn test_signal_fraction_all_background() {
        let samples = vec![0.005f32; 1000];
        assert_eq!(estimate_signal_fraction(&samples, 0.005, 0.0001), 0.0);
    }
}
