//! The soft half of the darkening black floor: a *gain* on the sky, chosen per pixel
//! from a 3x3 mean of the stretched luminance rather than from the pixel itself.
//!
//! Why spatial: sky grain after the stretch is ~0.35x the sky level, and any pointwise
//! curve that keeps a faint target's excess keeps noise excursions of the same size.
//! The softplus knee this replaced flattened the lower half of the noise while the
//! upper half survived — 20-35 % of sky pixels within one level of the pedestal and
//! relative grain up 1.7-2x (2026-09-14 globular, IMX533): blocky dark clumps with
//! bright specks at a pixel-resolving eyepiece. A 3x3 mean has a third of the noise,
//! so flat sky takes the full gain (relative grain kept) while coherent structure
//! above the sky, and stars, keep their level.
//!
//! Measured on that render at a 3x3 guide and a 1.05-2.0 sky shoulder: grain x0.81,
//! 89 % of the halo's excess kept, sky beside a star lifted 1.16x. A 5x5 guide lifted
//! it 1.6x — a bright square around every star.
//!
//! Everything is expressed per row ([`luma_row`], [`guide_row`], [`SkyShadow::apply_row`])
//! and the sky is sampled at fixed positions, so the encoder's streamed rows (denoised or
//! not), the whole-image reference and the planar frame path compute the same image.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

/// Sky darkening per unit of [`super::ShadowFloorRequest::fraction`]. -5 % measures 64 %
/// darker on the IMX533 fixture (the knee: 65-71 %), the guard pedestal included; the
/// slider's -6 % end stop reaches [`MAX_DARKENING`].
const DARKENING_PER_FRACTION: f32 = 0.8;

/// Darkest gain the slider can reach: the sky keeps a tenth of its level.
const MAX_DARKENING: f32 = 0.9;

/// Guide level, in sky levels, below which the full gain applies. Just above the sky:
/// the 3x3 guide's noise is ~0.12 sky, so flat sky seldom reaches the shoulder.
const SHOULDER_START: f32 = 1.05;

/// Guide level, in sky levels, from which nothing is darkened.
const SHOULDER_END: f32 = 2.0;

/// Rec.709 weights, the luminance every luminance-preserving kernel here uses.
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Rows and columns per row the sky is measured on: <= 65k guide samples, and fixed
/// positions, so a path that only renders those rows measures the same sky.
const SAMPLE_ROWS: usize = 32;
const SAMPLE_COLUMNS: usize = 256;

/// Histogram bins per sigma of the sky's guide spread, and the cap on bin count.
const BINS_PER_SIGMA: f32 = 4.0;
const MAX_BINS: usize = 4096;

/// Share of samples a histogram peak needs to count as the sky. A registration border
/// (5 % of a drifting stack at one dark level) is a tall, narrow spike, not the sky.
const MIN_SKY_SHARE: f32 = 0.2;

/// Darkest sky accepted, as a fraction of the solver's anchor. A foreground darker than
/// the sky over a third of the frame (roof, tree, dew shadow) is a qualifying peak too:
/// taken at 0.3 sky, it left the real sky past the shoulder, undarkened. The anchor has
/// missed the rendered sky by 1.3x (IMX464), well inside this.
const MIN_SKY_OF_ANCHOR: f32 = 0.5;

/// A resolved soft darkening: `gain` on flat sky at `sky` (output-referred).
///
/// `sky` from the solve is only a fallback; the kernels re-measure it on the guide. The
/// solver's target put the IMX464 fixture's sky at 13 levels where it rendered at 17,
/// inside the shoulder — the gain followed the noise and relative grain rose 1.4x.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyShadow {
    pub gain: f32,
    pub sky: f32,
}

impl SkyShadow {
    /// `None` when the request darkens nothing.
    pub fn from_sky(fraction: f32, sky_level: f32) -> Option<Self> {
        let darkening = (fraction.max(0.0) * DARKENING_PER_FRACTION).min(MAX_DARKENING);
        if darkening <= 0.0 || sky_level <= 0.0 || !sky_level.is_finite() {
            return None;
        }
        Some(Self {
            gain: 1.0 - darkening,
            sky: sky_level,
        })
    }

