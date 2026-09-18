use super::config::AutoStretchConfig;
use super::solver::solve_stretch_factor_newton;
use super::stats::{estimate_signal_fraction, AutoStretchResult};
use crate::frame::Frame;
use crate::render::black_point::estimate_background_mode;
use crate::render::stretch::{estimate_tone_mapping_strength, ToneMappingAlgorithm};
use crate::statistics::ImageStats;

/// Smallest sky-above-black gap the solver will be asked to stretch, keeping the
/// stretch factor finite when the sky sits on the black point.
///
/// A numerical guard and nothing more. It used to be `1e-4` — about 6.5 ADU of a
/// 16-bit frame — which on an IMX533 deep-sky stack is larger than `k * sigma` from
/// roughly 16 subs on, so past that depth the floor, not the solve, set the black
/// point. That accident was the only reason a deeper stack ever looked smoother
/// (grain 4.4 -> 1.5 output levels over 106 subs, falling as sigma once floored),
/// and it arrived at whatever depth the camera's gain happened to put sigma below
/// it. `depth_grain_gain` does that deliberately instead.
///
/// `1e-5` — 0.65 ADU — rather than smaller: `solve_stretch_factor_newton` treats a gap
/// of `1e-6` or less as degenerate and returns an identity stretch, so the floor has to
/// stay clear of it. Real gaps are far above either: ~2.9e-4 on a single IMX533 sub and
/// ~1.6e-4 at 106 frames with the depth gain applied.
const MIN_EFFECTIVE_MEDIAN: f32 = 1e-5;

/// Stack depth past which the sky stops getting calmer.
///
/// The split below is only affordable while the stack's noise really is falling as
/// `sqrt(N)`. It is not, deep into a real session — rejection, drift and a sky that
/// changes all take from it. On the 106-sub IMX533 set sigma falls as `N^0.41` over
/// the first 32 subs and as `N^0.19` from there to 106, and once that exponent drops
/// below the 0.25 this spends, the target starts paying for the sky: rendered target
/// contrast peaked at 32 subs and fell 77 -> 69 output levels by 106. Stopping at 64
/// keeps the whole sweep inside what the stack delivers (`N^0.37` over 1-64) and hands
/// everything past it to the target. Guarded by `stack_depth_grain_tests`.
const MAX_GAIN_DEPTH: f32 = 64.0;

/// Ceiling on the black point factor the solve may actually use.
///
/// `with_black_point_sigma` clamps to 5.0, and the depth gain then multiplies it, so
/// the product is what has to be bounded — not the setting. 5.0 x `MAX_GAIN_DEPTH^0.25`,
/// stated once rather than left to be discovered from two constants.
const MAX_EFFECTIVE_SIGMA: f32 = 5.0 * 2.828_427;

