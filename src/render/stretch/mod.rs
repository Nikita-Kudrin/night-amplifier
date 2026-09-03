//! Non-linear stretch and tone mapping functions for image enhancement
//!
//! This module provides the core stretch/tone mapping functions used in astronomical
//! imaging to boost faint details while preserving bright stars.

pub mod asinh;
pub mod mtf;
pub mod saturation;

// Re-export public items to maintain API compatibility
pub use asinh::{asinh, asinh_stretch, asinh_stretch_color_preserving, asinh_stretch_frame};
pub use mtf::{mtf, mtf_stretch_color_preserving, mtf_stretch_frame, solve_mtf_midtone};
pub use saturation::{
    apply_shadow_saturation_boost, SaturationBoostConfig, SaturationPlugin, SATURATION_PLUGIN,
};

use crate::error::Result;
use crate::render::output::ShadowFloor;
use crate::frame::Frame;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::sync::Arc;

/// Tone mapping algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToneMappingAlgorithm {
    /// Asinh (inverse hyperbolic sine) stretch - default for astrophotography
    #[default]
    Asinh,
    /// Midtones Transfer Function (Histogram Transformation)
    Mtf,
}

/// Apply tone mapping to a frame using the specified algorithm. `strength` is
/// algorithm-specific: stretch factor for Asinh (typical 1.0-20.0), midtone
/// parameter for MTF (typical 0.1-0.4). `color_intensity` (how far channels may
/// diverge from luminance scaling) is **Asinh only** — MTF applies its curve per
/// channel by design and ignores it entirely; see [`mtf_stretch_frame`], which no
/// longer even accepts the parameter.
pub fn apply_tone_mapping(
    frame: &mut Frame,
    algorithm: ToneMappingAlgorithm,
    strength: f32,
    color_intensity: f32,
) -> Result<()> {
    match algorithm {
        ToneMappingAlgorithm::Asinh => asinh_stretch_frame(frame, strength, color_intensity),
        ToneMappingAlgorithm::Mtf => mtf_stretch_frame(frame, [strength, strength, strength]),
    }
}

/// Entries in the fused stretch+contrast scale LUT.
///
/// 8192 entries (32 KB) with linear interpolation, per the live-view performance plan:
/// small enough to stay cache-resident, and interpolation keeps the worst-case error
/// below 0.15 LSB of 8-bit output even at the most aggressive midtones.
const SCALE_LUT_SIZE: usize = 8192;

/// Identifies a cached scale LUT.
///
/// Deliberately does **not** include the black point: the black point is subtracted
/// per-pixel inside the kernel and has no effect on the table's contents. Keying on it
/// would invalidate the cache every time the solver's black point drifted, which is
/// every frame in live view — exactly the per-frame LUT rebuild this cache exists to
/// avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LutCacheKey {
    algorithm: ToneMappingAlgorithm,
    strength: i32,
    contrast_strength: i32,
    contrast_midpoint: i32,
    floor_depth: i32,
    floor_knee: i32,
}

impl LutCacheKey {
    fn new(
        algorithm: ToneMappingAlgorithm,
        strength: f32,
        contrast: Option<&crate::render::output::ContrastConfig>,
        floor: ShadowFloor,
    ) -> Self {
        // Quantize parameters to avoid recalculating on tiny float jitter
        // 10000.0 gives 4 decimals of precision, which is plenty for these params
        let quantize = |v: f32| (v * 10000.0).round() as i32;
        let (cs, cm) = match contrast {
            Some(c) => (quantize(c.strength), quantize(c.midpoint)),
            None => (0, 0),
        };
        // The floor belongs in the key where the black point deliberately does
        // not: it changes the table's contents rather than being subtracted
        // per pixel. It is cache-friendly for the same reason the stretch factor
        // is — it is anchored to `target_background`, which the solver holds
        // steady, not to a per-frame statistic.
        Self {
            algorithm,
            strength: quantize(strength),
            contrast_strength: cs,
            contrast_midpoint: cm,
            floor_depth: quantize(floor.depth),
            floor_knee: quantize(floor.knee),
        }
    }
}

struct LutCache {
    key: Option<LutCacheKey>,
    lut: Arc<Vec<f32>>,
}