    /// This shadow re-anchored to the sky measured on guide `samples`.
    pub(crate) fn with_measured_sky(self, samples: &[f32]) -> Self {
        match estimate_sky(samples, MIN_SKY_OF_ANCHOR * self.sky) {
            Some(sky) => Self { sky, ..self },
            None => self,
        }
    }

    /// Multiplier for a pixel of luminance `luma` whose 3x3 mean is `mean`. The guide
    /// is the larger of the mean and the pixel's own excess over one sky level, so an
    /// undersampled star core is not averaged down by its dark neighbours; noise needs
    /// +3 sigma (~0.13 % of sky pixels) to reach the shoulder that way.
    #[inline]
    pub fn multiplier_at(&self, luma: f32, mean: f32) -> f32 {
        self.multiplier(mean.max(luma - self.sky))
    }

    /// Smoothstep from `gain` to 1 across the shoulder; never decreasing, so a
    /// brighter guide never renders darker. Branch-free, so row loops vectorise.
    #[inline]
    pub fn multiplier(&self, guide: f32) -> f32 {
        let end = (SHOULDER_END * self.sky).min(1.0);
        let start = (SHOULDER_START * self.sky).min(end);
        let t = ((guide - start) / (end - start).max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
        self.gain + (1.0 - self.gain) * t * t * (3.0 - 2.0 * t)
    }

    /// Scale one interleaved RGB row, given its luminance and guide rows.
    pub(crate) fn apply_row(&self, rgb: &mut [f32], luma: &[f32], guide: &[f32]) {
        for ((px, &l), &g) in rgb.chunks_exact_mut(3).zip(luma).zip(guide) {
            let m = self.multiplier_at(l, g);
            px.iter_mut().for_each(|v| *v *= m);
        }
    }
}

/// Rows the sky is sampled on, for a frame `height` rows tall.
pub(crate) fn sky_sample_rows(height: usize) -> Vec<usize> {
    if height <= SAMPLE_ROWS {
        return (0..height).collect();
    }
    (0..SAMPLE_ROWS)
        .map(|k| (2 * k + 1) * height / (2 * SAMPLE_ROWS))
        .collect()
}

/// Column stride of the sky samples within a sampled row.
pub(crate) fn sky_sample_stride(width: usize) -> usize {
    (width / SAMPLE_COLUMNS).max(1)
}

/// Rec.709 luminance of an interleaved RGB row.
pub(crate) fn luma_row(rgb: &[f32], out: &mut [f32]) {
    for (o, px) in out.iter_mut().zip(rgb.chunks_exact(3)) {
        *o = LUMA[0] * px[0] + LUMA[1] * px[1] + LUMA[2] * px[2];
    }
}

/// 3x3 mean of row `mid` with its neighbours (pass `mid` again at a frame edge),
/// replicating edge columns.
pub(crate) fn guide_row(up: &[f32], mid: &[f32], down: &[f32], out: &mut [f32]) {
    let width = out.len();
    if width == 0 {
        return;
    }
    let column = |x: usize| up[x] + mid[x] + down[x];
    if width == 1 {
        out[0] = column(0) / 3.0;
        return;
    }
    // Interior: sum three column sums, zipped so the loop vectorises.
    let (up, mid, down) = (&up[..width], &mid[..width], &down[..width]);
    for ((o, (u, m)), d) in out[1..width - 1]
        .iter_mut()
        .zip(up.windows(3).zip(mid.windows(3)))
        .zip(down.windows(3))
    {
        *o = ((u[0] + u[1] + u[2]) / 3.0 + (m[0] + m[1] + m[2]) / 3.0 + (d[0] + d[1] + d[2]) / 3.0) / 3.0;
    }
    let edge = |a: usize, b: usize| {
        ((up[a] + up[a] + up[b]) / 3.0 + (mid[a] + mid[a] + mid[b]) / 3.0 + (down[a] + down[a] + down[b]) / 3.0) / 3.0
    };
    out[0] = edge(0, 1);
    out[width - 1] = edge(width - 1, width - 2);
}

/// The sky level in guide samples: the darkest histogram peak holding at least
/// [`MIN_SKY_SHARE`] of them within its half maximum and above `floor`, refined to the
/// median of the samples there.
///
/// Not the median of all samples: a faint target over most of the frame *is* the median.
/// A 1.6-sky nebula over 60 % of a frame was darkened like the sky (kept 50 % of its
/// level where the shoulder keeps ~79 %) and lost half its contrast. Bins are sized from
/// the 5th-25th percentile gap (~1 sigma of whatever the darkest fifth is), so the
/// histogram resolves the sky however wide the target makes the full range.
fn estimate_sky(samples: &[f32], floor: f32) -> Option<f32> {
    // Selection, not a sort: 92k samples sorted cost ~2 ms of a 4.6 ms encode.
    let mut finite: Vec<f32> = samples.iter().copied().filter(|v| v.is_finite()).collect();
    let n = finite.len();
    if n < 16 {
        return None;
    }
    let (hi, q25, q05, q50) = {
        let mut quantile = |q: f32| quantile(&mut finite, q);
        (quantile(0.99), quantile(0.25), quantile(0.05), quantile(0.5))
    };
    let lo = finite.iter().copied().fold(f32::MAX, f32::min);
    let mut spread = q25 - q05;
    if spread <= 0.0 {
        // >= 20 % tied at the low end (zeros where the black point clamped an obstruction):
        // size the bins from what lies above the tie. The median of everything was 7 %
        // under the sky at 30 % zeros, and nothing at all from 50 %.
        let mut above: Vec<f32> = finite.iter().copied().filter(|v| *v > q25).collect();
        if above.len() >= 16 {
            spread = quantile(&mut above, 0.25) - quantile(&mut above, 0.05);
        }
    }
    if hi <= lo || spread <= 0.0 {
        return (q50 > floor.max(0.0)).then_some(q50);
    }

    let width = (spread / BINS_PER_SIGMA).max((hi - lo) / MAX_BINS as f32);
    let bins = (((hi - lo) / width) as usize + 1).min(MAX_BINS);
    let mut counts = vec![0u32; bins];
    for &v in finite.iter().filter(|v| **v <= hi) {
        counts[(((v - lo) / width) as usize).min(bins - 1)] += 1;
    }
    let smoothed: Vec<u32> = (0..bins)
        .map(|i| counts[i.saturating_sub(2)..(i + 3).min(bins)].iter().sum())
        .collect();

    let needed = (MIN_SKY_SHARE * n as f32) as u32;
    let mut i = 0;
    while i < bins {
        if smoothed[i] == 0 {
            i += 1;
            continue;
        }
        // Grow right while above half of the highest bin seen, then back left from that
        // peak: one half-maximum region, however the noise wiggles inside it.
        let (mut peak, mut right) = (i, i);
        while right + 1 < bins && 2 * smoothed[right + 1] >= smoothed[peak] {
            right += 1;
            if smoothed[right] > smoothed[peak] {
                peak = right;
            }
        }
        let mut left = peak;
        while left > 0 && 2 * smoothed[left - 1] >= smoothed[peak] {
            left -= 1;
        }
        if counts[left..=right].iter().sum::<u32>() >= needed {
            let (from, to) = (lo + left as f32 * width, lo + (right + 1) as f32 * width);
            let mut near: Vec<f32> = finite.iter().copied().filter(|v| *v >= from && *v < to).collect();
            // Bin edges recomputed in f32 can miss every sample of a one-bin peak.
            if !near.is_empty() {
                let middle = near.len() / 2;
                let sky = *near.select_nth_unstable_by(middle, |a, b| a.total_cmp(b)).1;
                if sky > floor.max(0.0) {
                    return Some(sky);
                }
            }
        }
        i = right + 1;
    }
    None
}

/// The `q` quantile of `values` by selection (reorders them).
fn quantile(values: &mut [f32], q: f32) -> f32 {
    let k = ((values.len() - 1) as f32 * q) as usize;
    *values.select_nth_unstable_by(k, |a, b| a.total_cmp(b)).1
}

/// Apply to a whole interleaved RGB f32 image (already stretched, contrast applied):
/// the reference the encoder's streamed rows are pinned against. `luma` and `guide` are
/// grown to `width * height`.
#[cfg(test)]
pub(crate) fn apply_sky_shadow_interleaved(
    rgb: &mut [f32],
    width: usize,
    height: usize,
    shadow: SkyShadow,
    luma: &mut Vec<f32>,
    guide: &mut Vec<f32>,
) {
    let n = width * height;
    if n == 0 || rgb.len() < n * 3 {
        return;
    }
    let _span = tracing::info_span!("sky_shadow", width, height).entered();
    let luma = crate::render::denoise::take(luma, n);
    let guide = crate::render::denoise::take(guide, n);

    luma.par_chunks_mut(width)
        .zip(rgb.par_chunks(width * 3))
        .for_each(|(out, row)| luma_row(row, out));
    guides_from_luma(luma, guide, width, height);
    let shadow = shadow.with_measured_sky(&sample_guide(guide, width, height));

    rgb.par_chunks_mut(width * 3)
        .zip(luma.par_chunks(width).zip(guide.par_chunks(width)))
        .for_each(|(row, (l, g))| shadow.apply_row(row, l, g));
}

/// Apply to a planar 1- or 3-channel frame, the unfused counterpart the
/// `auto_stretch_frame` arms use. Must agree with [`apply_sky_shadow_interleaved`].
pub fn apply_sky_shadow_frame(frame: &mut Frame, shadow: SkyShadow) -> Result<()> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "apply_sky_shadow_frame requires 1 or 3 channels, got {channels}"
        )));
    }
    let (width, height) = (frame.width(), frame.height());
    let n = width * height;
    if n == 0 {
        return Ok(());
    }

    let mut luma = vec![0.0f32; n];
    if channels == 1 {
        luma.copy_from_slice(frame.channel_data(0));
    } else {
        let (r, g, b) = frame.planes();
        luma.par_iter_mut()
            .zip(r.par_iter().zip(g.par_iter().zip(b.par_iter())))
            .for_each(|(o, (&r, (&g, &b)))| *o = LUMA[0] * r + LUMA[1] * g + LUMA[2] * b);
    }
    let mut guide = vec![0.0f32; n];
    guides_from_luma(&luma, &mut guide, width, height);
    let shadow = shadow.with_measured_sky(&sample_guide(&guide, width, height));
    // `luma` becomes the multiplier, so every channel scales by one value.
    luma.par_iter_mut()
        .zip(guide.par_iter())
        .for_each(|(l, &g)| *l = shadow.multiplier_at(*l, g));

    for c in 0..channels {
        frame
            .channel_data_mut(c)
            .par_iter_mut()
            .zip(luma.par_iter())
            .for_each(|(v, &m)| *v *= m);
    }
    Ok(())
}

fn guides_from_luma(luma: &[f32], guide: &mut [f32], width: usize, height: usize) {
    guide
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, out)| {
            let row = |r: usize| &luma[r * width..(r + 1) * width];
            guide_row(row(y.saturating_sub(1)), row(y), row((y + 1).min(height - 1)), out);
        });
}

fn sample_guide(guide: &[f32], width: usize, height: usize) -> Vec<f32> {
    let stride = sky_sample_stride(width);
    sky_sample_rows(height)
        .into_iter()
        .flat_map(|y| guide[y * width..(y + 1) * width].iter().step_by(stride).copied())
        .collect()
}

#[cfg(test)]
mod tests {
    include!("sky_shadow_tests.rs");
}
