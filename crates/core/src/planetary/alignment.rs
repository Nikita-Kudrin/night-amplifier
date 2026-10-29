//! Frame alignment using cross-correlation for planetary stacking.
//!
//! Exhaustive NCC is `(2r+1)^2` window passes: radius 50 over the default 1504 px window of
//! a 3008 px frame was ~10^4 passes of 2.3 MP, 134 s a frame on an Orange Pi 5 (2026-10-24's
//! Moon), and Stop waited for every queued frame. The search now runs exhaustively only on
//! a 2^k-binned level of a [`LuminancePyramid`] and climbs from the winner at each finer
//! level to a 3x3 local maximum, whose neighbours the subpixel fit reuses. The reference's
//! windows are built once per reference ([`AlignmentReference`]).

use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use tracing::instrument;

use crate::frame::Frame;

use super::config::AlignmentRoi;
use super::quality::frame_to_luminance;

/// The coarsest level keeps at least this many pixels along the window's short side.
const MIN_COARSE_ROI_SIDE: usize = 96;
/// A level is searched exhaustively only while the radius still spans this much on it.
const MIN_COARSE_RADIUS: i32 = 2;
/// Re-centrings a climb may take before its best counts as the search's edge: from the
/// doubled coarser winner the peak is a step away, rarely two.
const MAX_CLIMB_STEPS: usize = 4;
/// Enough for radius 50 (`50 >> 4` is still a search); a coarser level could not refine.
const MAX_LEVELS: usize = 4;
/// Below this NCC the window matched nothing.
const MIN_CORRELATION: f32 = 0.3;
/// How far the window keeps from the frame's edges, at most an eighth of the side.
const EDGE_MARGIN: usize = 16;
/// A window flatter than this has nothing to correlate.
const FLAT_STD: f32 = 1e-6;

/// A frame's luminance and its 2x2-binned levels, built once per frame and shared by every
/// alignment against it.
pub struct LuminancePyramid {
    /// `levels[0]` is full resolution; each next one halves both sides.
    levels: Vec<Level>,
}

struct Level {
    lum: Vec<f32>,
    width: usize,
    height: usize,
}

impl LuminancePyramid {
    pub fn of(frame: &Frame) -> Self {
        Self::from_luminance(frame_to_luminance(frame), frame.width(), frame.height())
    }

    pub fn from_luminance(lum: Vec<f32>, width: usize, height: usize) -> Self {
        let mut levels = vec![Level { lum, width, height }];
        while levels.len() <= MAX_LEVELS {
            let last = &levels[levels.len() - 1];
            if last.width.min(last.height) / 2 < MIN_COARSE_ROI_SIDE {
                break;
            }
            let next = last.binned();
            levels.push(next);
        }
        Self { levels }
    }

    pub fn luminance(&self) -> &[f32] {
        &self.levels[0].lum
    }

    pub fn width(&self) -> usize {
        self.levels[0].width
    }

    pub fn height(&self) -> usize {
        self.levels[0].height
    }

    #[cfg(test)]
    pub(super) fn level_sizes(&self) -> Vec<(usize, usize)> {
        self.levels.iter().map(|level| (level.width, level.height)).collect()
    }

    #[cfg(test)]
    pub(super) fn level(&self, index: usize) -> &[f32] {
        &self.levels[index].lum
    }
}

impl Level {
    /// 2x2 box average; an odd last row or column is dropped.
    fn binned(&self) -> Self {
        let (out_w, out_h) = (self.width / 2, self.height / 2);
        let mut out = vec![0.0f32; out_w * out_h];
        out.par_chunks_mut(out_w).enumerate().for_each(|(y, row)| {
            let top = &self.lum[2 * y * self.width..];
            let bottom = &self.lum[(2 * y + 1) * self.width..];
            for (x, value) in row.iter_mut().enumerate() {
                *value = 0.25 * (top[2 * x] + top[2 * x + 1] + bottom[2 * x] + bottom[2 * x + 1]);
            }
        });
        Self { lum: out, width: out_w, height: out_h }
    }
}

