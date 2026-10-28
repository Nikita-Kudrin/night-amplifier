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
pub use saturation::{apply_shadow_saturation_boost, SaturationBoostConfig, SaturationPlugin};

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

/// Mantissa bits of a luminance that pick its table entry: 512 entries an octave, 0.2 % of
/// the value apart at every level (256 left asinh with contrast 0.16 of a level off at its
/// bright end). The rest of the mantissa is the interpolation's fraction. A half-float
/// table's trick: entries are evenly spaced within an octave, so linear interpolation
/// holds, and consecutive across octaves, so the table is seamless.
const SCALE_LUT_MANTISSA_BITS: u32 = 9;

/// The darkest luminance with an entry of its own, 2^-24 (0.004 ADU of a 14-bit sensor);
/// darker reads it. There every curve's ratio to its input is its limit at zero within
/// 0.06 %, even MTF at the solver's clamp floor (`m ≈ 1e-4`).
const SCALE_LUT_FLOOR_BITS: u32 = (127 - 24) << 23;

/// Entries in the fused stretch+contrast scale LUT: 24 octaves of 512, and 1.0 (48 KB).
///
/// Spread evenly over [0, 1], 8192 entries stepped 2 ADU of a 14-bit sensor, and once the
/// black point's gap fell under a step the sky was interpolated from the curve's limit at
/// zero and sank with depth (Cat's Eye 10.9 -> 6.2 levels by 27 subs); 16x finer only put
/// it off (the 110-sub Cat's Eye still sank). Indexed by the float's top bits the step is
/// relative, so a deeper stack never runs out of resolution.
pub const SCALE_LUT_SIZE: usize = ((0x3F80_0000 - SCALE_LUT_FLOOR_BITS) >> (23 - SCALE_LUT_MANTISSA_BITS)) as usize + 1;

/// The luminance scale-table entry `i` stands for: the curve's scale there is `lut[i]`.
#[inline]
pub fn scale_lut_luminance(i: usize) -> f32 {
    f32::from_bits(SCALE_LUT_FLOOR_BITS + ((i as u32) << (23 - SCALE_LUT_MANTISSA_BITS)))
}

/// A scale table holding `scale(L)` at every entry's luminance.
pub fn scale_lut_from(scale: impl Fn(f32) -> f32) -> Vec<f32> {
    (0..SCALE_LUT_SIZE).map(|i| scale(scale_lut_luminance(i))).collect()
}

/// Where luminance `lum` falls in the scale table: the entry below it (at most
/// `SCALE_LUT_SIZE - 2`) and the fraction of the way to the next. Under 2^-24 (negatives,
/// -0.0 and denormals too) is the bottom; above 1.0 and NaN of either sign the top, as
/// x86's default NaN carries the sign bit.
#[inline(always)]
pub fn scale_lut_position(lum: f32) -> (usize, f32) {
    const SHIFT: u32 = 23 - SCALE_LUT_MANTISSA_BITS;
    #[allow(clippy::manual_clamp)] // `clamp` keeps NaN; `min` sends it to the top entry
    let bits = (lum.min(1.0).max(0.0).to_bits() & 0x7FFF_FFFF).max(SCALE_LUT_FLOOR_BITS) - SCALE_LUT_FLOOR_BITS;
    let i = ((bits >> SHIFT) as usize).min(SCALE_LUT_SIZE - 2);
    (i, (bits - ((i as u32) << SHIFT)) as f32 * (1.0 / (1u32 << SHIFT) as f32))
}

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
    strength: u32,
    contrast: Option<(u32, u32)>,
    floor_depth: u32,
}

