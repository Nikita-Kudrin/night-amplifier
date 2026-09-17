//! The downsample kernel, as per-output-pixel source weights along one axis.
//!
//! Each output pixel integrates its exact source footprint `[i*s, (i+1)*s)` over the
//! source *linearly interpolated* (a 1 px tent per sample), not over whole source pixels.
//! A whole-pixel box at a non-integer ratio averages 2 samples on most lines and 3 on
//! every ~11th (3008 -> 1440): those lines carried 18 % less noise, a lattice at 18 arcmin
//! through a 100 mm eyepiece lens. Fractional box edges alone still vary 1.26x in 2D; the
//! tent keeps ratios from 1.4x up under 1.05x.
//!
//! The tent costs sharpness near unity — a 2.5 px star kept 83 % of the box's peak at
//! 1.07x (IMX464 at the 1440 tier), 90 % at 1.42x — so a `[-a, 1+2a, -a]` sharpen on the
//! output grid, folded into the taps, gives it back. Shift-invariant on the output grid,
//! it scales every pixel's noise alike and prints no lattice of its own. `a` tapers with
//! the ratio: from 1.9x the tent alone already beats the box's worst-phase peak.

/// Sharpen amount at and below [`SHARPEN_FULL_BELOW`]: 1.07x kept 97-99 % of the box's
/// peak with 10 % less grain than it.
const SHARPEN_MAX: f64 = 0.15;

/// Ratio up to which [`SHARPEN_MAX`] applies.
const SHARPEN_FULL_BELOW: f64 = 1.1;

/// Ratio from which no sharpening applies (2.09x: tent peak 0.97 average, 1.10 worst phase
/// against the box).
const SHARPEN_NONE_FROM: f64 = 1.9;

use std::sync::{Arc, Mutex};

/// Axes kept by [`AxisTaps::cached`]: two per tier (four tiers) per camera, main and guide.
const CACHED_AXES: usize = 16;

static CACHE: Mutex<Vec<((usize, usize), Arc<AxisTaps>)>> = Mutex::new(Vec::new());

pub(super) struct AxisTaps {
    taps: usize,
    pub(super) start: Vec<usize>,
    weights: Vec<f32>,
}

impl AxisTaps {
    /// [`AxisTaps::new`], built once per `(source_len, target_len)`: every encode of every
    /// tier otherwise rebuilt ~4k small weight vectors per frame.
    pub(super) fn cached(source_len: usize, target_len: usize) -> Arc<Self> {
        let key = (source_len, target_len);
        let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(position) = cache.iter().position(|(k, _)| *k == key) {
            // Most recent last, so eviction takes the least recently used.
            let entry = cache.remove(position);
            let taps = entry.1.clone();
            cache.push(entry);
            return taps;
        }
        let taps = Arc::new(Self::new(source_len, target_len));
        if cache.len() == CACHED_AXES {
            cache.remove(0);
        }
        cache.push((key, taps.clone()));
        taps
    }

    pub(super) fn new(source_len: usize, target_len: usize) -> Self {
        let scale = source_len as f64 / target_len as f64;
        let base: Vec<(usize, Vec<f64>)> =
            (0..target_len).map(|i| footprint(source_len, scale, i)).collect();
        let sharpen = sharpen_amount(scale);

        let rows: Vec<(usize, Vec<f64>)> = if sharpen == 0.0 || target_len < 3 {
            base
        } else {
            (0..target_len)
                .map(|i| {
                    let prev = &base[i.saturating_sub(1)];
                    let next = &base[(i + 1).min(target_len - 1)];
                    combine(&[(1.0 + 2.0 * sharpen, &base[i]), (-sharpen, prev), (-sharpen, next)])
                })
                .collect()
        };

        let taps = rows.iter().map(|(_, w)| w.len()).max().unwrap_or(1);
        let mut weights = vec![0.0f32; target_len * taps];
        let mut start = Vec::with_capacity(target_len);
        for ((first, row), out) in rows.iter().zip(weights.chunks_exact_mut(taps)) {
            // Shift the window left where it would run off the end, so every row reads
            // `taps` samples inside the frame.
            let first_in = (*first).min(source_len.saturating_sub(taps));
            let offset = first - first_in;
            for (k, &w) in row.iter().enumerate() {
                out[offset + k] = w as f32;
            }
            start.push(first_in);
        }
        Self {
            taps,
            start,
            weights,
        }
    }