/// Where to look: shifts within `radius` of `expected`, in [`Alignment`]'s convention.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Search {
    pub expected: (f32, f32),
    pub radius: usize,
}

impl Search {
    pub fn within(radius: usize) -> Self {
        Self { expected: (0.0, 0.0), radius }
    }

    /// The window's own offsets: the shift a frame shows is the negated `(dx, dy)`.
    fn centre(&self) -> (i32, i32) {
        (-self.expected.0.round() as i32, -self.expected.1.round() as i32)
    }
}

/// The translation that moves `frame` onto the reference (`dx = reference - frame`), the
/// NCC it was found at, and what it rests on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alignment {
    pub dx: f32,
    pub dy: f32,
    pub ncc: f32,
    pub fit: Fit,
}

/// What an [`Alignment`] rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The best correlation lies inside the search, refined to subpixel when asked.
    Correlated,
    /// The best correlation lies on the search's edge or the frame's, or the climb to it
    /// ran out of steps, so the true one may lie beyond it:
    /// 2026-10-24's Moon drifted 230 px, and 77 % of its frames sat on the edge of a ±50 px
    /// search at NCC 0.66-0.96, every one a wrong shift.
    SearchEdge,
    /// The reference window is flat, so there is nothing to correlate: the frames'
    /// centroids answered.
    Featureless,
    /// Nothing in the search correlated above 0.3: the shift is only the search's best
    /// guess, at the NCC it reached (a frame moved 260 px, past what the frame holds, was
    /// once stacked at its centroids' shift of 0).
    Uncorrelated,
}

/// The reference's window on every pyramid level a search can use, built once per
/// reference and window: rebuilt for every frame, the 1504 px one was a quarter of a
/// search's time, all of it on one thread.
pub struct AlignmentReference {
    /// `levels[k]` is the window on pyramid level `k`, while it keeps `MIN_COARSE_ROI_SIDE`.
    levels: Vec<ReferenceWindow>,
    /// The window on level 0: inside the frame and clear of its edges.
    window: AlignmentRoi,
    /// The reference's centroid, kept only where the window is flat and centroids answer.
    centroid: Option<(f32, f32)>,
}

impl AlignmentReference {
    pub fn new(reference: &LuminancePyramid, roi: &AlignmentRoi) -> Self {
        let (width, height) = (reference.width(), reference.height());
        let (x, y, w, h) = window_in_frame(roi, width, height);
        let levels: Vec<ReferenceWindow> = reference
            .levels
            .iter()
            .enumerate()
            .take_while(|&(k, _)| k == 0 || w.min(h) >> k >= MIN_COARSE_ROI_SIDE)
            .map(|(k, level)| ReferenceWindow::new(level, (x >> k, y >> k, w >> k, h >> k)))
            .collect();
        let centroid = (levels[0].std < FLAT_STD).then(|| compute_centroid(reference.luminance(), width, height));
        Self { levels, window: AlignmentRoi::new(x, y, w, h), centroid }
    }

    /// The window the reference correlates, in level-0 pixels.
    pub fn window(&self) -> AlignmentRoi {
        self.window
    }
}

/// Aligns `frame` to the reference by the NCC of the reference's window against shifted
/// windows of the frame, refined to subpixel when `subpixel_factor > 1`.
#[instrument(skip_all, fields(radius = search.radius, subpixel_factor = subpixel_factor))]
pub fn align(
    reference: &AlignmentReference,
    frame: &LuminancePyramid,
    search: Search,
    subpixel_factor: usize,
) -> Alignment {
    align_counted(reference, frame, search, subpixel_factor, &AtomicU64::new(0))
}

