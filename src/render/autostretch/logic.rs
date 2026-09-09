use super::config::AutoStretchConfig;
use super::solver::solve_stretch_factor_newton;
use super::stats::{estimate_signal_fraction, AutoStretchResult};
use crate::frame::Frame;
use crate::render::black_point::estimate_background_mode;
use crate::render::stretch::{estimate_tone_mapping_strength, ToneMappingAlgorithm};
use crate::statistics::ImageStats;

/// Smallest sky-above-black gap the solver will be asked to stretch, keeping the
/// stretch factor finite when the sky sits on the black point.
const MIN_EFFECTIVE_MEDIAN: f32 = 1e-4;

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

    // Floor the gap, then derive the black point from it.
    //
    // Flooring only `effective_median` let the solver be told the sky sits 1e-4 above
    // black while the black point actually applied put it at 5.7e-5 — the stretch was
    // solved for a sky 1.75x brighter than the one it rendered, and the error grows
    // with depth because the gap is `k * sigma`. Deriving both from one value keeps
    // them equal by construction.
    let gap = (adaptive_sigma * mean_sigma).max(MIN_EFFECTIVE_MEDIAN);
    let black_point = (mode - gap).max(0.0);
    let effective_median = (mode - black_point).max(MIN_EFFECTIVE_MEDIAN);

    // Everything the solve turns on, in one line. `signal_fraction` is measured
    // against `mode + 2 * mean_sigma`, so it moves with the *noise* as well as the
    // signal — a deepening stack shrinks sigma and can walk this across the 0.2/0.4
    // gates without the sky having changed at all.
    tracing::debug!(
        mode,
        mean_sigma,
        signal_fraction,
        adaptive_sigma,
        black_point,
        effective_median,
        "Auto-stretch inputs"
    );

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
        target_background,
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
        let background = 0.05;
        let mut frame = Frame::filled(64, 64, 3, background).unwrap();

        // Add some "stars". Written with `set_pixel`: the interleaved version put all
        // three samples in the red plane, so the stars were three adjacent red pixels
        // and the test passed only because the flat background is layout-invariant.
        for (sx, sy) in [(10usize, 10usize), (30, 30), (50, 50)] {
            frame.set_pixel(sx, sy, 0, 0.9);
            frame.set_pixel(sx, sy, 1, 0.85);
            frame.set_pixel(sx, sy, 2, 0.8);
        }
        let stats = compute_image_stats(&frame).unwrap();

        let config = AutoStretchConfig::new().with_black_point_sigma(0.5);
        let result = compute_auto_stretch(&frame, &stats, config);

        assert!(result.stretch_factor > 1.0);
        assert!(result.converged);
        assert!((result.original_median - background).abs() < 0.02);
    }

    /// Deterministic near-Gaussian sky, so `mean_sigma` is a known small number.
    fn sky_frame(width: usize, height: usize, level: f32, sigma: f32) -> Frame {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            let mut sum = 0.0f32;
            for _ in 0..12 {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                sum += (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 16_777_216.0;
            }
            sum - 6.0
        };
        let mut data = vec![0.0f32; width * height * 3];
        for v in data.iter_mut() {
            *v = (level + next() * sigma).max(0.0);
        }
        Frame::from_f32_vec(data, width, height, 3).unwrap()
    }

    /// The sky level the solver is told about must be the one the black point
    /// actually produces.
    ///
    /// The gap is `k * sigma`, so it shrinks as a stack deepens. Flooring only the
    /// value handed to the solver, and not the black point derived from the same
    /// quantity, makes the two disagree by more as integration grows: on a 71-frame
    /// field stack the solver stretched for a sky 1.75x brighter than the one the
    /// black point left, and the render dimmed steadily from 16 frames on.
    #[test]
    fn solver_sees_the_sky_the_black_point_actually_leaves() {
        // sigma small enough that k * sigma lands under the solver's floor.
        let frame = sky_frame(256, 256, 0.0024, 2e-5);
        let stats = compute_image_stats(&frame).unwrap();
        let config = AutoStretchConfig::new().with_black_point_sigma(2.0);

        let result = compute_auto_stretch(&frame, &stats, config);

        assert!(
            result.black_point > 0.0,
            "test needs a black point above the clamp, got {}",
            result.black_point
        );
        let actual_gap = result.original_median - result.black_point;
        assert!(
            (actual_gap - result.adjusted_median).abs() < 1e-9,
            "solver was told the sky sits {:.3e} above black; the black point leaves it {:.3e} ({:.2}x)",
            result.adjusted_median,
            actual_gap,
            result.adjusted_median / actual_gap
        );
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