    pub(super) fn of(&self, i: usize) -> (usize, &[f32]) {
        (self.start[i], &self.weights[i * self.taps..(i + 1) * self.taps])
    }
}

/// `a` for a downsample ratio: full near unity, none from 1.9x, linear between.
pub(super) fn sharpen_amount(scale: f64) -> f64 {
    let t = (SHARPEN_NONE_FROM - scale) / (SHARPEN_NONE_FROM - SHARPEN_FULL_BELOW);
    SHARPEN_MAX * t.clamp(0.0, 1.0)
}

/// Normalised area-tent weights of output pixel `i`, from source sample `first`.
fn footprint(source_len: usize, scale: f64, i: usize) -> (usize, Vec<f64>) {
    let (lo, hi) = (i as f64 * scale, (i + 1) as f64 * scale);
    // Sample j's tent spans [j - 0.5, j + 1.5], so the samples overlapping the footprint
    // lie in (lo - 1.5, hi + 0.5). Frame edges drop weight; normalising restores it.
    let first = ((lo - 1.5).floor() + 1.0).max(0.0) as usize;
    let end = (((hi + 0.5).ceil()) as usize).min(source_len);
    let mut w: Vec<f64> = (first..end)
        .map(|j| {
            let centre = j as f64 + 0.5;
            tent_cdf(hi - centre) - tent_cdf(lo - centre)
        })
        .collect();
    let sum: f64 = w.iter().sum();
    if sum > 0.0 {
        w.iter_mut().for_each(|v| *v /= sum);
    }
    (first, w)
}

/// Weighted sum of sparse weight rows, over the union of their ranges.
fn combine(parts: &[(f64, &(usize, Vec<f64>))]) -> (usize, Vec<f64>) {
    let first = parts.iter().map(|(_, (f, _))| *f).min().unwrap_or(0);
    let end = parts.iter().map(|(_, (f, w))| f + w.len()).max().unwrap_or(first);
    let mut out = vec![0.0; end - first];
    for (k, (f, w)) in parts {
        for (j, v) in w.iter().enumerate() {
            out[f + j - first] += k * v;
        }
    }
    (first, out)
}

/// Integral of the unit tent `max(0, 1 - |u|)` from minus infinity to `u`.
fn tent_cdf(u: f64) -> f64 {
    if u <= -1.0 {
        0.0
    } else if u <= 0.0 {
        (u + 1.0) * (u + 1.0) / 2.0
    } else if u <= 1.0 {
        1.0 - (1.0 - u) * (1.0 - u) / 2.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_output_pixel_keeps_unit_gain() {
        for (source, target) in [(1538, 1440), (2048, 1440), (3008, 1440), (7, 3), (5, 4)] {
            let taps = AxisTaps::new(source, target);
            for i in 0..target {
                let (first, w) = taps.of(i);
                let sum: f32 = w.iter().sum();
                assert!((sum - 1.0).abs() < 1e-4, "{source}->{target} pixel {i}: gain {sum}");
                assert!(first + w.len() <= source, "{source}->{target} pixel {i} reads past the edge");
            }
        }
    }

    #[test]
    fn cached_taps_are_shared_and_match_a_fresh_build() {
        // Retried: parallel tests share the cache and could evict between two lookups.
        let (a, _) = (0..3)
            .map(|_| (AxisTaps::cached(3001, 1437), AxisTaps::cached(3001, 1437)))
            .find(|(a, b)| Arc::ptr_eq(a, b))
            .expect("the second lookup rebuilt the taps");
        let fresh = AxisTaps::new(3001, 1437);
        assert_eq!((a.start.as_slice(), a.weights.as_slice()), (fresh.start.as_slice(), fresh.weights.as_slice()));
        for target in 0..2 * CACHED_AXES {
            AxisTaps::cached(2999, 100 + target);
        }
        let c = AxisTaps::cached(2999, 100);
        assert_eq!((c.start.len(), c.taps), (100, AxisTaps::new(2999, 100).taps), "an evicted entry came back wrong");
    }

    #[test]
    fn sharpening_tapers_off_with_the_ratio() {
        assert_eq!(sharpen_amount(1.068), SHARPEN_MAX);
        assert!(sharpen_amount(1.422) > 0.0 && sharpen_amount(1.422) < SHARPEN_MAX);
        assert_eq!(sharpen_amount(2.089), 0.0);
    }
}
