//! À trous (starlet) wavelet denoising of the luminance plane: repeated separable
//! convolution with the B3 spline kernel, holes doubling each level so level `l`
//! isolates ~`2^l`-pixel structure. Detail planes are soft-thresholded against the
//! finest level's noise; the image rebuilds from what survives plus the coarsest
//! residual.
//!
//! Each level is thresholded against **its own measured noise**. The ratios between
//! levels are not derivable: they hold only for white noise, and a stacked frame's is
//! not white — registration warps it and the subs stop being independent, so on real
//! IMX533 stacks the 32-128 px detail planes carry ~40x the power a white-noise
//! propagation predicts. Deriving them from level 1 (what this did until the sigma was
//! measured per level) left the coarse thresholds far too small to reach the mottle
//! they exist for.
//!
//! Thresholds still get *weaker* with scale: denoising hardest at coarse scales erases
//! the target — on a 0.62"/px image the Dumbbell's outer lobes are level-3/4 structure,
//! and `k=3` there removes them with the noise. Level 1 is untouched (`k=0`) unless
//! `star_protection` lowers it: it carries star cores at this sampling, and clipping it
//! loses a star's peak but not its wings — a bloated blob, not a cleaner frame.

use rayon::prelude::*;

use crate::statistics::fast_median;

/// B3 spline scaling kernel, `[1, 4, 6, 4, 1] / 16`.
const B3: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];

/// Scales a MAD into a Gaussian sigma.
const MAD_TO_SIGMA: f32 = 1.4826;

/// Detail planes computed.
///
/// Levels 1-4 reach ~1-16 px. That is *not* where an observer reads grain: on a 1440p
/// stream of a 106-sub IMX533 stack the 16-32 and 32-64 px bands measure 1.65 and 1.34
/// output levels against 1.01 at 8-16, so the two largest bands sat entirely outside a
/// 4-level transform. Nothing but the tone curve could reach them, and the tone curve
/// charges target contrast 1:1 — which is what made the Background Grain dial's cheap
/// half inert in the only bands that matter (measured: 0.0 % change in 8-128 px noise
/// across its bottom half).
///
/// Levels 5-6 reach ~32 and ~64 px and are what [`COARSE_FIRST`] guards: they are only
/// safe behind the interscale mask, because thresholding a coarse coefficient where a
/// star's flux sits spreads that flux into a disc around it.
pub const MAX_LEVELS: usize = 6;

/// First level that needs the interscale mask, as an index into `k`.
///
/// Levels 1-4 (indices 0-3) threshold against their own noise and leave stars alone:
/// their support is smaller than the gap between a star and its neighbours. From level 5
/// the support is 32 px and wider, so the smoothed plane carries a star's flux well past
/// the star — the detail plane goes *negative* just outside it, and soft-thresholding
/// that pulls light out of the core into a ring. Measured without the mask, every star
/// sat in a pool +1.2 to +2.5 output levels above the field sky with a -2.3 level trough
/// at r=9-15 px. `a_bright_star_keeps_no_ring` is the guard.
const COARSE_FIRST: usize = 4;

/// How far above the sky a *smoothed* plane must sit before the coarse levels leave it
/// alone, in robust sigmas of that plane.
///
/// The mask is read off the smoothed plane itself rather than dilated out from a map of
/// star positions, and that is the whole trick. The disc a coarse level would light is
/// exactly the region where the smoothing has carried a star's flux — so the smoothed
/// plane *is* the disc, at the right size, for every star, with no radius to guess. A
/// dilated point mask was measured and fails both ways: spread narrow it protects
/// nothing (ringing unchanged), spread wide it protects a dense field entirely (coarse
/// levels recover nothing at all).
/// The rule is nearly a step at the sky itself — *smooth what is at or below the sky
/// level, never anything brighter* — and a quarter sigma only keeps the transition
/// continuous, since a step in a threshold is a visible edge in the sky. That rule is
/// enough because the disc a coarse level would light is, by construction, brighter than
/// the sky. Swept: from 0.25 to 2.0 sigmas the recovered grain moves 3 % and the disc
/// around every star grows from +0.68 to +1.12 output levels, so the tight end is free.
const MASK_SIGMAS: f32 = 0.25;

/// Samples drawn to estimate the level-1 noise. A robust sigma converges long
/// before a full-plane sort is worth 2 M elements of work per frame.
const MAX_SIGMA_SAMPLES: usize = 1 << 16;