thread_local! {
    /// Per-thread rather than global: the live-view render task and the session-teardown
    /// stacked-PNG render run concurrently on different threads with different stretch
    /// parameters, and a single shared slot would let them evict each other every frame.
    /// Thread-local also removes the mutex from the hot path and the poisoning hazard.
    static SCALE_LUT_CACHE: RefCell<LutCache> = RefCell::new(LutCache {
        key: None,
        lut: Arc::new(Vec::new()),
    });
}

#[cfg(test)]
thread_local! {
    /// Counts LUT rebuilds so tests can assert the cache actually holds across frames.
    /// Thread-local to match the cache it observes, so tests running in parallel on other
    /// threads cannot perturb the count.
    static LUT_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Limit of `curve(L) / L` as `L → 0`, used to seed `scale_lut[0]`. Both tone curves
/// pass linearly through the origin, so the ratio converges — but derived
/// analytically, not by evaluating near zero: `asinh(x) = ln(x + sqrt(x² + 1))`, and
/// in f32 the `1.0 +` swallows most of the significand for tiny `x`, giving a limit
/// several percent wrong.
///
/// Only fixes the *value* at entry 0 — when the solver bottoms out at its clamp
/// floor (`m ≈ 1e-4`, near-black frame) the first bin spans over half the output
/// range and no table this size can represent it (the 65536-entry table this
/// replaced had the same limitation).
fn scale_limit_at_zero(
    algorithm: ToneMappingAlgorithm,
    strength: f32,
    contrast: Option<&crate::render::output::ContrastConfig>,
    floor: ShadowFloor,
) -> f32 {
    // `asinh_stretch` returns its input unchanged for a non-positive strength, so the limit
    // is 1.0. MTF has no meaningful limit there, but `solve_mtf_midtone` clamps the midtone
    // into (0, 1), so for MTF this branch is only a defensive fallback.
    let tone_limit = if strength <= 0.0 {
        1.0
    } else {
        match algorithm {
            // asinh(sL) / asinh(s) / L  →  s / asinh(s)
            ToneMappingAlgorithm::Asinh => strength / asinh::asinh(strength),
            // ((m-1)L) / ((2m-1)L - m) / L  →  (1-m)/m
            ToneMappingAlgorithm::Mtf => (1.0 - strength) / strength,
        }
    };

    // apply_s_curve(y) = y + strength·(y - mid)·4y(1 - y), clamped to [0, 1],
    // so s_curve(y) / y → 1 - 4·strength·mid. The clamp makes a negative slope mean the
    // curve genuinely floors at zero there, so saturating at 0 is faithful.
    let contrast_slope = match contrast {
        Some(config) if !config.is_disabled() => {
            (1.0 - 4.0 * config.strength * config.midpoint).max(0.0)
        }
        _ => 1.0,
    };

    // The floor's own limit, which for the clipping form is genuinely zero —
    // the one case where a zero at entry 0 is faithful rather than a bug, since
    // "everything under the floor is black" is exactly what was asked for.
    tone_limit * contrast_slope * floor.slope_at_zero()
}

fn build_scale_lut(
    algorithm: ToneMappingAlgorithm,
    strength: f32,
    contrast: Option<&crate::render::output::ContrastConfig>,
    floor: ShadowFloor,
) -> Vec<f32> {
    #[cfg(test)]
    LUT_BUILDS.with(|c| c.set(c.get() + 1));

    // The floor goes last, after contrast, because that is where it goes in the
    // unfused path too — the encoder's row tail applies it on the far side of
    // `apply_contrast_slice`. Matching the two is what keeps one slider position
    // meaning one thing whether or not saturation boost pushed contrast out of
    // this table.
    let curve = |l: f32| {
        let stretched = match algorithm {
            ToneMappingAlgorithm::Asinh => asinh::asinh_stretch(l, strength),
            ToneMappingAlgorithm::Mtf => mtf::mtf(l, strength),
        };
        let contrasted = match contrast {
            Some(config) => crate::render::output::apply_s_curve(stretched, config),
            None => stretched,
        };
        floor.apply(contrasted)
    };

    let mut lut = vec![0.0f32; SCALE_LUT_SIZE];
    // Entry 0 is the limit, not zero: a hard 0 here forces every pixel below one bin
    // width to pure black, discarding up to ~28 LSB of the faintest signal.
    lut[0] = scale_limit_at_zero(algorithm, strength, contrast, floor);
    for (i, entry) in lut.iter_mut().enumerate().skip(1) {
        let l_in = i as f32 / (SCALE_LUT_SIZE - 1) as f32;
        *entry = curve(l_in) / l_in;
    }
    lut
}

pub fn cached_scale_lut(
    algorithm: ToneMappingAlgorithm,
    strength: f32,
    contrast: Option<&crate::render::output::ContrastConfig>,
    floor: ShadowFloor,
) -> Arc<Vec<f32>> {
    let key = LutCacheKey::new(algorithm, strength, contrast, floor);
    SCALE_LUT_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.key != Some(key) {
            cache.lut = Arc::new(build_scale_lut(algorithm, strength, contrast, floor));
            cache.key = Some(key);
        }
        Arc::clone(&cache.lut)
    })
}

