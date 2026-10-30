//! Structure along the frame's edges that the node grid cannot follow: stray light in the
//! tube, a vignetted falloff, a band of glow a few percent wide. Per edge, a profile across
//! it is measured on what the grid's model left, as the median of five segments along the
//! edge, so it holds only what runs the edge's length; a target there occupies a segment
//! or two and is outvoted. Each segment's profile loses its straight line across the band
//! first: the grid already follows smooth trends, and a big target's tail crossing the band
//! is near straight. 2026-10-24's M57 kept 0.5 sigma of an instrument band without this,
//! 0.4 with it; ring-2 1.2 -> 0.5; an injected edge band 1.2 -> 0.7 (median of 14 stacks),
//! with no target losing light. Catalog footprints and a thin-coverage strip by the edge
//! are left out of the measurement ([`THIN`]), and steps are joined linearly.

use rayon::prelude::*;

use super::grid::{mad_with_scratch, median};
use super::model::BackgroundModel;
use super::protection::ProtectedRegion;
use crate::frame::Frame;
use crate::statistics::MAD_TO_SIGMA;

/// Depth of the band measured from each edge, as a share of the frame across it.
const REACH: f32 = 0.2;

/// Width of one step of the profile, as a share of the frame across it.
const STEP: f32 = 0.005;

/// Segments along each edge whose median makes the profile, over [`SPAN`] of its length.
const SEGMENTS: usize = 5;

/// The share of each edge the segments cover: corners belong to two edges and are left out.
const SPAN: (f32, f32) = (0.1, 0.9);

/// Share of [`REACH`] at full strength; the rest fades to zero so the band leaves no step.
const FULL: f32 = 0.7;

/// Fewest samples behind one segment's step, or that edge gets no profile.
const MIN_SAMPLES: usize = 64;

/// Fewest segments an edge's profile is the median of. A catalog footprint can empty a
/// segment's cells; with fewer left, a target there would no longer be outvoted.
const MIN_SEGMENTS: usize = 3;

/// Samples a cell aims for: its median lands within ~0.08 sigma, the profile (a median of
/// five, smoothed) within ~0.04, where reading every pixel made the estimate 30x the grid's.
const TARGET_SAMPLES: usize = 250;

/// Coarsest stride along the edge, however long it is.
const MAX_ALONG_STRIDE: usize = 8;

/// A profile's standard errors below which it is noise and dropped ([`garrote`]): a stack
/// re-estimates its model every few frames, and noise taken for a band moved its render.
const SIGNIFICANCE: f32 = 3.0;

/// Noise over the band's inner half's above which a step by the edge is a thin-coverage
/// strip (fewer than ~64 % of the subs reached it), measured from past it and left as the
/// grid has it. Ring-2's registration border read 4.5 and 3.0 against 0.87-1.13 for every
/// step of an auto-cropped stack; profiled through, it printed a line 8 levels brighter.
const THIN: f32 = 1.25;

/// Standard error of a median over that of a mean, for Gaussian samples (sqrt(pi / 2)).
const MEDIAN_ERROR: f32 = 1.2533;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

const EDGES: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

/// Per channel, per edge (left, right, top, bottom): the correction by pixel distance from
/// that edge, added to the model's background there.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeProfiles {
    profiles: Vec<[Vec<f32>; 4]>,
}

impl EdgeProfiles {
    /// What `model` left along `frame`'s edges, edge after edge, each measured with the
    /// earlier ones taken off its samples. Only the rows the samples touch are rendered.
    /// Samples inside `protected` (the classifier's catalog footprints) are left out: a
    /// catalogued filament along an edge looks like an instrument band, and it was taken.
    pub fn estimate(frame: &Frame, model: &BackgroundModel, protected: &[ProtectedRegion]) -> Self {
        let (w, h) = (frame.width(), frame.height());
        let plans = EDGES.map(|edge| Plan::new(w, h, edge, protected));
        let mut slot = vec![usize::MAX; h];
        let mut rows = Vec::new();
        for y in plans.iter().flatten().flat_map(Plan::rows) {
            if slot[y] == usize::MAX {
                slot[y] = rows.len();
                rows.push(y);
            }
        }
        let profiles = (0..frame.channels())
            .into_par_iter()
            .map(|c| {
                let background = model.sampler(c, &rows);
                let plane = frame.channel_data(c);
                let residual = |x: usize, y: usize| plane[y * w + x] - background.at(x, slot[y]);
                let mut found: [Vec<f32>; 4] = Default::default();
                for (e, plan) in plans.iter().enumerate() {
                    found[e] = plan.as_ref().map_or_else(Vec::new, |p| p.measure(&residual, &found));
                }
                found
            })
            .collect();
        Self { profiles }
    }