/// Largest overall strength multiplier. The settings layer, the UI slider and
/// this clamp all have to agree, or the top of the slider does nothing — or
/// worse, does something the slider cannot express.
///
/// **1.0, i.e. the tuned thresholds are the ceiling.** It was 2.0, and everything above
/// 1.0 was not merely useless but harmful: pushing levels 2-4 harder does not remove the
/// mottle, it *moves* it outward past the transform's reach (measured 1.0 -> 2.0 on a
/// 3028-frame M27: the 8-16 px band falls 1.13 -> 0.88 while 32-64 rises 1.43 -> 1.60 and
/// total 8-128 px noise does not move at all), and it rings — the whole stellar wing
/// lifts, r=9 px going 6.4 -> 10.1 output levels with the disc around each star doubling
/// from +0.40 to +0.84. Reported from the field as "stars start to have rings".
pub const MAX_STRENGTH: f32 = 1.0;

/// À trous wavelet denoising of the luminance plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaDenoiseConfig {
    /// Per-level gain applied to what survives the threshold, finest first.
    ///
    /// `1.0` everywhere reconstructs exactly what the transform decomposed, which is what
    /// every profile but Star Fields wants. Above 1.0 it *sharpens* at that scale, below
    /// it flattens — and because the threshold has already run, the gain multiplies what
    /// is left after the noise has gone rather than the noise itself. That is the whole
    /// trick: denoise, then amplify what survived.
    pub gain: [f32; MAX_LEVELS],
    pub enabled: bool,
    /// Per-level threshold in sigmas of that level's noise, finest first.
    ///
    /// `k[0]` applies to level 1 and defaults to zero — see the module note on
    /// star cores.
    pub k: [f32; MAX_LEVELS],
    /// Overall amount, scaling every threshold. `1.0` is the tuned default;
    /// this is the control an observer moves at the eyepiece.
    pub strength: f32,
}

/// No sharpening and no flattening: reconstruct what was decomposed.
pub const UNIT_GAIN: [f32; MAX_LEVELS] = [1.0; MAX_LEVELS];

impl Default for LumaDenoiseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            k: DEFAULT_K,
            gain: UNIT_GAIN,
            strength: 1.0,
        }
    }
}

/// Hardest at the finest scale above the stars, backing off as scale grows.
///
/// `k[0]` is the level-1 threshold and is not a fixed part of the tuning — it is
/// what [`LumaDenoiseConfig::thresholds_for_star_protection`] moves. The value
/// here is full protection, i.e. level 1 untouched.
pub const DEFAULT_K: [f32; MAX_LEVELS] = [0.0, 3.0, 2.0, 1.0, 0.0, 0.0];

/// Coarse thresholds at full Background Grain, for levels 5 and 6.
///
/// Weaker than the mid levels, for the reason the module note gives: the coarser the
/// scale, the more of what is there is the target rather than its texture. Measured on
/// the observer's own sessions these take 8-128 px sky noise down 15 % for 1 % of target
/// brightness — about fifteen times the exchange rate the tone curve offers, which
/// charges 1:1.
///
/// Half of what the sweep's best grain figure wanted. Doubling them buys another 2 % of
/// grain and takes the disc around each star from +0.48 to +0.68 output levels, which is
/// the whole of `a_bright_star_keeps_no_ring`'s margin for 2 %. The dense-field session
/// is what sets this, not the sparse ones.
pub const COARSE_K: [f32; 2] = [1.0, 0.5];

/// How much harder Star Fields leans on the finest scale than the other profiles.
///
/// That mode is looking for point sources against an empty sky, and at streaming
/// resolution a star spans 2-3 px while the noise it competes with is 1 px. Pushing the
/// level-1 threshold past [`MAX_LEVEL1_K`] is a matched filter in all but name: it costs
/// the star almost nothing and takes most of what it is hiding in. Measured against the
/// shipped threshold on a 181-frame 35 mm IMX464 field, detected stars go from 7809 to
/// 15297 per megapixel above sky+20 and from 3127 to 5884 above sky+60, with fine-scale
/// noise down 45 %. On long-focal sets the same setting is roughly neutral (a 1852-frame
/// globular loses 3 % of its faintest and gains 6 % of its brightest).
///
/// The cap this deliberately exceeds exists for the profiles that have nebulosity to
/// lose. Here [`STAR_FIELD_GAIN`] puts the star's peak back and then some.
pub const STAR_FIELD_FINE_BOOST: f32 = 2.5;