/// [`align`], adding every window pixel it compared to `compared`.
pub(super) fn align_counted(
    reference: &AlignmentReference,
    frame: &LuminancePyramid,
    search: Search,
    subpixel_factor: usize,
    compared: &AtomicU64,
) -> Alignment {
    if let Some((rx, ry)) = reference.centroid {
        let (fx, fy) = compute_centroid(frame.luminance(), frame.width(), frame.height());
        return Alignment { dx: rx - fx, dy: ry - fy, ncc: 0.0, fit: Fit::Featureless };
    }
    let full = RoiMatcher { frame: &frame.levels[0], reference: &reference.levels[0], compared };
    let (centre, radius) = (search.centre(), search.radius as i32);
    let Some(peak) = coarse_to_fine(reference, frame, &full, centre, radius) else {
        return Alignment { dx: -(centre.0 as f32), dy: -(centre.1 as f32), ncc: 0.0, fit: Fit::Uncorrelated };
    };
    let (x, y, ncc) = (peak.x, peak.y, peak.ncc.max(0.0));
    if ncc < MIN_CORRELATION {
        return Alignment { dx: -(x as f32), dy: -(y as f32), ncc, fit: Fit::Uncorrelated };
    }

    let neighbours = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let inside = peak.summit
        && (x - centre.0).abs() < radius
        && (y - centre.1).abs() < radius
        && neighbours.iter().all(|&(nx, ny)| full.fits(x + nx, y + ny));
    let (sub_x, sub_y) = if subpixel_factor > 1 && inside { peak.subpixel() } else { (0.0, 0.0) };
    let fit = if inside { Fit::Correlated } else { Fit::SearchEdge };
    Alignment { dx: -(x as f32 + sub_x), dy: -(y as f32 + sub_y), ncc, fit }
}

/// One climb on level 0 from the window offset `start`, within `start ± radius`:
/// `(x, y, summit)`.
#[cfg(test)]
pub(super) fn climb_from(
    reference: &AlignmentReference,
    frame: &LuminancePyramid,
    start: (i32, i32),
    radius: i32,
) -> Option<(i32, i32, bool)> {
    let compared = AtomicU64::new(0);
    let matcher = RoiMatcher { frame: &frame.levels[0], reference: &reference.levels[0], compared: &compared };
    matcher.climb(start, SearchBox::around(start, radius)).map(|peak| (peak.x, peak.y, peak.summit))
}

/// The best whole-pixel window offset in `centre ± radius`: searched exhaustively on the
/// coarsest level the window and radius allow, then climbed to level by level. `None`
/// where no window in reach lies inside the frame.
fn coarse_to_fine(
    reference: &AlignmentReference,
    frame: &LuminancePyramid,
    full: &RoiMatcher<'_>,
    centre: (i32, i32),
    radius: i32,
) -> Option<Peak> {
    let bounds = SearchBox::around(centre, radius);
    let available = reference.levels.len().min(frame.levels.len()) - 1;
    let coarsest = (1..=available).take_while(|&k| radius >> k >= MIN_COARSE_RADIUS).last().unwrap_or(0);
    if coarsest == 0 {
        return full.exhaustive(bounds);
    }
    let on_level = |k: usize| RoiMatcher { frame: &frame.levels[k], reference: &reference.levels[k], compared: full.compared };
    let coarse = on_level(coarsest).exhaustive(bounds.at_scale(coarsest))?;
    let (mut x, mut y) = (coarse.x, coarse.y);
    for k in (1..coarsest).rev() {
        let peak = on_level(k).climb((x * 2, y * 2), bounds.at_scale(k))?;
        (x, y) = (peak.x, peak.y);
    }
    full.climb((x * 2, y * 2), bounds)
}

/// The best offset a scan found, and the scan, whose 3x3 around it the subpixel fit reads.
struct Peak {
    x: i32,
    y: i32,
    ncc: f32,
    scan: Scan,
    /// Whether no offset of its 3x3 inside the search beats it: false when a climb ran out
    /// of steps still rising.
    summit: bool,
}

impl Peak {
    /// Parabolic interpolation through the peak and its four neighbours.
    fn subpixel(&self) -> (f32, f32) {
        let at = |dx: i32, dy: i32| self.scan.at(self.x + dx, self.y + dy).unwrap_or(0.0);
        let sub_x = parabolic_fit(at(-1, 0), self.ncc, at(1, 0));
        let sub_y = parabolic_fit(at(0, -1), self.ncc, at(0, 1));
        (sub_x.clamp(-0.5, 0.5), sub_y.clamp(-0.5, 0.5))
    }
}