    /// Channel `c`'s top and bottom profiles at row `y` of a frame `height` rows tall: one
    /// value for the whole row.
    pub(super) fn vertical(&self, c: usize, y: usize, height: usize) -> f32 {
        let [_, _, top, bottom] = &self.profiles[c];
        top.get(y).copied().unwrap_or(0.0) + bottom.get(height - 1 - y).copied().unwrap_or(0.0)
    }

    /// Channel `c`'s left and right profiles at column `x` of a frame `width` wide.
    pub(super) fn side(&self, c: usize, x: usize, width: usize) -> f32 {
        let [left, right, _, _] = &self.profiles[c];
        left.get(x).copied().unwrap_or(0.0) + right.get(width - 1 - x).copied().unwrap_or(0.0)
    }

    /// Add channel `c`'s left and right profiles to a row.
    pub(super) fn add_sides(&self, c: usize, row: &mut [f32]) {
        let [left, right, _, _] = &self.profiles[c];
        row.iter_mut().zip(left).for_each(|(v, p)| *v += p);
        row.iter_mut().rev().zip(right).for_each(|(v, p)| *v += p);
    }

    /// The largest correction anywhere, for tests and logs.
    pub fn peak(&self) -> f32 {
        self.profiles.iter().flatten().flatten().fold(0.0f32, |m, v| m.max(v.abs()))
    }
}

/// Where one edge is sampled: steps of depth across it, segments along it, and the strides
/// that keep each cell near [`TARGET_SAMPLES`].
struct Plan {
    edge: Edge,
    w: usize,
    h: usize,
    reach: usize,
    step: usize,
    steps: usize,
    t0: usize,
    length: usize,
    across_stride: usize,
    along_stride: usize,
    /// The footprints reaching the measured band: none, almost always.
    protected: Vec<ProtectedRegion>,
}

impl Plan {
    /// `None` for a frame too small to measure along `edge`.
    fn new(w: usize, h: usize, edge: Edge, protected: &[ProtectedRegion]) -> Option<Self> {
        let (across, along) = match edge {
            Edge::Left | Edge::Right => (w, h),
            Edge::Top | Edge::Bottom => (h, w),
        };
        let reach = (REACH * across as f32) as usize;
        let step = ((STEP * across as f32) as usize).max(2);
        let steps = (3 * reach / 2).div_ceil(step).min(across / 2 / step);
        let t0 = (SPAN.0 * along as f32) as usize;
        let length = ((SPAN.1 * along as f32) as usize - t0) / SEGMENTS;
        let across_stride = if step >= 4 { 2 } else { 1 };
        let per_row = step.div_ceil(across_stride);
        let along_stride = (length * per_row / TARGET_SAMPLES).clamp(1, MAX_ALONG_STRIDE);
        let mut plan = Self { edge, w, h, reach, step, steps, t0, length, across_stride, along_stride, protected: Vec::new() };
        if steps < 4 || (length / along_stride) * per_row < MIN_SAMPLES {
            return None;
        }
        let (a, b) = (plan.pixel(0, t0), plan.pixel(steps * step - 1, t0 + SEGMENTS * length - 1));
        let (x0, y0, x1, y1) = (a.0.min(b.0) as f32, a.1.min(b.1) as f32, a.0.max(b.0) as f32, a.1.max(b.1) as f32);
        plan.protected = protected
            .iter()
            .filter(|r| {
                let (rx0, ry0, rx1, ry1) = r.bounds();
                rx0 <= x1 && rx1 >= x0 && ry0 <= y1 && ry1 >= y0
            })
            .copied()
            .collect();
        Some(plan)
    }

    /// The pixel `(x, y)` at depth `d` from the edge, offset `t` along it.
    fn pixel(&self, d: usize, t: usize) -> (usize, usize) {
        match self.edge {
            Edge::Left => (d, t),
            Edge::Right => (self.w - 1 - d, t),
            Edge::Top => (t, d),
            Edge::Bottom => (t, self.h - 1 - d),
        }
    }

    /// `value` at cell `(segment, k)`'s samples outside every protected footprint, into `out`.
    /// Plain loops into a reused buffer: the iterator chain and a `Vec` per cell were most
    /// of the estimate, 20 threads queueing on the allocator's lock.
    fn gather(&self, segment: usize, k: usize, out: &mut Vec<f32>, value: impl Fn(usize, usize) -> f32) {
        out.clear();
        let start = self.t0 + segment * self.length;
        for t in (start..start + self.length).step_by(self.along_stride) {
            for d in (k * self.step..(k + 1) * self.step).step_by(self.across_stride) {
                let (x, y) = self.pixel(d, t);
                if !self.protected.iter().any(|r| r.contains(x as f32, y as f32)) {
                    out.push(value(x, y));
                }
            }
        }
    }