/// Per-level gain for Star Fields: sharpen the scales a star lives at, touch nothing
/// else.
///
/// Because the threshold has already run, this multiplies what survived rather than the
/// noise — denoise, then amplify. It buys ~4 % of star peak and 4-6 % more stars over the
/// unsharpened ladder, and it is what turns a globular's core from a mush back into
/// countable stars.
///
/// **The coarse gains stay at 1.0, and that is the load-bearing part.** Flattening them
/// to spread a cluster's glow was tried and digs a moat around every star — -8 output
/// levels at 0.8/0.5/0.4, visible as a black ring — for the same reason a coarse
/// *threshold* does: both take away the star's own broad wings. Past 1.4 on the fine
/// scales the same moat appears from the other direction, so 1.25 is not a shy number,
/// it is most of the usable range. Guarded by `a_bright_star_keeps_no_ring`.
pub const STAR_FIELD_GAIN: [f32; MAX_LEVELS] = [1.1, 1.1, 1.05, 1.0, 1.0, 1.0];

/// Level-1 threshold at zero star protection. A B3 spline à trous transform puts
/// ~94% of a white signal's variance in level 1, the only threshold that can move
/// sky grain much: on IMX533 it takes sky sigma from 4.71 to 1.47 output levels
/// (4.6x, vs 1.44x from the rest of the tuning) while integrated flux moves 0.43%
/// and the brightest star core doesn't move at all. Caps at 1.0 because past it,
/// star cores lose their peak while keeping their wings — a bloated blob, not a
/// cleaner frame.
pub const MAX_LEVEL1_K: f32 = 1.0;

impl LumaDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        k: DEFAULT_K,
        gain: UNIT_GAIN,
        strength: 1.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled
            && self.strength > 0.0
            && (self.k.iter().any(|&k| k > 0.0) || self.gain != UNIT_GAIN)
    }

    /// Per-level thresholds for a given star protection, `0..=1`.
    ///
    /// Protection moves `k[0]` alone. Levels 2-4 are where faint nebulosity lives and
    /// their tuning is not a matter of taste; level 1 is where both the sky grain and the
    /// star cores are, which is exactly the trade only an observer at the eyepiece can
    /// settle. `1.0` leaves level 1 untouched.
    pub fn thresholds_for_star_protection(protection: f32) -> [f32; MAX_LEVELS] {
        Self::thresholds_for(protection, 0.0)
    }

    /// Per-level thresholds for a star protection and a coarse strength, both `0..=1`.
    ///
    /// The two ends of the Background Grain dial. `coarse` drives levels 5-6, which are
    /// the only ones that reach the 16-64 px band an observer actually reads grain in;
    /// see [`MAX_LEVELS`].
    pub fn thresholds_for(protection: f32, coarse: f32) -> [f32; MAX_LEVELS] {
        let mut k = DEFAULT_K;
        k[0] = (1.0 - protection.clamp(0.0, 1.0)) * MAX_LEVEL1_K;
        let coarse = coarse.clamp(0.0, 1.0);
        for (i, &full) in COARSE_K.iter().enumerate() {
            k[COARSE_FIRST + i] = full * coarse;
        }
        k
    }

    /// Thresholds actually applied, in sigmas of each level's own noise.
    ///
    /// `strength` — the "Structure strength" control — scales **only levels 2-4**, the
    /// mid scales it is named for. It used to scale all of them, and that was a trap: an
    /// observer running it at 0.2 had every position of the Background Grain dial
    /// quietly divided by five, so the dial measured a 0.0 % change in visible sky noise
    /// across its whole bottom half. Two controls, one of them silently scaling the
    /// other, is not two controls. `strength = 0` still turns the filter off outright —
    /// that is `is_enabled`'s job, not this one's.
    fn scaled_k(&self) -> [f32; MAX_LEVELS] {
        let s = self.strength.clamp(0.0, MAX_STRENGTH);
        std::array::from_fn(|i| {
            let k = self.k[i].max(0.0);
            if (1..COARSE_FIRST).contains(&i) {
                k * s
            } else {
                k
            }
        })
    }
}

/// Denoise `luma` in place. `width * height` samples, linear light.
///
/// Only the tests take this door: production reaches the transform through
/// [`super::denoise_rgb_interleaved_with`], which lends it buffers.
#[cfg(test)]
pub fn denoise_luma(luma: &mut [f32], width: usize, height: usize, config: &LumaDenoiseConfig) {
    denoise_luma_with(luma, width, height, config, &mut Default::default());
}