/// The NCCs compared so far over a box, row-major: a climb re-centring on a neighbour asks
/// for most of its last 3x3 again.
struct Scan {
    bounds: SearchBox,
    /// `None` off the frame, on a flat window, or not compared yet.
    ncc: Vec<Option<f32>>,
    compared: Vec<bool>,
}

impl Scan {
    fn over(bounds: SearchBox) -> Self {
        Self { bounds, ncc: vec![None; bounds.len()], compared: vec![false; bounds.len()] }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        let SearchBox { x: (x0, x1), y: (y0, y1) } = self.bounds;
        (x0..=x1).contains(&x).then_some(())?;
        (y0..=y1).contains(&y).then_some(())?;
        Some(((y - y0) * (x1 - x0 + 1) + (x - x0)) as usize)
    }

    /// Compares every offset of `part` (inside `bounds`) not compared yet, in parallel.
    fn fill(&mut self, matcher: &RoiMatcher<'_>, part: SearchBox) {
        let todo: Vec<(usize, (i32, i32))> = part
            .offsets()
            .into_iter()
            .filter_map(|(x, y)| self.index(x, y).filter(|&i| !self.compared[i]).map(|i| (i, (x, y))))
            .collect();
        let found: Vec<Option<f32>> = todo.par_iter().map(|&(_, (x, y))| matcher.ncc_at(x, y)).collect();
        for (&(i, _), ncc) in todo.iter().zip(found) {
            (self.ncc[i], self.compared[i]) = (ncc, true);
        }
    }

    fn at(&self, x: i32, y: i32) -> Option<f32> {
        self.ncc[self.index(x, y)?]
    }

    /// The best offset of `part`; ties go to the first in row-major order, as a sequential
    /// scan finds it, so the answer does not depend on the thread count.
    fn best_in(&self, part: SearchBox) -> Option<(i32, i32, f32)> {
        let mut best: Option<(i32, i32, f32)> = None;
        for (x, y) in part.offsets() {
            if let Some(ncc) = self.at(x, y) {
                if best.is_none_or(|(_, _, top)| ncc > top) {
                    best = Some((x, y, ncc));
                }
            }
        }
        best
    }
}

/// Inclusive window offsets `[x0, x1] x [y0, y1]`; empty where `x0 > x1` or `y0 > y1`.
#[derive(Clone, Copy)]
struct SearchBox {
    x: (i32, i32),
    y: (i32, i32),
}

impl SearchBox {
    fn around((cx, cy): (i32, i32), radius: i32) -> Self {
        Self { x: (cx - radius, cx + radius), y: (cy - radius, cy + radius) }
    }

    /// The box in a level binned `scale` times; floor division, so it never shrinks.
    fn at_scale(self, scale: usize) -> Self {
        let shrink = |(lo, hi): (i32, i32)| (lo >> scale, (hi + (1 << scale) - 1) >> scale);
        Self { x: shrink(self.x), y: shrink(self.y) }
    }

    fn within(self, outer: Self) -> Self {
        let clip = |(lo, hi): (i32, i32), (olo, ohi): (i32, i32)| (lo.max(olo), hi.min(ohi));
        Self { x: clip(self.x, outer.x), y: clip(self.y, outer.y) }
    }

    fn offsets(self) -> Vec<(i32, i32)> {
        let xs = self.x.0..=self.x.1;
        (self.y.0..=self.y.1).flat_map(|dy| xs.clone().map(move |dx| (dx, dy))).collect()
    }

    fn len(self) -> usize {
        let span = |(lo, hi): (i32, i32)| usize::try_from(hi - lo + 1).unwrap_or(0);
        span(self.x) * span(self.y)
    }
}