/// How much wider than `black_point_sigma` the black point sits, for a stack of
/// `frames`.
///
/// Stacking `N` frames buys `sqrt(N)` in signal-to-noise. Under a scale-invariant
/// tone curve all of it goes to faint-signal contrast and none to the sky: the MTF
/// solve pins `mtf(k * sigma) = target_background`, so displayed sky grain is
/// `T(1-T)/k` whatever sigma is, and the sky looks exactly as grainy at 100 subs as
/// at one (measured: 4.2 output levels at 1 sub, 4.4 at 8).
///
/// This splits the gain evenly instead — `k` grows as `N^(1/4)`, so displayed grain
/// falls as `N^(-1/4)` and faint-signal contrast rises as `N^(1/4)` for as long as the
/// stack's own noise falls as `sqrt(N)`; see `MAX_GAIN_DEPTH` for where it stops. Their
/// ratio is `sqrt(N)` either way; only the split is a choice. A wider black point
/// clips nothing: it sits *further below* the sky, so the faintest signal is dimmer
/// but still above black.
pub fn depth_grain_gain(frames: u32) -> f32 {
    (frames.max(1) as f32).min(MAX_GAIN_DEPTH).powf(0.25)
}

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

    let adaptive_sigma = (if signal_fraction > 0.4 {
        (config.black_point_sigma * 0.6).max(1.5)
    } else if signal_fraction > 0.2 {
        config.black_point_sigma * 0.8
    } else {
        config.black_point_sigma
    } * depth_grain_gain(config.stack_depth))
    .min(MAX_EFFECTIVE_SIGMA);

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
        stack_depth = config.stack_depth,
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
        adaptive_sigma,
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
    fn depth_grain_gain_grows_as_the_fourth_root_and_stops() {
        let ceiling = MAX_GAIN_DEPTH.powf(0.25);
        assert!((depth_grain_gain(1) - 1.0).abs() < 1e-6);
        assert!((depth_grain_gain(0) - 1.0).abs() < 1e-6, "an unknown depth is one frame");
        assert!((depth_grain_gain(16) - 2.0).abs() < 1e-5);
        assert!((depth_grain_gain(64) - ceiling).abs() < 1e-5);
        assert!(
            (depth_grain_gain(10_000) - ceiling).abs() < 1e-5,
            "the gain must stop so a very deep stack keeps growing its target"
        );
    }

    /// The setting is clamped, the gain multiplies it, and the *product* is what places
    /// the black point — so that is what has to be bounded. Left unbounded the two
    /// clamps have to be read together to know the real ceiling, which is how a factor
    /// documented as 1.5-3.0 quietly became 20.
    #[test]
    fn the_black_point_factor_the_solve_uses_has_a_stated_ceiling() {
        let frame = noisy_sky(128, 0.05, 0.002);
        let stats = compute_image_stats(&frame).unwrap();
        let config = AutoStretchConfig::new()
            .with_tone_mapping(ToneMappingAlgorithm::Mtf)
            .with_black_point_sigma(99.0)
            .with_stack_depth(100_000);

        let result =
            compute_auto_stretch_with_algorithm(&frame, &stats, config, ToneMappingAlgorithm::Mtf);
        assert!(
            result.adaptive_sigma <= MAX_EFFECTIVE_SIGMA + 1e-4,
            "the solve used {} sigmas, past the stated ceiling of {MAX_EFFECTIVE_SIGMA}",
            result.adaptive_sigma
        );
        assert!(
            (result.adaptive_sigma - 5.0 * depth_grain_gain(u32::MAX)).abs() < 1e-3,
            "the ceiling must be exactly the two clamps multiplied, not a third number"
        );
    }

    /// A per-channel black point is subtracted from the frame the curve is solved for,
    /// so it has to be placed at the sigma the solve used — not at the raw setting,
    /// which at depth differs by the whole gain.
    #[test]
    fn the_result_reports_the_sigma_its_black_point_was_placed_at() {
        let frame = noisy_sky(128, 0.05, 0.002);
        let stats = compute_image_stats(&frame).unwrap();
        let config = AutoStretchConfig::new().with_tone_mapping(ToneMappingAlgorithm::Mtf);

        for depth in [1u32, 16, 64] {
            let r = compute_auto_stretch_with_algorithm(
                &frame,
                &stats,
                config.with_stack_depth(depth),
                ToneMappingAlgorithm::Mtf,
            );
            let gap = r.original_median - r.black_point;
            assert!(
                (gap - r.adaptive_sigma * stats.mean_sigma()).abs() < 1e-6,
                "{depth} frames: the black point sits {gap} below the sky but the result \
                 reports {} sigmas of {}",
                r.adaptive_sigma,
                stats.mean_sigma()
            );
        }
    }

    /// The same sky at two depths: the deeper one is stretched more gently, which is
    /// what a calmer sky is made of. Nothing else about the frame changes.
    #[test]
    fn a_deeper_stack_lowers_the_black_point_and_softens_the_curve() {
        let frame = noisy_sky(128, 0.05, 0.002);
        let stats = compute_image_stats(&frame).unwrap();
        let config = AutoStretchConfig::new().with_tone_mapping(ToneMappingAlgorithm::Mtf);

        let shallow = compute_auto_stretch_with_algorithm(
            &frame,
            &stats,
            config.with_stack_depth(1),
            ToneMappingAlgorithm::Mtf,
        );
        let deep = compute_auto_stretch_with_algorithm(
            &frame,
            &stats,
            config.with_stack_depth(16),
            ToneMappingAlgorithm::Mtf,
        );

        let shallow_gap = shallow.original_median - shallow.black_point;
        let deep_gap = deep.original_median - deep.black_point;
        let ratio = deep_gap / shallow_gap;
        assert!(
            (ratio - 2.0).abs() < 0.05,
            "16 frames should double the sky-to-black gap, got {ratio:.3}x"
        );
        assert!(
            deep.midtones[0] > shallow.midtones[0],
            "the deeper stack should take the gentler curve: midtone {} against {}",
            deep.midtones[0],
            shallow.midtones[0]
        );
    }

    /// A sky with per-pixel noise, which the solver needs to have any statistics at all.
    fn noisy_sky(size: usize, level: f32, sigma: f32) -> Frame {
        let mut seed: u32 = 12345;
        let mut frame = Frame::zeros(size, size, 3).unwrap();
        for y in 0..size {
            for x in 0..size {
                for c in 0..3 {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * sigma * 3.46;
                    frame.set_pixel(x, y, c, level + noise);
                }
            }
        }
        frame
    }

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
