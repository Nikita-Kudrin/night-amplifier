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
    let background = estimate_background_mode(frame);
    let mode = background.mode;
    let mean_sigma = stats.mean_sigma();

    let signal_fraction = estimate_signal_fraction(&background.luminance_samples, mode, mean_sigma);

    let adaptive_sigma = if signal_fraction > 0.4 {
        (config.black_point_sigma * 0.6).max(1.5)
    } else if signal_fraction > 0.2 {
        config.black_point_sigma * 0.8
    } else {
        config.black_point_sigma
    };

    let black_point = (mode - adaptive_sigma * mean_sigma).max(0.0);
    let effective_median = (mode - black_point).max(1e-4);

    let target_background = if signal_fraction > 0.4 {
        (config.target_background * 1.3).min(0.20)
    } else {
        config.target_background
    };

    let mut midtones = [0.5, 0.5, 0.5];
    let mut w = 0.0;
    let mut eff_r = effective_median;
    let mut eff_g = effective_median;
    let mut eff_b = effective_median;

    if stats.channels.len() == 3 {
        let m_r = stats.channels[0].median;
        let m_g = stats.channels[1].median;
        let m_b = stats.channels[2].median;

        let m_avg = (m_r + m_g + m_b) / 3.0;
        // Only unlink if the background is bright enough to represent a real color cast
        // (e.g., > 0.005). If it's near zero (e.g. after background subtraction),
        // calculating divergence on residual noise will force unlinked stretching
        // and destroy the color balance (often turning the image green).
        if m_avg > 0.005 {
            let max_m = m_r.max(m_g).max(m_b);
            let min_m = m_r.min(m_g).min(m_b);
            let delta_rel = (max_m - min_m) / m_avg;

            w = ((delta_rel - 0.05) / (0.15 - 0.05)).clamp(0.0, 1.0);

            let adj_r = m_r - black_point;
            let adj_g = m_g - black_point;
            let adj_b = m_b - black_point;

            eff_r = adj_r.max(1e-4);
            eff_g = adj_g.max(1e-4);
            eff_b = adj_b.max(1e-4);
        }
    }

    let stretch_factor = match algorithm {
        ToneMappingAlgorithm::Asinh => {
            let adaptive_config = AutoStretchConfig {
                target_background,
                ..config
            };
            let result =
                solve_stretch_factor_newton(effective_median, target_background, &adaptive_config);
            midtones = [result.stretch_factor; 3];
            result.stretch_factor
        }
        ToneMappingAlgorithm::Mtf => {
            let m_linked =
                estimate_tone_mapping_strength(algorithm, effective_median, target_background);
            let m_r_un = estimate_tone_mapping_strength(algorithm, eff_r, target_background);
            let m_g_un = estimate_tone_mapping_strength(algorithm, eff_g, target_background);
            let m_b_un = estimate_tone_mapping_strength(algorithm, eff_b, target_background);

            midtones[0] = m_linked * (1.0 - w) + m_r_un * w;
            midtones[1] = m_linked * (1.0 - w) + m_g_un * w;
            midtones[2] = m_linked * (1.0 - w) + m_b_un * w;

            m_linked
        }
    };

    AutoStretchResult {
        stretch_factor,
        midtones,
        black_point,
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
        let mut data = vec![0.0f32; 64 * 64 * 3];
        for i in 0..data.len() {
            let rand = (i % 100) as f32 / 100.0;
            data[i] = 0.05 + rand * 0.1; // 0.05 to 0.15
        }

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
