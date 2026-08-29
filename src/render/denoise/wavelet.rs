//! À trous (starlet) wavelet denoising of the luminance plane.
//!
//! The transform is the standard one for astronomical images: repeated
//! separable convolution with the B3 spline kernel, the holes doubling each
//! level, so level `l` isolates structure around `2^l` pixels across. Detail
//! planes are soft-thresholded against the noise level implied by the finest
//! one and the image is rebuilt from what survives plus the coarsest residual.
//!
//! # Why the thresholds get *weaker* with scale
//!
//! The obvious tuning — denoise hardest at the coarse scales, where the eye
//! notices mottle most — erases the target. On a 0.62"/px image the Dumbbell's
//! outer lobes are level-3 and level-4 structure; a `k` of 3 there removes
//! them along with the noise. Sky noise is overwhelmingly fine-scale, so the
//! defaults run hardest just above the stars and back off as scale grows.
//!
//! Level 1 is left alone entirely (`k = 0`). It carries the star cores at this
//! sampling, and a star clipped at level 1 loses its peak without losing its
//! wings, which reads as a bloated blob rather than as a cleaner frame.

use rayon::prelude::*;

use crate::statistics::fast_median;

/// B3 spline scaling kernel, `[1, 4, 6, 4, 1] / 16`.
const B3: [f32; 5] = [0.0625, 0.25, 0.375, 0.25, 0.0625];

/// Scales a MAD into a Gaussian sigma.
const MAD_TO_SIGMA: f32 = 1.4826;

/// How the noise standard deviation of a Gaussian propagates into each detail
/// plane of a B3 spline à trous transform (Starck & Murtagh). Only the ratios
/// matter here: the level-1 plane is measured, the rest are derived from it.
const LEVEL_SIGMA: [f32; MAX_LEVELS] = [0.8907, 0.2007, 0.0856, 0.0413];

/// Detail planes computed. Level 5 would isolate structure ~32 px across,
/// which at display resolution is the target itself rather than its texture.
pub const MAX_LEVELS: usize = 4;

/// Samples drawn to estimate the level-1 noise. A robust sigma converges long
/// before a full-plane sort is worth 2 M elements of work per frame.
const MAX_SIGMA_SAMPLES: usize = 1 << 16;

/// À trous wavelet denoising of the luminance plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaDenoiseConfig {
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

impl Default for LumaDenoiseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            k: DEFAULT_K,
            strength: 1.0,
        }
    }
}

/// Hardest at the finest scale above the stars, backing off as scale grows.
pub const DEFAULT_K: [f32; MAX_LEVELS] = [0.0, 3.0, 2.0, 1.0];

impl LumaDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        k: DEFAULT_K,
        strength: 1.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.strength > 0.0 && self.k.iter().any(|&k| k > 0.0)
    }

    /// Thresholds actually applied, in sigmas of each level's own noise.
    fn scaled_k(&self) -> [f32; MAX_LEVELS] {
        let s = self.strength.clamp(0.0, 4.0);
        std::array::from_fn(|i| self.k[i].max(0.0) * s)
    }
}

/// Denoise `luma` in place. `width * height` samples, linear light.
pub fn denoise_luma(luma: &mut [f32], width: usize, height: usize, config: &LumaDenoiseConfig) {
    let n = width * height;
    if n == 0 || luma.len() < n {
        return;
    }

    let k = config.scaled_k();

    // `luma` becomes the reconstruction accumulator: the detail planes are added
    // into it and the coarsest residual last, so no fourth full-size buffer is
    // needed to hold the sum.
    let mut coarse = luma[..n].to_vec();
    let mut next = vec![0.0f32; n];
    let mut scratch = vec![0.0f32; n];
    luma[..n].fill(0.0);

    let mut level_sigma = 0.0f32;

    for level in 0..MAX_LEVELS {
        let hole = 1usize << level;
        // A hole wider than the image reduces to a plain copy: every tap lands
        // on the same mirrored sample, so the detail plane is zero and the
        // remaining levels have nothing left to say.
        if hole >= width.max(height) {
            break;
        }

        atrous_smooth(&coarse, &mut next, &mut scratch, width, height, hole);

        if level == 0 {
            level_sigma = estimate_detail_sigma(&coarse, &next, n);
        }

        let threshold = k[level] * level_sigma * (LEVEL_SIGMA[level] / LEVEL_SIGMA[0]);
        accumulate_detail(&mut luma[..n], &coarse, &next, threshold);

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

/// `out += soft_threshold(coarse - next, threshold)`.
///
/// Soft rather than hard thresholding: hard leaves a discontinuity at the
/// threshold, which on a low-slope sky turns into visible blotches exactly
/// where the filter was supposed to smooth.
fn accumulate_detail(out: &mut [f32], coarse: &[f32], next: &[f32], threshold: f32) {
    let chunk = crate::parallel::balanced_chunk_len(out.len());
    out.par_chunks_mut(chunk)
        .zip(coarse.par_chunks(chunk))
        .zip(next.par_chunks(chunk))
        .for_each(|((out, coarse), next)| {
            if threshold <= 0.0 {
                for (o, (&c, &s)) in out.iter_mut().zip(coarse.iter().zip(next.iter())) {
                    *o += c - s;
                }
                return;
            }
            for (o, (&c, &s)) in out.iter_mut().zip(coarse.iter().zip(next.iter())) {
                let d = c - s;
                *o += d.signum() * (d.abs() - threshold).max(0.0);
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

    /// The cost of the level-1 exemption, pinned so it is a decision rather
    /// than a surprise.
    ///
    /// A B3 spline à trous transform puts roughly 94 % of a *white* signal's
    /// variance in the level-1 detail plane (0.8907² against 0.2007², 0.0856²
    /// and 0.0413²). Leaving that plane alone to protect star cores therefore caps
    /// what the default `k` can do to pure white noise at a few per cent, which
    /// is what this measures.
    ///
    /// Real sky noise is not white by the time it reaches here: the encoder's
    /// box downsample correlates neighbouring samples first, which moves
    /// variance into levels 2-4 where the default thresholds do bite. On the
    /// IMX533 fixture the same configuration takes sky sigma from 7.41 to 4.45
    /// output levels — see `display_output_tests`. Raising `k[0]` takes it to
    /// 1.48 and is the lever that trades star cores for grain.
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

    #[test]
    fn reflect_handles_degenerate_lengths() {
        assert_eq!(reflect(-3, 1), 0);
        assert_eq!(reflect(7, 1), 0);
        assert_eq!(reflect(-1, 5), 1);
        assert_eq!(reflect(5, 5), 3);
        assert_eq!(reflect(-8, 5), 0);
    }
}