    /// Every row a sample lies on: along a side edge every cell of a segment shares them,
    /// across a top or bottom edge every segment does.
    fn rows(&self) -> Vec<usize> {
        match self.edge {
            Edge::Left | Edge::Right => (0..SEGMENTS)
                .flat_map(|s| (self.t0 + s * self.length..self.t0 + (s + 1) * self.length).step_by(self.along_stride))
                .collect(),
            // Per step, as [`Self::gather`] walks it: the stride restarts at every step.
            Edge::Top | Edge::Bottom => (0..self.steps)
                .flat_map(|k| (k * self.step..(k + 1) * self.step).step_by(self.across_stride))
                .map(|d| self.pixel(d, 0).1)
                .collect(),
        }
    }

    /// The profile across the edge, one value per pixel of [`REACH`], from `residual` less
    /// the profiles already `found`. Empty when fewer than [`MIN_SEGMENTS`] segments keep
    /// enough sky in every cell.
    fn measure(&self, residual: &(impl Fn(usize, usize) -> f32 + Sync), found: &[Vec<f32>; 4]) -> Vec<f32> {
        let (steps, step, reach) = (self.steps, self.step, self.reach);
        let cells: Vec<Option<(f32, f32)>> = (0..SEGMENTS * steps)
            .into_par_iter()
            .map_init(
                || {
                    // Sized for a whole cell: grown push by push, the reallocations queued
                    // 20 threads on the allocator's lock.
                    let samples = self.length.div_ceil(self.along_stride) * step.div_ceil(self.across_stride);
                    (Vec::with_capacity(samples), Vec::with_capacity(samples))
                },
                |(values, scratch), i| {
                    let value = |x, y| residual(x, y) - taken(found, self.w, self.h, (x, y));
                    self.gather(i / steps, i % steps, values, value);
                    (values.len() >= MIN_SAMPLES).then(|| clipped_median(values, scratch))
                },
            )
            .collect();
        let segments: Vec<Vec<(f32, f32)>> =
            cells.chunks(steps).filter_map(|segment| segment.iter().copied().collect::<Option<Vec<_>>>()).collect();
        if segments.len() < MIN_SEGMENTS {
            return Vec::new();
        }
        let first = thin_steps(&segments);
        if steps - first < 4 {
            return Vec::new();
        }
        let depth: Vec<f32> = (first..steps).map(|k| (k as f32 + 0.5) * step as f32).collect();
        let detrended: Vec<Vec<f32>> = segments
            .iter()
            .map(|segment| without_line(&depth, &segment[first..].iter().map(|c| c.0).collect::<Vec<_>>()))
            .collect();
        let combined: Vec<f32> = (0..depth.len())
            .map(|k| median(&mut detrended.iter().map(|p| p[k]).collect::<Vec<_>>()))
            .collect();
        // A median of five cells, then of three steps: about half a cell's error.
        let mut errors: Vec<f32> = segments.iter().flat_map(|s| &s[first..]).map(|c| c.1).collect();
        let threshold = SIGNIFICANCE * 0.5 * median(&mut errors);
        let n = combined.len();
        let smoothed: Vec<f32> = std::iter::repeat_n(0.0, first)
            .chain((0..n).map(|k| garrote(median(&mut combined[k.saturating_sub(1)..(k + 2).min(n)].to_vec()), threshold)))
            .collect();
        let full = FULL * reach as f32;
        (0..reach)
            .map(|d| {
                let fade = ((d as f32 - full) / (reach as f32 - full)).clamp(0.0, 1.0);
                between_steps(&smoothed, step, d) * (fade * std::f32::consts::FRAC_PI_2).cos().powi(2)
            })
            .collect()
    }
}

/// How many steps from the edge are a thin-coverage strip ([`THIN`]): the leading run whose
/// noise, the median over `segments` of each cell's error, exceeds the inner half's.
fn thin_steps(segments: &[Vec<(f32, f32)>]) -> usize {
    let steps = segments[0].len();
    let noise: Vec<f32> = (0..steps).map(|k| median(&mut segments.iter().map(|s| s[k].1).collect::<Vec<_>>())).collect();
    let reference = median(&mut noise[steps / 2..].to_vec());
    noise.iter().take_while(|&&n| n > THIN * reference).count()
}

/// `smoothed` (one value per step of `step` pixels) at pixel depth `d`, joined linearly
/// between step centres: held whole, each step printed a line the edge's length where the
/// band only sloped (2.1 sigma between neighbouring pixels, 7-13 px apart, on ring-2).
fn between_steps(smoothed: &[f32], step: usize, d: usize) -> f32 {
    let u = ((d as f32 + 0.5) / step as f32 - 0.5).clamp(0.0, (smoothed.len() - 1) as f32);
    let k = (u as usize).min(smoothed.len().saturating_sub(2));
    let t = u - k as f32;
    match smoothed.get(k + 1) {
        Some(&next) => smoothed[k] + (next - smoothed[k]) * t,
        None => smoothed[k],
    }
}