/// Computes the weighted centroid of luminance data.
pub fn compute_centroid(lum: &[f32], width: usize, height: usize) -> (f32, f32) {
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_weight = 0.0f64;

    // Use a simple threshold to focus on the object and ignore background noise
    let threshold = 0.1f32;

    for y in 0..height {
        for x in 0..width {
            let val = lum[y * width + x];
            if val > threshold {
                let w = (val - threshold) as f64;
                sum_x += x as f64 * w;
                sum_y += y as f64 * w;
                sum_weight += w;
            }
        }
    }

    if sum_weight > 1e-6 {
        ((sum_x / sum_weight) as f32, (sum_y / sum_weight) as f32)
    } else {
        (width as f32 / 2.0, height as f32 / 2.0)
    }
}

/// The window `(x, y, w, h)` inside the frame and `EDGE_MARGIN` clear of its edges, shrunk
/// rather than moved so it stays on the target. Clamped against an edge, a frame that had
/// not moved peaked where its outward neighbour lay off the frame, so it read as a match
/// on the frame's edge: a planet off to one side lost every frame that had not moved in.
fn window_in_frame(roi: &AlignmentRoi, width: usize, height: usize) -> (usize, usize, usize, usize) {
    let span = |start: usize, len: usize, side: usize| {
        let margin = EDGE_MARGIN.min(side / 8);
        let lo = start.clamp(margin, side - margin - 1);
        let hi = start.saturating_add(len).clamp(lo + 1, side - margin);
        (lo, hi - lo)
    };
    let (x, w) = span(roi.x, roi.width, width);
    let (y, h) = span(roi.y, roi.height, height);
    (x, y, w, h)
}

/// The reference window with its mean taken out, so the cross term needs no frame mean.
/// Statistics are f64: an f32 sum over a 1504 px window drifts in the fourth digit.
struct ReferenceWindow {
    /// Taken out of the frame's samples too before they are summed: on a 0.9 mean with
    /// 0.07 % texture, one-pass f32 `E[x²] - E[x]²` placed a frame 3 px off at NCC 0.99.
    mean: f32,
    zero_mean: Vec<f32>,
    /// What f32 rounding left of the zero-mean window's sum, taken out of each cross term.
    residual_sum: f64,
    std: f32,
    /// `(x, y, w, h)` in its level, inside the frame.
    roi: (usize, usize, usize, usize),
}

impl ReferenceWindow {
    fn new(level: &Level, roi: (usize, usize, usize, usize)) -> Self {
        let (x, y, w, h) = roi;
        let window = extract_roi(&level.lum, level.width, x, y, w, h);
        let n = window.len().max(1) as f64;
        let mean = window.iter().map(|&v| f64::from(v)).sum::<f64>() / n;
        let variance = window.iter().map(|&v| (f64::from(v) - mean).powi(2)).sum::<f64>() / n;
        let zero_mean: Vec<f32> = window.iter().map(|&v| (f64::from(v) - mean) as f32).collect();
        let residual_sum = zero_mean.iter().map(|&v| f64::from(v)).sum();
        Self { mean: mean as f32, zero_mean, residual_sum, std: variance.sqrt() as f32, roi }
    }
}

/// The reference window, correlated against shifted windows of one frame's level.
struct RoiMatcher<'a> {
    frame: &'a Level,
    reference: &'a ReferenceWindow,
    compared: &'a AtomicU64,
}