/// [`denoise_luma`], reusing the caller's ping-pong pair and convolution
/// intermediate instead of allocating three full-size buffers per call.
pub(super) fn denoise_luma_with(
    luma: &mut [f32],
    width: usize,
    height: usize,
    config: &LumaDenoiseConfig,
    buffers: &mut [Vec<f32>; 3],
) {
    let n = width * height;
    if n == 0 || luma.len() < n {
        return;
    }

    let k = config.scaled_k();
    let wants_coarse = k[COARSE_FIRST..].iter().any(|&k| k > 0.0);

    // `luma` becomes the reconstruction accumulator: the detail planes are added
    // into it and the coarsest residual last, so no fourth full-size buffer is
    // needed to hold the sum.
    let [coarse_buf, next_buf, scratch_buf] = buffers;
    // `coarse` and `next` are swapped each level, so these are re-bindings of
    // the slices rather than of the buffers behind them.
    let mut coarse = super::take(coarse_buf, n);
    let mut next = super::take(next_buf, n);
    let scratch = super::take(scratch_buf, n);
    coarse.copy_from_slice(&luma[..n]);
    luma[..n].fill(0.0);

    for level in 0..MAX_LEVELS {
        let hole = 1usize << level;
        // A hole wider than the image reduces to a plain copy: every tap lands
        // on the same mirrored sample, so the detail plane is zero and the
        // remaining levels have nothing left to say.
        if hole >= width.max(height) {
            break;
        }
        // No coarse level is on, so the remaining passes would only decompose what
        // they immediately reconstruct.
        if level >= COARSE_FIRST && !wants_coarse {
            break;
        }

        atrous_smooth(coarse, next, scratch, width, height, hole);

        // Measured per level, not propagated from level 1: see the module note.
        // For white noise the two agree exactly — `k` means the same thing either
        // way — so this only changes a frame whose noise is correlated, which is
        // every stack.
        let level_sigma = estimate_detail_sigma(coarse, next, n);
        let threshold = k[level] * level_sigma;
        let gain = config.gain[level];

        if level >= COARSE_FIRST {
            // The smoothed plane is the mask: see `MASK_SIGMAS`.
            let (sky, spread) = robust_level(next, n);
            accumulate_detail_garrote(
                &mut luma[..n],
                coarse,
                next,
                threshold,
                gain,
                sky,
                MASK_SIGMAS * spread,
            );
        } else {
            accumulate_detail(&mut luma[..n], coarse, next, threshold, gain);
        }

        std::mem::swap(&mut coarse, &mut next);
    }

    let chunk = crate::parallel::balanced_chunk_len(n);
    luma[..n]
        .par_chunks_mut(chunk)
        .zip(coarse.par_chunks(chunk))
        .for_each(|(out, residual)| {
            for (o, &c) in out.iter_mut().zip(residual.iter()) {
                *o += c;
            }
        });
}

/// Robust centre and spread of a plane, from the same strided subsample
/// [`estimate_detail_sigma`] uses.
fn robust_level(plane: &[f32], n: usize) -> (f32, f32) {
    let stride = (n / MAX_SIGMA_SAMPLES).max(1);
    let mut samples: Vec<f32> = (0..n).step_by(stride).map(|i| plane[i]).collect();
    if samples.len() < 32 {
        return (0.0, 0.0);
    }
    let median = fast_median(&mut samples);
    for v in samples.iter_mut() {
        *v = (*v - median).abs();
    }
    let spread = fast_median(&mut samples) * MAD_TO_SIGMA;
    (median, if spread.is_finite() && spread > 0.0 { spread } else { 0.0 })
}

