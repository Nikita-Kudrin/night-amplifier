//! Automatic crop of a stack's thinly covered border: drift, field rotation, bumps.
//!
//! Decided on the coverage grid the stack already carries ([`NoiseField::coverage`],
//! `offered / N` per [`NOISE_REDUCTION`] px cell), so it follows what actually landed rather
//! than a model of the mount: the subs' common footprint (every cell at 100 %) kept 56 % of a
//! synthetic frame after one bumped sub and 30 % after three, where [`PICK_COVERAGE`] ignores
//! them. [`CropTracker`] adds hysteresis, since under drift the best rectangle moves on nearly
//! every sub (51 of 64 on a real session) and the live view would creep.

use crate::error::{Result, StackError};
use crate::frame::{NoiseField, NOISE_REDUCTION};

/// A new rectangle has every cell at this share of the stack or more.
pub const PICK_COVERAGE: f32 = 0.95;
/// The shown rectangle stays while every cell keeps this share: edge noise ≤ `1/sqrt(0.85)`, +8.5 %.
/// With [`PICK_COVERAGE`] it cut a drifting session's changes 51 → 17 for < 1 % of the area.
pub const KEEP_COVERAGE: f32 = 0.85;
/// A fresh pick this much larger replaces the shown one, or an early small crop never grows back.
pub const GROW_SHARE: f64 = 1.05;
/// A covered area below this share of the frame is not cropped to: the stack has moved on,
/// which is Wanderer's or the observer's call, not a crop's.
pub const MIN_CROP_SHARE: f64 = 0.10;
/// Nor below this many pixels a side, or half the image's side where that is less.
pub const MIN_CROP_SIDE: usize = 64;
/// No crop before this depth: the first at which [`PICK_COVERAGE`] tolerates one missing sub
/// (19/20). Shallower, one bumped sub cut the view to the common footprint until depth 20
/// (14 subs at 56 % from a bump at sub 5); waiting also cut a real session's changes 14 -> 5.
pub const MIN_CROP_DEPTH: usize = (1.0 / (1.0 - PICK_COVERAGE as f64) + 0.5) as usize;

/// Coverage values are ratios of small integers; this keeps one landing exactly on a threshold
/// inside it whichever way it rounded.
const THRESHOLD_SLACK: f32 = 1e-6;

/// A crop of an image, in its pixels.
///
/// From [`largest_covered`] the origin is on the coverage grid and the size a multiple of it,
/// except where the rectangle reaches the image's right or bottom edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl CropRect {
    pub fn area(&self) -> usize {
        self.width * self.height
    }

    /// Whether this is the whole of an image of `width x height`.
    pub fn is_full(&self, width: usize, height: usize) -> bool {
        self.x == 0 && self.y == 0 && self.width == width && self.height == height
    }

    /// Whether it lies inside an image of `width x height` and holds at least one pixel.
    pub fn fits(&self, width: usize, height: usize) -> bool {
        self.width > 0
            && self.height > 0
            && self.x + self.width <= width
            && self.y + self.height <= height
    }

    /// Refuses a rectangle that does not fit an image of `width x height`.
    pub fn check(&self, width: usize, height: usize) -> Result<()> {
        if self.fits(width, height) {
            return Ok(());
        }
        Err(StackError::InvalidConfiguration(format!(
            "crop {}x{} at ({}, {}) does not fit a {width}x{height} image",
            self.width, self.height, self.x, self.y
        )))
    }

    /// The coverage cells it spans, `(x0, y0, x1, y1)` exclusive.
    fn cells(&self) -> (usize, usize, usize, usize) {
        (
            self.x / NOISE_REDUCTION,
            self.y / NOISE_REDUCTION,
            (self.x + self.width).div_ceil(NOISE_REDUCTION),
            (self.y + self.height).div_ceil(NOISE_REDUCTION),
        )
    }

    /// The pixels cells `(x0, y0, x1, y1)` cover in an image of `width x height`.
    fn of_cells((x0, y0, x1, y1): (usize, usize, usize, usize), width: usize, height: usize) -> Self {
        let (px0, py0) = (x0 * NOISE_REDUCTION, y0 * NOISE_REDUCTION);
        Self {
            x: px0,
            y: py0,
            width: (x1 * NOISE_REDUCTION).min(width) - px0,
            height: (y1 * NOISE_REDUCTION).min(height) - py0,
        }
    }
}