/// Fused stretch and contrast function
///
/// Builds (or reuses) a LUT combining the tone mapping and contrast curves, then applies
/// black point subtraction, tone mapping and contrast to the frame in a single pass.
///
/// Replaces three separate full-frame passes (`subtract_black_point_uniform`,
/// `mtf_stretch_frame`/`asinh_stretch_frame`, `apply_contrast_frame`). All three were
/// rayon-parallel, so this one must be too or the fusion is a net loss — a serial fused
/// pass measures ~3x slower than the three parallel passes it replaces, and makes
/// `benches/render_benchmark.rs` `auto_stretch_frame` 67 % slower end to end.
pub fn apply_fused_stretch_frame(
    frame: &mut Frame,
    black_point: f32,
    algorithm: ToneMappingAlgorithm,
    strength: f32,
    color_intensity: f32,
    contrast: Option<&crate::render::output::ContrastConfig>,
    floor: ShadowFloor,
) -> Result<()> {
    if frame.channels() != 3 {
        return Err(crate::error::StackError::InvalidConfiguration(
            "Fused stretch requires 3 channels".into(),
        ));
    }

    let scale_lut = cached_scale_lut(algorithm, strength, contrast, floor);
    apply_scale_lut_frame(frame, black_point, &scale_lut, color_intensity)
}

/// Applies an already-built scale LUT to a planar frame, in place. Split out of
/// [`apply_fused_stretch_frame`]'s tail so callers already holding a
/// `StretchResult::scale_lut` don't re-derive this loop — one that did drove the
/// *interleaved* kernel over `frame.data_mut().par_chunks_mut(width * 3)`, so every
/// "pixel" it saw was three horizontally-adjacent red-plane samples.
///
/// Rows, not whole planes: `with_min_len(32)` lets rayon coalesce them, and
/// per-plane dispatch would need three passes to keep a pixel's channels together.
pub fn apply_scale_lut_frame(
    frame: &mut Frame,
    black_point: f32,
    scale_lut: &[f32],
    color_intensity: f32,
) -> Result<()> {
    if frame.channels() != 3 {
        return Err(crate::error::StackError::InvalidConfiguration(
            "Fused stretch requires 3 channels".into(),
        ));
    }

    let width = frame.width();
    let (r, g, b) = frame.planes_mut();

    r.par_chunks_mut(width)
        .zip_eq(g.par_chunks_mut(width))
        .zip_eq(b.par_chunks_mut(width))
        .with_min_len(32)
        .for_each(|((r_row, g_row), b_row)| {
            crate::render::simd::apply_luminance_scale_lut_simd_planar(
                r_row,
                g_row,
                b_row,
                black_point,
                scale_lut,
                color_intensity,
            );
        });

    Ok(())
}