/// Coarse-level shrinkage: the non-negative garrote, with the threshold taken to zero
/// wherever the mask says a star is.
///
/// **Garrote, not the soft threshold the finer levels use**, and this is the fix for the
/// ringing rather than the mask. A soft threshold subtracts `t` from *every* surviving
/// coefficient, including the large ones a star puts in the coarse planes — and at level
/// 5-6 the smoothed plane carries that star's flux tens of pixels out, so the constant
/// shift moves real light from the core into a ring around it (measured without this: a
/// -2.3 output level trough at r=9-15 px with every star in a pool +1.2 levels above the
/// field). The garrote shrinks by `t^2 / d`, so it still collapses noise near the
/// threshold but leaves `d >> t` almost untouched, which is exactly the bias that was
/// doing the damage. It is continuous at `t`, so it does not reintroduce the blotches
/// hard thresholding was rejected for.
#[allow(clippy::too_many_arguments)]
fn accumulate_detail_garrote(
    out: &mut [f32],
    coarse: &[f32],
    next: &[f32],
    threshold: f32,
    gain: f32,
    sky: f32,
    scale: f32,
) {
    let chunk = crate::parallel::balanced_chunk_len(out.len());
    out.par_chunks_mut(chunk)
        .zip(coarse.par_chunks(chunk))
        .zip(next.par_chunks(chunk))
        .for_each(|((out, coarse), next)| {
            for (o, (&c, &s)) in out.iter_mut().zip(coarse.iter().zip(next.iter())) {
                let d = c - s;
                // How far this sample's *smoothed* neighbourhood stands above the sky,
                // which is what says whether the coarse detail here is a star's skirt or
                // the sky's own texture.
                let lit = if scale > 0.0 {
                    ((s - sky) / scale).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let t = threshold * (1.0 - lit);
                if t <= 0.0 {
                    *o += gain * d;
                    continue;
                }
                let magnitude = d.abs();
                *o += gain
                    * if magnitude <= t {
                        0.0
                    } else {
                        d * (1.0 - (t * t) / (magnitude * magnitude))
                    };
            }
        });
}

/// `out += soft_threshold(coarse - next, threshold)`.
///
/// Soft rather than hard thresholding: hard leaves a discontinuity at the
/// threshold, which on a low-slope sky turns into visible blotches exactly
/// where the filter was supposed to smooth. The coarse levels use
/// [`accumulate_detail_garrote`] instead, for a reason its own note gives.
fn accumulate_detail(out: &mut [f32], coarse: &[f32], next: &[f32], threshold: f32, gain: f32) {
    let chunk = crate::parallel::balanced_chunk_len(out.len());
    out.par_chunks_mut(chunk)
        .zip(coarse.par_chunks(chunk))
        .zip(next.par_chunks(chunk))
        .for_each(|((out, coarse), next)| {
            if threshold <= 0.0 {
                for (o, (&c, &s)) in out.iter_mut().zip(coarse.iter().zip(next.iter())) {
                    *o += gain * (c - s);
                }
                return;
            }
            for (o, (&c, &s)) in out.iter_mut().zip(coarse.iter().zip(next.iter())) {
                let d = c - s;
                *o += gain * d.signum() * (d.abs() - threshold).max(0.0);
            }
        });
}

/// Robust sigma of the level-1 detail plane, from a strided subsample.
fn estimate_detail_sigma(coarse: &[f32], next: &[f32], n: usize) -> f32 {
    let stride = (n / MAX_SIGMA_SAMPLES).max(1);
    let mut samples: Vec<f32> = (0..n)
        .step_by(stride)
        .map(|i| coarse[i] - next[i])
        .collect();
    if samples.len() < 32 {
        return 0.0;
    }

    let median = fast_median(&mut samples);
    for v in samples.iter_mut() {
        *v = (*v - median).abs();
    }
    let sigma = fast_median(&mut samples) * MAD_TO_SIGMA;
    if sigma.is_finite() && sigma > 0.0 {
        sigma
    } else {
        0.0
    }
}

/// One à trous smoothing step: separable B3 spline convolution with `hole - 1`
/// zeros between taps, mirrored at the borders.
fn atrous_smooth(
    src: &[f32],
    dst: &mut [f32],
    scratch: &mut [f32],
    width: usize,
    height: usize,
    hole: usize,
) {
    convolve_rows(src, scratch, width, height, hole);
    convolve_cols(scratch, dst, width, height, hole);
}

/// Mirror an out-of-range index back into `0..len`, matching the half-sample
/// reflection the transform assumes at the frame edge.
#[inline]
fn reflect(i: isize, len: usize) -> usize {
    let n = len as isize;
    let mut i = i;
    // A loop rather than one fold: with a level-4 hole of 8 on a narrow frame a
    // single reflection can still land outside.
    while i < 0 || i >= n {
        if i < 0 {
            i = -i;
        }
        if i >= n {
            i = 2 * n - 2 - i;
        }
        if n == 1 {
            return 0;
        }
    }
    i as usize
}

fn convolve_rows(src: &[f32], dst: &mut [f32], width: usize, height: usize, hole: usize) {
    let interior = 2 * hole;
    dst[..width * height]
        .par_chunks_mut(width)
        .with_min_len(8)
        .enumerate()
        .for_each(|(y, out_row)| {
            let row = &src[y * width..][..width];
            let (lo, hi) = interior_bounds(width, interior);
            for x in 0..lo {
                out_row[x] = tap_mirrored(row, x, width, hole);
            }
            for x in lo..hi {
                let mut acc = 0.0;
                for (j, &w) in B3.iter().enumerate() {
                    acc += w * row[x + (j * hole) - interior];
                }
                out_row[x] = acc;
            }
            for x in hi..width {
                out_row[x] = tap_mirrored(row, x, width, hole);
            }
        });
}

fn convolve_cols(src: &[f32], dst: &mut [f32], width: usize, height: usize, hole: usize) {
    let interior = 2 * hole;
    let (lo, hi) = interior_bounds(height, interior);
    dst[..width * height]
        .par_chunks_mut(width)
        .with_min_len(8)
        .enumerate()
        .for_each(|(y, out_row)| {
            if y >= lo && y < hi {
                let base = (y - interior) * width;
                out_row.copy_from_slice(&src[base..][..width]);
                for o in out_row.iter_mut() {
                    *o *= B3[0];
                }
                for (j, &w) in B3.iter().enumerate().skip(1) {
                    let row = &src[(y + j * hole - interior) * width..][..width];
                    for (o, &v) in out_row.iter_mut().zip(row.iter()) {
                        *o += w * v;
                    }
                }
                return;
            }
            for (x, o) in out_row.iter_mut().enumerate() {
                let mut acc = 0.0;
                for (j, &w) in B3.iter().enumerate() {
                    let sy = reflect(y as isize + (j * hole) as isize - interior as isize, height);
                    acc += w * src[sy * width + x];
                }
                *o = acc;
            }
        });
}

/// The range of indices whose whole 5-tap window is in bounds.
#[inline]
fn interior_bounds(len: usize, interior: usize) -> (usize, usize) {
    let lo = interior.min(len);
    let hi = len.saturating_sub(interior).max(lo);
    (lo, hi)
}

/// One mirrored tap of the separable kernel, for the border regions.
#[inline]
fn tap_mirrored(line: &[f32], pos: usize, len: usize, hole: usize) -> f32 {
    let interior = 2 * hole;
    let mut acc = 0.0;
    for (j, &w) in B3.iter().enumerate() {
        let idx = reflect(pos as isize + (j * hole) as isize - interior as isize, len);
        acc += w * line[idx];
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, height: usize, value: f32) -> Vec<f32> {
        vec![value; width * height]
    }

    /// The transform is a partition of unity: with every threshold at zero the
    /// details and the residual must sum back to the input. Any error here is a
    /// kernel or border bug, and would show up in production as a brightness
    /// shift rather than as noise.
    #[test]
    fn zero_thresholds_reconstruct_the_input() {
        let (w, h) = (61, 47);
        let original: Vec<f32> = (0..w * h)
            .map(|i| ((i * 37 % 101) as f32) / 101.0 + ((i / w) as f32) * 0.001)
            .collect();
        let mut luma = original.clone();

        let config = LumaDenoiseConfig {
            enabled: true,
            k: [0.0; MAX_LEVELS],
            gain: UNIT_GAIN,
            strength: 1.0,
        };
        // `is_enabled` is false for an all-zero k, so drive the kernel directly.
        denoise_luma(&mut luma, w, h, &config);

        for (i, (&got, &want)) in luma.iter().zip(original.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-4,
                "sample {i}: reconstruction {got} != input {want}"
            );
        }
    }

    /// A constant field has no detail at any scale, so no threshold can change
    /// it — including at the borders, where the mirrored taps are.
    #[test]
    fn a_flat_field_survives_any_threshold() {
        let (w, h) = (40, 33);
        let mut luma = flat(w, h, 0.25);
        denoise_luma(
            &mut luma,
            w,
            h,
            &LumaDenoiseConfig {
                enabled: true,
                k: [5.0; MAX_LEVELS],
                gain: UNIT_GAIN,
                strength: 1.0,
            },
        );
        for (i, &v) in luma.iter().enumerate() {
            assert!((v - 0.25).abs() < 1e-4, "sample {i} drifted to {v}");
        }
    }

    fn xorshift_noise(n: usize, base: f32, amplitude: f32) -> Vec<f32> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                base + (((state >> 40) as f32 / 16777216.0) - 0.5) * amplitude
            })
            .collect()
    }

    fn sigma(v: &[f32]) -> f32 {
        let mean: f32 = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn mean(v: &[f32]) -> f32 {
        v.iter().sum::<f32>() / v.len() as f32
    }

    /// The kernel works: thresholding the finest scale takes white noise apart,
    /// and the background level does not move while it happens.
    #[test]
    fn thresholding_level_one_removes_white_noise_without_shifting_the_level() {
        let (w, h) = (128, 128);
        let noisy = xorshift_noise(w * h, 0.2, 0.02);
        let mut denoised = noisy.clone();

        let mut config = LumaDenoiseConfig::default();
        config.k[0] = 1.0;
        denoise_luma(&mut denoised, w, h, &config);

        assert!(
            sigma(&denoised) < sigma(&noisy) * 0.3,
            "sigma only fell from {} to {}",
            sigma(&noisy),
            sigma(&denoised)
        );
        assert!(
            (mean(&denoised) - mean(&noisy)).abs() < 1e-3,
            "denoising shifted the background level from {} to {}",
            mean(&noisy),
            mean(&denoised)
        );
    }

    /// The cost of the level-1 exemption, pinned so it's a decision, not a surprise. A B3
    /// spline à trous transform puts ~94% of a *white* signal's variance in level 1
    /// (0.8907² vs 0.2007², 0.0856², 0.0413²); leaving it alone caps what default `k` can
    /// do to pure white noise at a few percent, which this measures.
    ///
    /// Real sky noise isn't white here: the encoder's box downsample correlates
    /// neighbouring samples first, moving variance into levels 2-4 where defaults bite.
    /// On IMX533 the same config takes sky sigma from 6.76 to 4.71 output levels (see
    /// `display_output_tests`); zero star protection takes it to 1.47 — the lever
    /// trading star cores for grain.
    #[test]
    fn the_default_thresholds_barely_touch_white_noise() {
        let (w, h) = (128, 128);
        let noisy = xorshift_noise(w * h, 0.2, 0.02);
        let mut denoised = noisy.clone();
        denoise_luma(&mut denoised, w, h, &LumaDenoiseConfig::default());

        let ratio = sigma(&noisy) / sigma(&denoised);
        assert!(
            (1.0..1.4).contains(&ratio),
            "default k reduced white noise by {ratio:.2}x; the level-1 exemption \
             should hold this near 1x — if it moved, the trade-off changed"
        );
    }

    /// A star is what the level-1 exemption protects. A bright, tight peak must
    /// keep essentially all of its amplitude.
    #[test]
    fn a_tight_peak_keeps_its_amplitude() {
        let (w, h) = (64, 64);
        let mut luma = flat(w, h, 0.05);
        let peak = 0.9;
        luma[32 * w + 32] = peak;
        luma[32 * w + 31] = 0.4;
        luma[32 * w + 33] = 0.4;
        luma[31 * w + 32] = 0.4;
        luma[33 * w + 32] = 0.4;

        denoise_luma(&mut luma, w, h, &LumaDenoiseConfig::default());

        assert!(
            luma[32 * w + 32] > peak * 0.9,
            "star core fell to {} from {peak}",
            luma[32 * w + 32]
        );
    }

    /// The star-protection control, at both ends.
    ///
    /// Full protection must leave the level-1 threshold at zero — that is
    /// exactly the shipped default — and no protection must put it at the
    /// measured ceiling. A mapping that ran the other way would be the worst
    /// kind of bug here: the control would still move grain, just backwards.
    #[test]
    fn star_protection_maps_onto_the_level_one_threshold() {
        let full = LumaDenoiseConfig::thresholds_for_star_protection(1.0);
        assert_eq!(full, DEFAULT_K);
        assert_eq!(full[0], 0.0);

        let none = LumaDenoiseConfig::thresholds_for_star_protection(0.0);
        assert_eq!(none[0], MAX_LEVEL1_K);
        assert_eq!(&none[1..], &DEFAULT_K[1..], "only level 1 may move");

        let half = LumaDenoiseConfig::thresholds_for_star_protection(0.5);
        assert!((half[0] - MAX_LEVEL1_K / 2.0).abs() < 1e-6);

        // Out-of-range input must clamp rather than invert the relationship.
        assert_eq!(
            LumaDenoiseConfig::thresholds_for_star_protection(-1.0)[0],
            MAX_LEVEL1_K
        );
        assert_eq!(
            LumaDenoiseConfig::thresholds_for_star_protection(2.0)[0],
            0.0
        );
    }

    /// What makes the far end of the control safe to offer: at zero protection
    /// Correlated noise is what a stack has, and it is the case a propagated sigma
    /// gets wrong.
    ///
    /// The old model derived every level's noise from level 1 by a fixed ratio, which
    /// holds only for white noise. Asserted here on the ratio itself, so no absolute
    /// scaling enters: a white field lands near the B3 spline's own propagation
    /// (level 4 / level 1 ~ 0.046), and a field whose power has been moved into the
    /// coarse planes lands far above it. A stack is the second kind — registration
    /// warps it and the subs stop being independent — so a derived coarse threshold is
    /// many times too small, which is why 32-128 px mottle survived this filter.
    #[test]
    fn the_coarse_to_fine_noise_ratio_is_not_a_constant() {
        const N: usize = 256;

        let ratio = |plane: &[f32]| -> f32 {
            let mut coarse = plane.to_vec();
            let mut next = vec![0.0f32; N * N];
            let mut scratch = vec![0.0f32; N * N];
            let mut first = 0.0f32;
            let mut last = 0.0f32;
            for level in 0..MAX_LEVELS {
                atrous_smooth(&coarse, &mut next, &mut scratch, N, N, 1 << level);
                let sigma = estimate_detail_sigma(&coarse, &next, N * N);
                if level == 0 {
                    first = sigma;
                }
                last = sigma;
                std::mem::swap(&mut coarse, &mut next);
            }
            last / first
        };

        let white = noise_plane(N, 0.01, 1);
        // One smoothing step at a small hole correlates neighbouring samples, which is
        // what registration warping does to a stack. Its
        // absolute sigma drops too, which is exactly why the assertion is on the ratio.
        let mut correlated = vec![0.0f32; N * N];
        let mut scratch = vec![0.0f32; N * N];
        atrous_smooth(&white, &mut correlated, &mut scratch, N, N, 2);

        let white_ratio = ratio(&white);
        let corr_ratio = ratio(&correlated);

        assert!(
            white_ratio < 0.15,
            "white noise should sit near the spline's own propagation, got {white_ratio:.3}"
        );
        assert!(
            corr_ratio > white_ratio * 3.0,
            "correlated noise must carry far more of its power at the coarse levels: \
             {corr_ratio:.3} against {white_ratio:.3} — a threshold derived from level 1 \
             would be that many times too small"
        );
    }

    /// A field of independent Gaussian-ish noise, deterministic per `seed`.
    fn noise_plane(size: usize, sigma: f32, seed: u64) -> Vec<f32> {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 40) as f32 / 16_777_216.0) - 0.5
        };
        (0..size * size).map(|_| 0.02 + rng() * sigma * 3.46).collect()
    }

    /// the level-1 threshold bites hard on white noise, and the star core still
    /// keeps its peak. This is the kernel-level half of the fixture measurement
    /// in `display_output_tests`.
    #[test]
    fn no_protection_cuts_grain_and_still_keeps_a_star_core() {
        let (w, h) = (128, 128);
        let config = LumaDenoiseConfig {
            enabled: true,
            k: LumaDenoiseConfig::thresholds_for_star_protection(0.0),
            gain: UNIT_GAIN,
            strength: 1.0,
        };

        let noisy = xorshift_noise(w * h, 0.2, 0.02);
        let mut denoised = noisy.clone();
        denoise_luma(&mut denoised, w, h, &config);
        let ratio = sigma(&noisy) / sigma(&denoised);
        assert!(
            ratio > 2.0,
            "no protection should bite hard on white noise; got {ratio:.2}x"
        );

        let mut luma = flat(w, h, 0.05);
        let peak = 0.9;
        luma[64 * w + 64] = peak;
        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            luma[(64 + dy) as usize * w + (64 + dx) as usize] = 0.4;
        }
        denoise_luma(&mut luma, w, h, &config);
        assert!(
            luma[64 * w + 64] > peak * 0.9,
            "star core fell to {} from {peak} at zero protection",
            luma[64 * w + 64]
        );
    }

    #[test]
    fn reflect_handles_degenerate_lengths() {
        assert_eq!(reflect(-3, 1), 0);
        assert_eq!(reflect(7, 1), 0);
        assert_eq!(reflect(-1, 5), 1);
        assert_eq!(reflect(5, 5), 3);
        assert_eq!(reflect(-8, 5), 0);
    }
}