/// The profiles in `found` at `(x, y)`: what an earlier edge already took there.
fn taken(found: &[Vec<f32>; 4], w: usize, h: usize, (x, y): (usize, usize)) -> f32 {
    let [left, right, top, bottom] = found;
    let at = |p: &Vec<f32>, d: usize| p.get(d).copied().unwrap_or(0.0);
    at(left, x) + at(right, w - 1 - x) + at(top, y) + at(bottom, h - 1 - y)
}

/// `profile` less its straight line in `depth`, fitted to its best half (least trimmed
/// squares): a band can fill a third of the range, and a least-squares line through it left
/// the profile 3 sigma negative at the edge, adding light there.
fn without_line(depth: &[f32], profile: &[f32]) -> Vec<f32> {
    let n = depth.len();
    let third = n / 3;
    let ends = |range: std::ops::Range<usize>| {
        let mut v = profile[range.clone()].to_vec();
        ((range.start + range.end) as f32 / 2.0, median(&mut v))
    };
    let ((x0, y0), (x1, y1)) = (ends(0..third.max(1)), ends(n - third.max(1)..n));
    let through_ends = (y0 - x0 * (y1 - y0) / (x1 - x0), (y1 - y0) / (x1 - x0));
    let all = vec![true; n];
    let starts = [fit_line(depth, profile, &all), through_ends, (median(&mut profile.to_vec()), 0.0)];
    let line = starts
        .into_iter()
        .map(|start| trimmed_line(depth, profile, start))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or((0.0, 0.0), |(line, _)| line);
    depth.iter().zip(profile).map(|(&x, &y)| y - line.0 - line.1 * x).collect()
}

/// Concentration steps from `line`: refit to the half of the points closest to it until
/// that half stops changing. Returns the line and its trimmed sum of squares.
fn trimmed_line(x: &[f32], y: &[f32], mut line: (f32, f32)) -> ((f32, f32), f32) {
    let half = x.len().div_ceil(2);
    let mut keep = vec![false; x.len()];
    let mut cost = f32::MAX;
    for _ in 0..20 {
        let mut order: Vec<(f32, usize)> = (0..x.len()).map(|i| ((y[i] - line.0 - line.1 * x[i]).powi(2), i)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut next = vec![false; x.len()];
        order[..half].iter().for_each(|&(_, i)| next[i] = true);
        cost = order[..half].iter().map(|&(r, _)| r).sum();
        if next == keep {
            break;
        }
        keep = next;
        line = fit_line(x, y, &keep);
    }
    (line, cost)
}

/// Least-squares `(intercept, slope)` through the `keep` points.
fn fit_line(x: &[f32], y: &[f32], keep: &[bool]) -> (f32, f32) {
    let points: Vec<(f32, f32)> = (0..x.len()).filter(|&i| keep[i]).map(|i| (x[i], y[i])).collect();
    let n = points.len() as f32;
    let (mx, my) = points.iter().fold((0.0, 0.0), |(a, b), &(x, y)| (a + x / n, b + y / n));
    let (sxy, sxx) = points
        .iter()
        .fold((0.0, 0.0), |(sxy, sxx), &(x, y)| (sxy + (x - mx) * (y - my), sxx + (x - mx) * (x - mx)));
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (my - slope * mx, slope)
}

/// Median after two rounds of 3-sigma clipping, so stars in the strip do not count.
/// `(median, its standard error)` of what survives.
fn clipped_median(values: &mut Vec<f32>, scratch: &mut Vec<f32>) -> (f32, f32) {
    let mut centre = median(values);
    let mut sigma = 0.0;
    for _ in 0..2 {
        sigma = mad_with_scratch(values, centre, scratch) * MAD_TO_SIGMA;
        let before = values.len();
        values.retain(|&v| (v - centre).abs() <= 3.0 * sigma);
        if values.is_empty() {
            return (centre, sigma);
        }
        centre = median(values);
        if values.len() == before {
            break;
        }
    }
    (centre, MEDIAN_ERROR * sigma / (values.len() as f32).sqrt())
}

/// `value` shrunk by `t^2 / value` beyond `t`, zero within: noise is dropped and a real
/// band kept nearly whole, with no step where the two meet.
fn garrote(value: f32, t: f32) -> f32 {
    if value.abs() <= t {
        0.0
    } else {
        value - t * t / value
    }
}

#[cfg(test)]
mod tests {
    include!("edges_tests.rs");
}