/// Estimate the strength parameter to achieve target background brightness
///
/// # Arguments
/// * `algorithm` - Which tone mapping algorithm
/// * `input_median` - Current median brightness of the image
/// * `target_output` - Desired output brightness (typically 0.15-0.25)
///
/// # Returns
/// Recommended strength parameter for the algorithm
pub fn estimate_tone_mapping_strength(
    algorithm: ToneMappingAlgorithm,
    input_median: f32,
    target_output: f32,
) -> f32 {
    match algorithm {
        ToneMappingAlgorithm::Asinh => asinh::estimate_stretch_factor(input_median, target_output),
        ToneMappingAlgorithm::Mtf => mtf::solve_mtf_midtone(input_median, target_output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_tone_mapping_asinh() {
        let mut frame = Frame::filled(10, 10, 3, 0.2).unwrap();
        apply_tone_mapping(&mut frame, ToneMappingAlgorithm::Asinh, 5.0, 1.0).unwrap();

        // Values should be boosted
        assert!(frame.get_pixel(5, 5, 0) > 0.2);
    }

    #[test]
    fn test_estimate_tone_mapping_strength() {
        let input = 0.1;
        let target = 0.2;

        let asinh_strength =
            estimate_tone_mapping_strength(ToneMappingAlgorithm::Asinh, input, target);

        // Both should produce reasonable positive values
        assert!(asinh_strength > 0.0);
    }

    #[test]
    fn test_tone_mapping_algorithm_default() {
        let algo = ToneMappingAlgorithm::default();
        assert_eq!(algo, ToneMappingAlgorithm::Asinh);
    }

    fn noisy_astro_frame(width: usize, height: usize) -> Frame {
        // Dark sky background with sparse stars — the shape the tone curves are solved for.
        // Stars are whole pixels, as a debayered frame delivers them, not single channels.
        let mut seed = 0x1234_5678u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        let mut data = Vec::with_capacity(width * height * 3);
        for i in 0..width * height {
            if i % 331 == 0 {
                let core = 0.3 + rand() * 0.7;
                for _ in 0..3 {
                    data.push((core * (0.85 + rand() * 0.3)).min(1.0));
                }
            } else {
                for _ in 0..3 {
                    data.push(0.004 + rand() * 6.0e-4);
                }
            }
        }
        Frame::from_f32_vec(data, width, height, 3).unwrap()
    }

    // test_fused_stretch_matches_separate_passes was removed because MTF stretch is intentionally
    // applied independently per channel now, while the fused scale LUT preserves luminance.
    // They are no longer expected to match mathematically.

    /// Same check for asinh, which had no LUT at all before the fusion and is the default
    /// tone mapping algorithm.
    #[test]
    fn test_fused_stretch_matches_separate_passes_asinh() {
        let frame = noisy_astro_frame(96, 64);
        let strength = 12.0;

        let mut fused = frame.clone();
        apply_fused_stretch_frame(
            &mut fused,
            0.0,
            ToneMappingAlgorithm::Asinh,
            strength,
            1.0,
            None,
            ShadowFloor::NONE,
        )
        .unwrap();

        let mut separate = frame.clone();
        asinh::asinh_stretch_frame(&mut separate, strength, 1.0).unwrap();

        let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 as i32;
        let max_delta = separate
            .data()
            .iter()
            .zip(fused.data().iter())
            .map(|(a, b)| (to_u8(*b) - to_u8(*a)).abs())
            .max()
            .unwrap_or(0);

        assert!(
            max_delta <= 1,
            "asinh fused path drifts by {max_delta} LSB from the exact per-pixel stretch"
        );
    }

    /// The faintest signal must survive the LUT. Before the fix, `scale_lut[0] = 0.0` sent
    /// everything below one bin width (1/8191) to pure black.
    #[test]
    fn test_fused_stretch_preserves_sub_bin_luminance() {
        let faint = 0.5 / (SCALE_LUT_SIZE - 1) as f32;
        let mut frame = Frame::filled(8, 8, 3, faint).unwrap();

        apply_fused_stretch_frame(
            &mut frame,
            0.0,
            ToneMappingAlgorithm::Mtf,
            0.01,
            1.0,
            None,
            ShadowFloor::NONE,
        )
        .unwrap();

        let out = frame.get_pixel(4, 4, 0);
        assert!(
            out > 0.0,
            "sub-bin luminance crushed to pure black (was {faint}, became {out})"
        );
        // An aggressive midtone should lift it well clear of zero, not merely leave it be.
        assert!(out > faint, "faint signal was not stretched at all: {out}");
    }

    #[test]
    fn test_fused_stretch_rejects_non_rgb() {
        let mut frame = Frame::filled(8, 8, 1, 0.2).unwrap();
        let result =
            apply_fused_stretch_frame(
                &mut frame,
                0.0,
                ToneMappingAlgorithm::Mtf,
                0.2,
                1.0,
                None,
                ShadowFloor::NONE,
            );
        assert!(result.is_err());
    }

    /// The LUT cache must survive the black point changing, because the solver's black
    /// point drifts every frame in live view while the table itself does not depend on it.
    #[test]
    fn test_lut_cache_ignores_black_point_changes() {
        let builds = || LUT_BUILDS.with(|c| c.get());

        // Warm this thread's cache so the first call below is not counted as a build.
        let mut warm = Frame::filled(4, 4, 3, 0.2).unwrap();
        apply_fused_stretch_frame(
            &mut warm,
            0.001,
            ToneMappingAlgorithm::Mtf,
            0.123,
            1.0,
            None,
            ShadowFloor::NONE,
        )
        .unwrap();

        let before = builds();
        for black_point in [0.002, 0.05, 0.4] {
            let mut frame = Frame::filled(4, 4, 3, 0.5).unwrap();
            apply_fused_stretch_frame(
                &mut frame,
                black_point,
                ToneMappingAlgorithm::Mtf,
                0.123,
                1.0,
                None,
                ShadowFloor::NONE,
            )
            .unwrap();
        }
        assert_eq!(
            builds(),
            before,
            "changing only the black point rebuilt the LUT"
        );

        // A different curve parameter must still rebuild.
        let mut frame = Frame::filled(4, 4, 3, 0.5).unwrap();
        apply_fused_stretch_frame(
            &mut frame,
            0.002,
            ToneMappingAlgorithm::Mtf,
            0.321,
            1.0,
            None,
            ShadowFloor::NONE,
        )
        .unwrap();
        assert_eq!(builds(), before + 1);
    }

    /// Entry 0 must hold the finite `L -> 0` limit of `curve(L) / L`, and it must be
    /// consistent with the rest of the table (entry 1 is the nearest sampled point).
    #[test]
    fn test_scale_lut_zero_entry_is_the_curve_limit() {
        for midtone in [0.005, 0.02, 0.1, 0.3] {
            let lut = build_scale_lut(ToneMappingAlgorithm::Mtf, midtone, None, ShadowFloor::NONE);
            let expected = (1.0 - midtone) / midtone;
            assert!(
                (lut[0] - expected).abs() / expected < 1e-4,
                "mtf m={midtone}: lut[0] = {}, expected {expected}",
                lut[0]
            );
            // Entry 0 must join continuously onto the sampled part of the table. The bug
            // this guards set it to 0.0 next to a neighbour in the hundreds.
            assert!(
                (lut[0] - lut[1]).abs() / lut[0] < 0.05,
                "entry 0 ({}) is discontinuous with entry 1 ({})",
                lut[0],
                lut[1]
            );
            // Both stretches compress highlights, so the scale falls across the table.
            // Checked over a wide span, not adjacent entries: near the origin the scale is
            // flat to well under one f32 ULP, so neighbours there are not ordered.
            assert!(
                lut[0] >= lut[SCALE_LUT_SIZE / 2]
                    && lut[SCALE_LUT_SIZE / 2] > lut[SCALE_LUT_SIZE - 1]
            );
        }

        for strength in [5.0, 20.0, 100.0] {
            let lut = build_scale_lut(ToneMappingAlgorithm::Asinh, strength, None, ShadowFloor::NONE);
            let expected = strength / asinh::asinh(strength);
            assert!(
                (lut[0] - expected).abs() / expected < 1e-4,
                "asinh s={strength}: lut[0] = {}, expected {expected}",
                lut[0]
            );
            assert!(
                (lut[0] - lut[1]).abs() / lut[0] < 0.05,
                "entry 0 ({}) is discontinuous with entry 1 ({})",
                lut[0],
                lut[1]
            );
            assert!(
                lut[0] >= lut[SCALE_LUT_SIZE / 2]
                    && lut[SCALE_LUT_SIZE / 2] > lut[SCALE_LUT_SIZE - 1]
            );
        }
    }

    /// With contrast fused in, entry 0 picks up the S-curve's slope at the origin.
    #[test]
    fn test_scale_lut_zero_entry_includes_contrast_slope() {
        use crate::render::output::ContrastConfig;

        let midtone = 0.02;
        let contrast = ContrastConfig::default();
        let lut = build_scale_lut(
            ToneMappingAlgorithm::Mtf,
            midtone,
            Some(&contrast),
            ShadowFloor::NONE,
        );

        let expected = ((1.0 - midtone) / midtone)
            * (1.0 - 4.0 * contrast.strength * contrast.midpoint).max(0.0);
        assert!(
            (lut[0] - expected).abs() / expected < 1e-3,
            "lut[0] = {}, expected ~{expected}",
            lut[0]
        );
    }
}