/// `v` with its low mantissa bits cleared, so quantisation is **relative** to the
/// value — needed since the solver's frame-to-frame jitter is a fraction of the
/// value, not an absolute amount. Rounding `v * 10_000` (`1e-4` step) once let two
/// stretch profiles 3% apart (0.001053 vs 0.001085, 106-sub IMX533 stack) collide:
/// whichever rendered second silently reused the first's curve (core 141->144, sky
/// 18->19). Keeping 13 of f32's 23 mantissa bits gives a `2^-13` step (~0.012% of
/// value), inside the solver's `tolerance` of 1e-3 and ~250 steps clear of that
/// collision; masking (vs scaling) also gives zero/negative/inf/NaN distinct keys.
fn quantize_relative(v: f32) -> u32 {
    const KEPT_MANTISSA_BITS: u32 = 13;
    v.to_bits() & !((1u32 << (23 - KEPT_MANTISSA_BITS)) - 1)
}

impl LutCacheKey {
    fn new(
        algorithm: ToneMappingAlgorithm,
        strength: f32,
        contrast: Option<&crate::render::output::ContrastConfig>,
        floor: ShadowFloor,
    ) -> Self {
        // The floor belongs in the key where the black point deliberately does
        // not: it changes the table's contents rather than being subtracted
        // per pixel. It is cache-friendly for the same reason the stretch factor
        // is — it is anchored to `target_background`, which the solver holds
        // steady, not to a per-frame statistic.
        Self {
            algorithm,
            strength: quantize_relative(strength),
            // `Option`, not a `(0, 0)` stand-in: contrast off and a contrast whose
            // parameters quantise to zero are different tables.
            contrast: contrast
                .map(|c| (quantize_relative(c.strength), quantize_relative(c.midpoint))),
            floor_depth: quantize_relative(floor.depth),
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

    scale_lut_from(|l| curve(l) / l)
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

/// Fused stretch and contrast: builds (or reuses) a LUT combining the tone mapping and
/// contrast curves, then applies black point subtraction, tone mapping and contrast in a
/// single pass — replacing three separate full-frame passes
/// (`subtract_black_point_uniform`, `mtf_stretch_frame`/`asinh_stretch_frame`,
/// `apply_contrast_frame`).
///
/// All three were rayon-parallel, so this one must be too, or the fusion is a net loss: a
/// serial fused pass measures ~3x slower and makes `auto_stretch_frame` 67% slower overall.
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

    /// Limit of `curve(L) / L` as `L → 0`, derived analytically: what entry 0, read at
    /// 2^-24, must hold. Both tone curves pass linearly through the origin.
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

    /// Entry 0 stands for 2^-24 and everything darker, so it must hold the curve's limit at
    /// zero, the darkest sky's scale, for every curve and floor the solver can ask for: to
    /// 0.06 % for the tone curves, 1.4 % with contrast at MTF's clamp floor (the S-curve's
    /// ratio converges slowest), a thousandth of a level at 2^-24 either way.
    #[test]
    fn the_first_entry_holds_the_curve_s_limit_at_zero() {
        let contrast = crate::render::output::ContrastConfig::default();
        for (algorithm, strength) in [(ToneMappingAlgorithm::Mtf, 1e-4f32), (ToneMappingAlgorithm::Mtf, 0.2), (ToneMappingAlgorithm::Asinh, 1.0), (ToneMappingAlgorithm::Asinh, 1e4)] {
            for contrast in [None, Some(&contrast)] {
                for floor in [ShadowFloor::NONE, ShadowFloor { depth: 0.02 }] {
                    let (entry, limit) = (build_scale_lut(algorithm, strength, contrast, floor)[0], scale_limit_at_zero(algorithm, strength, contrast, floor));
                    let tolerance = if contrast.is_some() { 0.015 } else { 6e-4 };
                    assert!((entry - limit).abs() <= tolerance * limit, "{algorithm:?} {strength}: entry 0 is {entry}, the limit {limit}");
                    assert!((entry - limit).abs() * scale_lut_luminance(0) * 255.0 < 1e-3);
                }
            }
        }
    }

    /// Each entry is read back exactly at its own luminance, consecutive entries stand for
    /// rising luminances evenly spaced within an octave and doubling their step across one,
    /// and what they render never falls as the luminance rises.
    #[test]
    fn the_table_is_read_exactly_at_its_entries_and_seamlessly_across_octaves() {
        let lut = build_scale_lut(ToneMappingAlgorithm::Mtf, 0.01, None, ShadowFloor::NONE);
        for i in 0..SCALE_LUT_SIZE {
            let l = scale_lut_luminance(i);
            assert_eq!(crate::render::simd::scale_lut_lookup(&lut, l), lut[i], "entry {i} at {l:e}");
            let (at, frac) = scale_lut_position(l);
            assert_eq!((at.min(i), frac), (i.min(SCALE_LUT_SIZE - 2), if i == SCALE_LUT_SIZE - 1 { 1.0 } else { 0.0 }));
        }
        assert_eq!((scale_lut_luminance(0), scale_lut_luminance(SCALE_LUT_SIZE - 1)), (2f32.powi(-24), 1.0));
        let step = |i: usize| scale_lut_luminance(i + 1) - scale_lut_luminance(i);
        for i in 0..SCALE_LUT_SIZE - 2 {
            let ratio = step(i + 1) / step(i);
            assert!(ratio == 1.0 || ratio == 2.0, "entries {i}..{}: the step changed {ratio}x", i + 2);
            assert!(scale_lut_luminance(i + 1) * lut[i + 1] > scale_lut_luminance(i) * lut[i], "the render fell after entry {i}");
        }
    }

    /// Zero, -0.0, denormals and anything under 2^-24 read the first entry; above 1.0,
    /// infinity and NaN the last; nothing reads outside the table.
    #[test]
    fn out_of_range_luminance_lands_at_the_table_s_ends() {
        let (bottom, top) = ((0, 0.0), (SCALE_LUT_SIZE - 2, 1.0));
        for lum in [0.0f32, -0.0, f32::from_bits(1), f32::MIN_POSITIVE, 2f32.powi(-25), -0.5, f32::NEG_INFINITY] {
            assert_eq!(scale_lut_position(lum), bottom, "{lum:e}");
        }
        for lum in [1.0f32, 1.5, 1e30, f32::INFINITY, f32::NAN, -f32::NAN] {
            assert_eq!(scale_lut_position(lum), top, "{lum:e}");
        }
        let (i, frac) = scale_lut_position(f32::from_bits(0x3F7F_FFFF));
        assert!(i == SCALE_LUT_SIZE - 2 && frac < 1.0 && frac > 0.99, "just under 1.0: {i} {frac}");
    }

    /// The curve the table stands for, in f64: what the kernel would render with no table.
    fn exact(algorithm: ToneMappingAlgorithm, strength: f32, contrast: Option<&crate::render::output::ContrastConfig>, floor: ShadowFloor, l: f64) -> f64 {
        let s = f64::from(strength);
        let toned = match algorithm {
            ToneMappingAlgorithm::Asinh => (s * l).asinh() / s.asinh(),
            ToneMappingAlgorithm::Mtf if l >= 1.0 => 1.0,
            ToneMappingAlgorithm::Mtf => ((s - 1.0) * l) / ((2.0 * s - 1.0) * l - s),
        };
        let contrasted = contrast.map_or(toned, |c| {
            let (x, st, mid) = (toned.clamp(0.0, 1.0), f64::from(c.strength), f64::from(c.midpoint));
            (x + st * (x - mid) * 4.0 * x * (1.0 - x)).clamp(0.0, 1.0)
        });
        let depth = f64::from(floor.depth);
        if floor.is_none() { contrasted } else { ((contrasted - depth) / (1.0 - depth)).max(0.0) }
    }

    /// The table renders every luminance a frame can hold within 0.15 of an 8-bit level of
    /// the curve it stands for, from 1e-9 to 1, at every tone curve the solver can return
    /// (MTF down to its 1e-4 clamp, asinh to its 10000 cap), contrast on and off, floors
    /// on and off. An evenly spread table fails it under its first step (2 ADU of a 14-bit
    /// sensor): there the sky is interpolated from the curve's limit at zero, and on Cat's
    /// Eye and M13 it sank with depth as the black point closed in on it.
    #[test]
    fn the_scale_table_renders_every_luminance_within_a_fraction_of_a_level() {
        let contrast = crate::render::output::ContrastConfig::default();
        let curves = [(ToneMappingAlgorithm::Mtf, [1e-4f32, 1e-3, 1e-2, 0.05, 0.25]), (ToneMappingAlgorithm::Asinh, [1.0, 10.0, 100.0, 1000.0, 10000.0])];
        let configs = curves
            .into_iter()
            .flat_map(|(algorithm, strengths)| strengths.map(|s| (algorithm, s)))
            .flat_map(|(algorithm, s)| [None, Some(&contrast)].map(|c| (algorithm, s, c)))
            .flat_map(|(algorithm, s, c)| [ShadowFloor::NONE, ShadowFloor { depth: 0.02 }, ShadowFloor { depth: 0.06 }].map(|f| (algorithm, s, c, f)));
        let mut worst = (0.0f64, String::new());
        for (algorithm, strength, contrast, floor) in configs {
            let lut = build_scale_lut(algorithm, strength, contrast, floor);
            for k in 0..=9000 {
                let lum = 10f64.powf(-9.0 + k as f64 / 1000.0) as f32;
                let table = (lum * crate::render::simd::scale_lut_lookup(&lut, lum)).clamp(0.0, 1.0);
                let error = (f64::from(table) - exact(algorithm, strength, contrast, floor, f64::from(lum))).abs() * 255.0;
                if error > worst.0 {
                    worst = (error, format!("{algorithm:?} {strength} contrast {} floor {} at L = {lum:e}", contrast.is_some(), floor.depth));
                }
            }
        }
        assert!(worst.0 <= 0.15, "the table is {:.3} levels off the curve: {}", worst.0, worst.1);
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

    /// Two curves an observer can tell apart must not share a cached table.
    ///
    /// The regression this guards: the key quantised `strength` to an absolute `1e-4`,
    /// and an MTF midtone on a deep-sky stack is ~`1e-3`. The Nebulae (since removed) and
    /// Deep Sky profiles solved to 0.001053 and 0.001085 on a 106-sub IMX533 stack — 3 % apart,
    /// the same key — so the second render of a frame used the first profile's tone
    /// curve, and which one you got depended on render order alone.
    #[test]
    fn the_lut_cache_separates_curves_it_can_tell_apart() {
        let key = |strength: f32| {
            LutCacheKey::new(ToneMappingAlgorithm::Mtf, strength, None, ShadowFloor::NONE)
        };

        assert_ne!(
            key(0.001_052_682),
            key(0.001_085_046_8),
            "two stretch profiles' curves 3 % apart share a cached table"
        );

        // Across four decades, since the depth gain pushes the solved midtone down as a
        // stack deepens and an absolute step is coarsest exactly where it ends up.
        for decade in [1e-4f32, 1e-3, 1e-2, 1e-1] {
            assert_ne!(
                key(decade),
                key(decade * 1.01),
                "a 1 % change at {decade} does not reach the cache"
            );
        }

        // ...and the jitter it exists to absorb still hits, at every magnitude.
        for decade in [1e-4f32, 1e-3, 1e-2, 1e-1, 1.0] {
            assert_eq!(
                key(decade),
                key(decade * (1.0 + 1e-5)),
                "a 0.001 % wobble at {decade} rebuilt the table"
            );
        }

        // Contrast off is not contrast at zero: different tables.
        let off = LutCacheKey::new(ToneMappingAlgorithm::Mtf, 0.1, None, ShadowFloor::NONE);
        let zero = crate::render::output::ContrastConfig { strength: 0.0, midpoint: 0.0 };
        assert_ne!(
            off,
            LutCacheKey::new(ToneMappingAlgorithm::Mtf, 0.1, Some(&zero), ShadowFloor::NONE)
        );
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