/// The largest rectangle whose every coverage cell holds `threshold` of the stack or more,
/// in the field's source pixels; possibly the whole image. `None` when no cell does.
///
/// Largest rectangle under a histogram, row by row: O(cells), ~141k at 3008².
pub fn largest_covered(field: &NoiseField, threshold: f32) -> Option<CropRect> {
    let (w, h) = (field.width(), field.height());
    let coverage = field.coverage();
    let mut heights = vec![0usize; w];
    let mut stack: Vec<usize> = Vec::with_capacity(w + 1);
    let mut best: Option<(usize, (usize, usize, usize, usize))> = None;
    for y in 0..h {
        for (x, run) in heights.iter_mut().enumerate() {
            *run = if coverage[y * w + x] >= threshold - THRESHOLD_SLACK { *run + 1 } else { 0 };
        }
        stack.clear();
        for x in 0..=w {
            let here = heights.get(x).copied().unwrap_or(0);
            while let Some(&top) = stack.last() {
                if heights[top] <= here {
                    break;
                }
                stack.pop();
                let run = heights[top];
                let left = stack.last().map_or(0, |&l| l + 1);
                let area = run * (x - left);
                if best.is_none_or(|(a, _)| area > a) {
                    best = Some((area, (left, y + 1 - run, x, y + 1)));
                }
            }
            stack.push(x);
        }
    }
    best.map(|(_, cells)| CropRect::of_cells(cells, field.source_width(), field.source_height()))
}

/// The lowest coverage cell `rect` spans, or `None` when it does not fit the field's image.
pub fn min_coverage(field: &NoiseField, rect: &CropRect) -> Option<f32> {
    if !rect.fits(field.source_width(), field.source_height()) {
        return None;
    }
    let (x0, y0, x1, y1) = rect.cells();
    let w = field.width();
    let coverage = field.coverage();
    let lowest = (y0..y1.min(field.height()))
        .flat_map(|y| coverage[y * w + x0..y * w + x1.min(w)].iter().copied())
        .fold(1.0f32, f32::min);
    Some(lowest)
}

/// What a [`CropTracker`] decided for one snapshot of the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropDecision {
    /// Show the whole image: every part of it is covered well enough.
    Uncropped,
    Cropped(CropRect),
    /// The well-covered area is below [`MIN_CROP_SHARE`] / [`MIN_CROP_SIDE`]: shown whole.
    TooThin,
}

impl CropDecision {
    pub fn rect(&self) -> Option<CropRect> {
        match self {
            Self::Cropped(rect) => Some(*rect),
            Self::Uncropped | Self::TooThin => None,
        }
    }
}

/// The crop one stack shows, with hysteresis: a rectangle is picked at [`PICK_COVERAGE`] and
/// kept while every cell stays at [`KEEP_COVERAGE`], unless a fresh pick is [`GROW_SHARE`]
/// larger or covers the whole image.
///
/// Resets itself when the depth goes down, which every stack restart does; a rectangle kept
/// across an unnoticed restart is still only kept if the new stack's coverage clears it.
#[derive(Debug, Default, Clone)]
pub struct CropTracker {
    shown: Option<CropRect>,
    depth: usize,
}

impl CropTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The rectangle last decided on, if the image was cropped.
    pub fn shown(&self) -> Option<CropRect> {
        self.shown
    }

    /// Decide the crop for a stack of `depth` subs whose coverage is `field`; `None` is a
    /// stack every sub covered completely. Below [`MIN_CROP_DEPTH`] it is shown whole.
    pub fn observe(&mut self, depth: usize, field: Option<&NoiseField>) -> CropDecision {
        if depth < self.depth {
            self.reset();
        }
        self.depth = depth;
        let decision = match field {
            Some(field) if depth >= MIN_CROP_DEPTH => self.decide(field),
            _ => CropDecision::Uncropped,
        };
        self.shown = decision.rect();
        decision
    }

    fn decide(&self, field: &NoiseField) -> CropDecision {
        let (w, h) = (field.source_width(), field.source_height());
        let fresh = largest_covered(field, PICK_COVERAGE);
        if fresh.is_some_and(|rect| rect.is_full(w, h)) {
            return CropDecision::Uncropped;
        }
        if let Some(shown) = self.shown {
            let kept = min_coverage(field, &shown).is_some_and(|c| c >= KEEP_COVERAGE - THRESHOLD_SLACK);
            let outgrown = fresh.is_some_and(|rect| rect.area() as f64 >= shown.area() as f64 * GROW_SHARE);
            if kept && !outgrown {
                return CropDecision::Cropped(shown);
            }
        }
        match fresh {
            Some(rect) if large_enough(&rect, w, h) => CropDecision::Cropped(rect),
            _ => CropDecision::TooThin,
        }
    }
}

fn large_enough(rect: &CropRect, width: usize, height: usize) -> bool {
    rect.width >= MIN_CROP_SIDE.min(width / 2)
        && rect.height >= MIN_CROP_SIDE.min(height / 2)
        && rect.area() as f64 >= MIN_CROP_SHARE * (width * height) as f64
}

#[cfg(test)]
#[path = "crop_tests.rs"]
mod tests;
