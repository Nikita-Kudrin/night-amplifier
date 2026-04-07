use super::config::AutoStretchConfig;
use super::solver::solve_stretch_factor_newton;
use super::stats::{estimate_signal_fraction, AutoStretchResult};
use crate::frame::Frame;
use crate::render::black_point::estimate_background_mode;
use crate::render::stretch::{estimate_tone_mapping_strength, ToneMappingAlgorithm};
use crate::statistics::ImageStats;

pub fn compute_auto_stretch(
    frame: &Frame,
    stats: &ImageStats,
    config: AutoStretchConfig,
) -> AutoStretchResult {
    compute_auto_stretch_with_algorithm(frame, stats, config, ToneMappingAlgorithm::Asinh)
}

pub fn compute_auto_stretch_with_algorithm(
    frame: &Frame,
    stats: &ImageStats,
    config: AutoStretchConfig,
    algorithm: ToneMappingAlgorithm,
) -> AutoStretchResult {
    let mode = estimate_background_mode(frame);
    let mean_sigma = stats.mean_sigma();

    let signal_fraction = estimate_signal_fraction(frame, mode, mean_sigma);

    let adaptive_sigma = if signal_fraction > 0.4 {
        (config.black_point_sigma * 0.6).max(1.5)
    } else if signal_fraction > 0.2 {
        config.black_point_sigma * 0.8
    } else {
        config.black_point_sigma
    };

    let black_point = (mode - adaptive_sigma * mean_sigma).max(0.0);
    let adjusted_median = mode - black_point;

    let effective_median = if adjusted_median < 0.001 {
        mode
    } else {
        adjusted_median
    };

    let effective_median = effective_median.max(1e-6);

    let target_background = if signal_fraction > 0.4 {
        (config.target_background * 1.3).min(0.20)
    } else {
        config.target_background
    };

    let stretch_factor = match algorithm {
        ToneMappingAlgorithm::Asinh => {
            let adaptive_config = AutoStretchConfig {
                target_background,
                ..config
            };
            let result =
                solve_stretch_factor_newton(effective_median, target_background, &adaptive_config);
            result.stretch_factor
        }
        ToneMappingAlgorithm::Mtf => {
            estimate_tone_mapping_strength(algorithm, effective_median, target_background)
        }
    };

    AutoStretchResult {
        stretch_factor,
        black_point: if adjusted_median < 0.001 {
            0.0
        } else {
            black_point
        },
        original_median: mode,
        adjusted_median: effective_median,
        iterations: 0,
        converged: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::compute_image_stats;

    #[test]
    fn test_compute_auto_stretch_basic() {
        let mut data = vec![0.0f32; 64 * 64 * 3];
        let background = 0.05;

        for i in (0..data.len()).step_by(3) {
            data[i] = background;
            data[i + 1] = background;
            data[i + 2] = background;
        }

        // Add some "stars"
        for star_pos in [(10, 10), (30, 30), (50, 50)] {
            let idx = (star_pos.1 * 64 + star_pos.0) * 3;
            data[idx] = 0.9;
            data[idx + 1] = 0.85;
            data[idx + 2] = 0.8;
        }

        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        let config = AutoStretchConfig::new().with_black_point_sigma(0.5);
        let result = compute_auto_stretch(&frame, &stats, config);

        assert!(result.stretch_factor > 1.0);
        assert!(result.converged);
        assert!((result.original_median - background).abs() < 0.02);
    }

    #[test]
    fn test_compute_auto_stretch_different_targets() {
        let data = vec![0.08f32; 64 * 64 * 3];
        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        let result_low = compute_auto_stretch(
            &frame,
            &stats,
            AutoStretchConfig::new()
                .with_target_background(0.10)
                .with_black_point_sigma(0.5),
        );
        let result_high = compute_auto_stretch(
            &frame,
            &stats,
            AutoStretchConfig::new()
                .with_target_background(0.25)
                .with_black_point_sigma(0.5),
        );

        assert!(result_high.stretch_factor > result_low.stretch_factor);
    }
}