impl RoiMatcher<'_> {
    /// Where the window shifted by `(dx, dy)` starts, if all of it lies inside the frame.
    fn origin(&self, dx: i32, dy: i32) -> Option<(usize, usize)> {
        let (roi_x, roi_y, roi_w, roi_h) = self.reference.roi;
        let shifted_x = usize::try_from(roi_x as i64 + i64::from(dx)).ok()?;
        let shifted_y = usize::try_from(roi_y as i64 + i64::from(dy)).ok()?;
        (shifted_x + roi_w <= self.frame.width && shifted_y + roi_h <= self.frame.height).then_some((shifted_x, shifted_y))
    }

    /// Whether the frame holds the whole window shifted by `(dx, dy)`: a best match whose
    /// neighbour it does not hold may have its true peak past the frame's edge.
    fn fits(&self, dx: i32, dy: i32) -> bool {
        self.origin(dx, dy).is_some()
    }

    /// NCC against the window shifted by `(dx, dy)`; `None` off the frame or on a flat window.
    fn ncc_at(&self, dx: i32, dy: i32) -> Option<f32> {
        let (_, _, roi_w, roi_h) = self.reference.roi;
        let (shifted_x, shifted_y) = self.origin(dx, dy)?;
        self.compared.fetch_add((roi_w * roi_h) as u64, Ordering::Relaxed);
        // One pass over samples less the reference's mean: sum, sum of squares and the cross
        // term, per row in f32 and across rows in f64, so a 1504 px window keeps its precision.
        let offset = self.reference.mean;
        let (mut sum, mut sum_sq, mut cross) = (0.0f64, 0.0f64, 0.0f64);
        for ry in 0..roi_h {
            let start = (shifted_y + ry) * self.frame.width + shifted_x;
            let frame_row = &self.frame.lum[start..start + roi_w];
            let ref_row = &self.reference.zero_mean[ry * roi_w..(ry + 1) * roi_w];
            let (mut s, mut sq, mut c) = (0.0f32, 0.0f32, 0.0f32);
            for (&f, &r) in frame_row.iter().zip(ref_row) {
                let f = f - offset;
                s += f;
                sq += f * f;
                c += f * r;
            }
            sum += f64::from(s);
            sum_sq += f64::from(sq);
            cross += f64::from(c);
        }
        let n = (roi_w * roi_h) as f64;
        let mean = sum / n;
        let frame_std = (sum_sq / n - mean * mean).max(0.0).sqrt();
        if frame_std < f64::from(FLAT_STD) {
            return None;
        }
        let cross = cross - mean * self.reference.residual_sum;
        Some((cross / (n * f64::from(self.reference.std) * frame_std)) as f32)
    }

    /// Every offset in `bounds`; `None` where none lies inside the frame.
    fn exhaustive(&self, bounds: SearchBox) -> Option<Peak> {
        let mut scan = Scan::over(bounds);
        scan.fill(self, bounds);
        let (x, y, ncc) = scan.best_in(bounds)?;
        Some(Peak { x, y, ncc, scan, summit: true })
    }

    /// From `start` to an offset no neighbour in `bounds` beats, re-centring on the best of
    /// each 3x3 up to `MAX_CLIMB_STEPS` times. A fixed ±2 box around `start` handed back a
    /// point on its own edge as the peak. `None` where nothing in reach lies inside the frame.
    fn climb(&self, start: (i32, i32), bounds: SearchBox) -> Option<Peak> {
        let reach = MAX_CLIMB_STEPS as i32 + 1;
        let mut scan = Scan::over(SearchBox::around(start, reach).within(bounds));
        let (mut centre, mut steps) = (start, 0);
        loop {
            let around = SearchBox::around(centre, 1).within(bounds);
            scan.fill(self, around);
            let (x, y, ncc) = scan.best_in(around)?;
            let summit = (x, y) == centre;
            if summit || steps == MAX_CLIMB_STEPS {
                return Some(Peak { x, y, ncc, scan, summit });
            }
            (centre, steps) = ((x, y), steps + 1);
        }
    }
}

/// Extracts a region of interest from luminance data.
fn extract_roi(
    lum: &[f32],
    width: usize,
    x: usize,
    y: usize,
    roi_w: usize,
    roi_h: usize,
) -> Vec<f32> {
    let mut roi = Vec::with_capacity(roi_w * roi_h);
    for ry in 0..roi_h {
        roi.extend_from_slice(&lum[(y + ry) * width + x..(y + ry) * width + x + roi_w]);
    }
    roi
}

/// Fits a parabola to three samples and returns the subpixel offset.
fn parabolic_fit(c0: f32, c1: f32, c2: f32) -> f32 {
    let denom = c0 + c2 - 2.0 * c1;
    if denom.abs() > 1e-6 {
        (c0 - c2) / (2.0 * denom)
    } else {
        0.0
    }
}
