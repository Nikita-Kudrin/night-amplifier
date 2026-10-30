//! Planetary tone: levels read off the target, then one gentle curve on each pixel's brightest
//! channel ([`crate::render::stretch::ScaleBasis::MaxChannel`]).
//!
//! The Deep Sky curves lift a sky 3 sigma above black to 0.08 and clipped every disc whole. The
//! old planetary profile, a fixed asinh x2 under the deep-sky S-curve, clipped anything past ~48 %
//! of full scale (Jupiter 53-67 % of its disc, the white-light Sun 83 %) and left brightness to
//! exposure x gain: under-exposed, Jupiter's median sat at 21 of 255. Levels from the frame itself
//! fill the range at any exposure, clipped nothing on `Moon_Mosaic`, `Planets` or the drifting
//! Moon, and move mid-grey by 0.1-0.3 levels frame to frame (1.1 worst), so nothing is smoothed.

mod daylight;
mod sky;
mod target;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod target_review_tests;

pub use daylight::{DaylightBalance, DaylightBalanceSource};
pub use target::{NotTheMoon, TargetMeasure, MOON_COLOUR};

use crate::frame::Frame;
use crate::render::stretch::{asinh, quantize_relative, scale_lut_from, ScaleBasis, ScaleLut};
use rayon::prelude::*;
use std::cell::RefCell;
use std::sync::Arc;

/// Bins of the brightest-channel histogram the levels are read from, at full resolution: 4 ADU
/// of a 16-bit sensor, interpolated within. A sampled one (`estimate_background_mode`'s 50k)
/// saw eight pixels of Uranus, a 675 px disc on 2712x1538.
const BINS: usize = 16_384;

/// Fewer pixels than this can set neither the peak nor the white point. The peak is read off
/// each pixel's dimmer with its right-hand neighbour, which an isolated hot pixel or cosmic ray
/// never reaches: a rank alone needed one pixel in 10 000 to be safe, which on a 675 px Uranus
/// put the target's threshold in its faint skirt, and on a 26 MP sensor in the sky.
const LEAST_PIXELS: usize = 16;

/// A pixel brighter than this share of the peak is target; below it, sky. A frame-filling full
/// Moon has nothing below it: the deep-sky mode estimate read its maria as the sky (0.30), and
/// a black point there rendered them black.
const TARGET_SHARE: f32 = 0.12;

/// The share of the frame below the target over which its sky takes the black point over from
/// the sensor's own black, 0: a zoomed-in Moon or Sun has no sky. A band, not a step, so a disc
/// drifting across the frame's edge walks its black point over rather than flipping it.
const SKY_SHARE_BAND: (f32, f32) = (0.005, 0.02);

/// The sky's level, as shares of the target's threshold, over which it fades out of the black
/// point as haze or twilight lifts it towards the target. [`SKY_SHARE_BAND`] counts pixels, so
/// a sky crossing the threshold left that band within its own noise: the black point fell from
/// 0.068 to 0 for a 0.001 rise. Past the threshold a sky counts as target, so a daylight sky
/// at a fifth of the disc is still not taken off.
const SKY_LEVEL_BAND: (f32, f32) = (0.5, 1.0);

/// The target's white point: 0.1 % of its pixel pairs may pass it, [`LEAST_PIXELS`] at least.
const WHITE_QUANTILE: f64 = 0.999;

/// Least white-to-black span, capping the gain at ~50x: a slew or a cloud leaves nothing but
/// noise, which levels alone would stretch to full range.
const MIN_SPAN: f32 = 0.02;

/// Where the white point renders: headroom over it for the 0.1 % above.
pub const WHITE_OUTPUT: f32 = 0.94;

/// The curve's lift of the darker half: `asinh(2x)/asinh(2)`, x1.38 at the bottom. Over plain
/// levels it took Jupiter's moon from 38 to 51 levels at a 10-30 % lower highlight slope; at 3
/// the Sun's limb darkening flattened, and gamma 0.8 doubled the sky's noise (slope unbounded
/// at black).
const CURVE_BETA: f32 = 2.0;

/// Black and white points of one planetary frame, on its brightest channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlanetaryLevels {
    pub black: f32,
    pub white: f32,
    /// The median below the target, or `None` with nothing below it. It sets the black point
    /// only as far as [`SKY_SHARE_BAND`] says the frame has a sky, and [`SKY_LEVEL_BAND`] that
    /// it lies well under the target.
    pub sky: Option<f32>,
    /// The brightness above which a pixel counts as target.
    pub target_from: f32,
}

impl PlanetaryLevels {
    /// The levels of `frame` (one or three channels), from its brightest-channel histogram.
    pub fn solve(frame: &Frame) -> Self {
        let Histogram { brightness: histogram, pairs } = Histogram::of(frame);
        let total = histogram.total;
        let peak = pairs.value_in(0..BINS, pairs.total.saturating_sub(LEAST_PIXELS));
        let target_from = TARGET_SHARE * peak;
        let split = Counts::bin(target_from);
        let below: usize = histogram.counts[..split].iter().map(|&c| c as usize).sum();

        let sky = (below > 0).then(|| histogram.value_in(0..split, below / 2));
        let target_pairs = pairs.total - pairs.counts[..split].iter().map(|&c| c as usize).sum::<usize>();
        let above_white = LEAST_PIXELS.max(((1.0 - WHITE_QUANTILE) * target_pairs as f64).ceil() as usize);
        let white = pairs.value_in(split..BINS, target_pairs.saturating_sub(above_white));
        let share = band(below as f32 / total.max(1) as f32, SKY_SHARE_BAND);
        let black = sky.map_or(0.0, |sky| {
            let under_target = 1.0 - band(sky / target_from.max(f32::MIN_POSITIVE), SKY_LEVEL_BAND);
            share * under_target * sky
        });
        Self {
            black,
            white: white.max(black + MIN_SPAN),
            sky,
            target_from,
        }
    }

    /// White over black: what the curve's input is a share of.
    pub fn span(&self) -> f32 {
        self.white - self.black
    }
}

/// Where `x` lies across `(low, high)`, 0 to 1.
fn band(x: f32, (low, high): (f32, f32)) -> f32 {
    ((x - low) / (high - low)).clamp(0.0, 1.0)
}

/// Planetary's tone curve at `x`, the brightness above black as a share of the span.
pub fn curve(x: f32) -> f32 {
    (WHITE_OUTPUT * asinh(CURVE_BETA * x.max(0.0)) / asinh(CURVE_BETA)).min(1.0)
}

thread_local! {
    /// The last table built on this thread, keyed on its span. Per thread for the reason
    /// `stretch::SCALE_LUT_CACHE` is: the live render and a stacked-PNG render run at once.
    static LUT_CACHE: RefCell<Option<(u32, Arc<ScaleLut>)>> = const { RefCell::new(None) };
}

/// The max-channel scale table for `span`: `curve(v / span) / v` at every brightness `v`
/// above black. Cached per thread on the span, quantised relatively (`quantize_relative`).
pub fn scale_lut(span: f32) -> Arc<ScaleLut> {
    let key = quantize_relative(span);
    LUT_CACHE.with(|cell| {
        let cache = &mut *cell.borrow_mut();
        match cache {
            Some((cached, lut)) if *cached == key => Arc::clone(lut),
            _ => {
                let lut = Arc::new(scale_lut_from(|v| curve(v / span) / v).with_basis(ScaleBasis::MaxChannel));
                Arc::clone(&cache.insert((key, lut)).1)
            }
        }
    })
}

/// Samples counted into [`BINS`] even bins over [0, 1].
struct Counts {
    counts: Vec<u32>,
    total: usize,
}

/// A frame's brightest channel per pixel, and per horizontal pair of pixels the dimmer of the
/// two: one pass, both counted.
struct Histogram {
    brightness: Counts,
    pairs: Counts,
}

impl Histogram {
    /// Row by row, so a pair never spans two rows and each pixel's brightness is read once: a
    /// flat walk tested every pixel for a row's end, an integer division a sample.
    fn of(frame: &Frame) -> Self {
        let (width, height) = (frame.width(), frame.height());
        let empty = || (vec![0u32; BINS], vec![0u32; BINS]);
        let rows = (crate::parallel::balanced_chunk_len(width * height) / width.max(1)).max(1);
        let (r, g, b) = match frame.channels() {
            3 => frame.planes(),
            _ => (frame.channel_data(0), frame.channel_data(0), frame.channel_data(0)),
        };
        let (counts, pairs) = (0..height.div_ceil(rows))
            .into_par_iter()
            .fold(empty, |(mut counts, mut pairs), k| {
                for y in k * rows..((k + 1) * rows).min(height) {
                    let row = y * width..(y + 1) * width;
                    let (r, g, b) = (&r[row.clone()], &g[row.clone()], &b[row]);
                    let mut brightness = r.iter().zip(g).zip(b).map(|((r, g), b)| r.max(*g).max(*b));
                    let Some(mut left) = brightness.next() else { continue };
                    for v in brightness {
                        counts[Counts::bin(left)] += 1;
                        pairs[Counts::bin(left.min(v))] += 1;
                        left = v;
                    }
                    counts[Counts::bin(left)] += 1;
                }
                (counts, pairs)
            })
            .reduce(empty, |(mut a, mut p), (b, q)| {
                a.iter_mut().zip(&b).for_each(|(a, b)| *a += b);
                p.iter_mut().zip(&q).for_each(|(p, q)| *p += q);
                (a, p)
            });
        Self {
            brightness: Counts { counts, total: width * height },
            pairs: Counts { counts: pairs, total: width.saturating_sub(1) * height },
        }
    }
}

impl Counts {
    /// The bin holding `v`; NaN and negatives the bottom, 1.0 and over the top.
    fn bin(v: f32) -> usize {
        ((v * BINS as f32) as usize).min(BINS - 1)
    }

    /// The value of the sample at `rank` (from the bottom) among the samples in `bins`,
    /// interpolated within its bin.
    fn value_in(&self, bins: std::ops::Range<usize>, rank: usize) -> f32 {
        let end = bins.end;
        let mut before = 0usize;
        for bin in bins {
            let count = self.counts[bin] as usize;
            if before + count > rank {
                let within = (rank - before) as f32 + 0.5;
                return ((bin as f32 + within / count as f32) / BINS as f32).min(1.0);
            }
            before += count;
        }
        (end as f32 / BINS as f32).min(1.0)
    }
}
