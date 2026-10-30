//! What a planetary frame's target looks like, and whether it is the Moon: the one target
//! bright, common and neutral enough to read a camera's daylight balance off automatically.
//!
//! Three things tell the Moon from the rest. A planet's disc darkens towards its limb (Jupiter
//! to 0.40-0.44 of its centre on the `Planets` stacks) where the Moon's limb is as bright as its
//! centre (0.79-1.42 on the mosaic panels, 1.06-1.17 on the drifting Moon). Maria and craters
//! texture the Moon at every scale, where a planet, a twilight sky or a defocused blob is smooth
//! once its own grain is discounted. And its colour under the balance in force sits near the
//! Moon's: Mars is 0.15 mag redder than anything else, an eclipsed Moon or an H-alpha Sun far
//! more, the ice giants 0.4 mag bluer. Colour and shape are read above the sky, per channel
//! (`super::sky`), and sizes in sensor pixels whatever the preview's bin.

use super::sky::{self, Block, Sky};
use super::{DaylightBalance, PlanetaryLevels};
use crate::frame::Frame;
use rayon::prelude::*;

/// A sample at or above this tells nothing of its colour: a channel clipped at the sensor.
const CLIPPED: f32 = 0.999;

/// Least unclipped target pixels a measure averages, in sensor pixels like [`MIN_DIAMETER`]:
/// under it the mean is a few pixels' noise.
pub const MIN_PIXELS: usize = 5_000;

/// Least equivalent disc diameter, in sensor pixels: a planet at a short focal length is a speck
/// whose limb and texture cannot be read. In sensor pixels, not the binned preview's
/// ([`TargetMeasure::binned`]), so the Preview Resolution setting does not decide what is the Moon.
pub const MIN_DIAMETER: f32 = 150.0;

/// The shape measures are read on the disc resampled to about this many pixels across, so a
/// Moon at 135 mm and one at 3 m score alike.
const SHAPE_SIZE: f32 = 256.0;

/// A target's limb at or over this share of its centre is undarkened: the Moon, not a planet.
pub const LIMB_UNDARKENED: f32 = 0.65;

/// Texture (the mean deviation from a 5x5 local mean, as a share of the disc's brightness)
/// past what the disc's own grain accounts for, at or over which a disc has surface detail:
/// the drifting Moon reads 0.009-0.020 with up to 3 % noise added, the mosaic panels
/// 0.014-0.018, a featureless disc 0 however noisy, a twilight sky read as target 0.0034.
/// Jupiter's belts read 0.005-0.015 on sharpened stacks; its limb is what refuses it.
pub const TEXTURE_MIN: f32 = 0.005;

/// What the mean absolute deviation of pure Gaussian noise is of its sigma.
const NOISE_DEVIATION: f32 = 0.8;

/// A sky whose blocks spread past this share of the target's brightness above it is too uneven
/// for its median to stand for what lies over the disc. A flat sky, twilight's or a pedestal's,
/// is taken off whatever its level (`super::sky`).
const MAX_HAZE: f32 = 0.02;

/// Over this share of target pixels clipped, what remains unclipped is the disc's darker, bluer
/// maria rather than its average.
const MAX_CLIPPED_SHARE: f32 = 0.2;

/// The Moon's colour in sunlight, relative to a grey that reflects sunlight evenly, as a colour
/// camera's red, green and blue see it. By B-V (0.92 against the Sun's 0.65) it is (1.12, 1,
/// 0.85); against seven Sun-like stars on an IMX533 it read (1.08, 1, 0.99). The middle, until
/// a star calibration settles it per camera.
pub const MOON_COLOUR: [f32; 3] = [1.10, 1.00, 0.92];

/// How far the red and blue corrections a Moon implies may stray from the balance in force,
/// as ratios: red wide, since a camera without a UV/IR cut filter reads a third redder than
/// the prior; blue narrow, since only extinction moves it, and Mars (blue at 0.67 of the
/// Moon's) and the ice giants (1.4-1.5) lie outside it.
const RED_CORRECTION: (f32, f32) = (0.5, 2.0);
const BLUE_CORRECTION: (f32, f32) = (0.75, 1.33);

/// Fewest resampled cells the centre and the limb each need for a limb ratio.
const LIMB_CELLS: usize = 20;

/// What a planetary frame's target measures: its colour, size and shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetMeasure {
    /// Mean of each channel over the unclipped target pixels less the sky's, or `None` for a
    /// mono frame, no target or a channel with nothing above its sky.
    pub colour: Option<[f32; 3]>,
    /// Target pixels with no channel clipped, and those with one.
    pub unclipped: usize,
    pub clipped: usize,
    /// Diameter of the disc that would hold the target's pixels.
    pub diameter: f32,
    /// Mean deviation from a 5x5 local mean on the resampled disc, over its brightness.
    pub texture: f32,
    /// The disc's own cell-to-cell noise at the same scale, over its brightness: from second
    /// differences along rows, which a planet's limb-darkening gradient cancels out of.
    pub grain: f32,
    /// The limb's median brightness (0.85-0.95 of the equivalent radius) over the centre's
    /// (inside half of it); `None` where either lies off the frame, as a zoomed-in Moon's does.
    pub limb: Option<f32>,
    /// The sky's level per channel, `None` with too little sky to read (`super::sky`).
    pub sky: Option<[f32; 3]>,
    /// The spread of the sky's blocks over the target's brightness above the sky; 0 with no sky.
    pub haze: f32,
    /// Sensor pixels per measured pixel along each axis, 1 unless [`Self::binned`].
    pub bin: usize,
}

/// Why a frame's target is not taken for the Moon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotTheMoon {
    /// A mono frame has no colour to balance.
    Mono,
    /// Fewer than [`MIN_PIXELS`] unclipped, or a disc under [`MIN_DIAMETER`].
    TooSmall,
    /// Over [`MAX_CLIPPED_SHARE`] of the target clipped at the sensor.
    Clipped,
    /// A sky spread past [`MAX_HAZE`] of the target's brightness.
    HazySky,
    /// No surface detail past the frame's grain: a planet's bands, a twilight sky, a blob.
    Smooth,
    /// A disc darkening towards its limb: a planet.
    LimbDarkened,
    /// A colour that is not the Moon's under any plausible balance, or one needing gains past
    /// [`DaylightBalance::GAIN_RANGE`]: Mars, an eclipse, a filter.
    Coloured,
}

impl TargetMeasure {
    /// The measure of `frame`'s target above `levels.target_from`, its colour less the sky's. One
    /// pass over the frame, one over the target's bounding box.
    pub fn of(frame: &Frame, levels: &PlanetaryLevels) -> Self {
        let (width, height) = (frame.width(), frame.height());
        let (r, g, b) = match frame.channels() {
            3 => frame.planes(),
            _ => (frame.channel_data(0), frame.channel_data(0), frame.channel_data(0)),
        };
        let target_from = levels.target_from;
        let (extent, blocks) = Extent::of((r, g, b), width, height, target_from);
        let sky = Sky::of(&blocks, width, height);

        let total = extent.total();
        let diameter = 2.0 * (total as f32 / std::f32::consts::PI).sqrt();
        let brightness = |x: usize, y: usize| {
            let i = y * width + x;
            r[i].max(g[i]).max(b[i])
        };
        let floor = sky.map_or(0.0, |s| s.brightness);
        let shape = (total > 0).then(|| Shape::of(&extent, diameter, (target_from, floor), brightness));
        let mean = |sum: f64| (sum / extent.unclipped.max(1) as f64) as f32;
        let colour = [0, 1, 2].map(|c| mean(extent.sums[c]) - sky.map_or(0.0, |s| s.level[c]));
        let has_colour = frame.channels() == 3 && extent.unclipped > 0 && colour.iter().all(|&c| c > 0.0);
        let haze = sky.map_or(0.0, |s| s.spread / (mean(extent.brightness) - s.brightness).max(f32::MIN_POSITIVE));
        Self {
            colour: has_colour.then_some(colour),
            unclipped: extent.unclipped,
            clipped: extent.clipped,
            diameter,
            texture: shape.as_ref().map_or(0.0, |s| s.texture),
            grain: shape.as_ref().map_or(0.0, |s| s.grain),
            limb: shape.and_then(|s| s.limb),
            sky: sky.map(|s| s.level),
            haze,
            bin: 1,
        }
    }

    /// This measure of a frame binned by `bin` from the sensor's: the size gates count sensor
    /// pixels, the shape is read on the frame as measured.
    pub fn binned(self, bin: usize) -> Self {
        Self { bin: bin.max(1), ..self }
    }

    /// Surface detail past what the disc's grain accounts for.
    pub fn texture_excess(&self) -> f32 {
        self.texture - NOISE_DEVIATION * self.grain
    }

    /// The balance that renders this target in the Moon's colour, if it is the Moon as seen
    /// through `balance`: each gate in turn, the first failing one named.
    pub fn moon(&self, balance: &DaylightBalance) -> Result<DaylightBalance, NotTheMoon> {
        let Some(colour) = self.colour else {
            return Err(if self.unclipped == 0 { NotTheMoon::TooSmall } else { NotTheMoon::Mono });
        };
        if self.unclipped * self.bin * self.bin < MIN_PIXELS || self.diameter * (self.bin as f32) < MIN_DIAMETER {
            return Err(NotTheMoon::TooSmall);
        }
        if self.clipped as f32 > MAX_CLIPPED_SHARE * (self.clipped + self.unclipped) as f32 {
            return Err(NotTheMoon::Clipped);
        }
        if self.haze > MAX_HAZE {
            return Err(NotTheMoon::HazySky);
        }
        if self.texture_excess() < TEXTURE_MIN {
            return Err(NotTheMoon::Smooth);
        }
        if self.limb.is_some_and(|limb| limb < LIMB_UNDARKENED) {
            return Err(NotTheMoon::LimbDarkened);
        }
        let correction = |c: usize| (MOON_COLOUR[c] / MOON_COLOUR[1]) / (colour[c] / colour[1]);
        let within = |v: f32, (low, high): (f32, f32)| v >= low && v <= high;
        let (red, blue) = (correction(0), correction(2));
        if !within(red, RED_CORRECTION) || !within(blue, BLUE_CORRECTION) {
            return Err(NotTheMoon::Coloured);
        }
        balance.corrected(red, blue).ok_or(NotTheMoon::Coloured)
    }
}

/// The target's pixel extent, summed in parallel.
#[derive(Debug, Default, Clone, Copy)]
struct Extent {
    unclipped: usize,
    clipped: usize,
    /// Each channel's sum, and the brightest channel's, over the unclipped target pixels.
    sums: [f64; 3],
    brightness: f64,
    sum_x: f64,
    sum_y: f64,
    min_x: usize,
    max_x: usize,
    min_y: usize,
    max_y: usize,
}

impl Extent {
    /// The target above `target_from` of a `width` x `height` frame's planes, and the sky below
    /// it summed by [`sky::BLOCK`]-pixel blocks, row-major. Chunks hold whole block rows, so
    /// each fills only its own blocks and they join in order.
    fn of((r, g, b): (&[f32], &[f32], &[f32]), width: usize, height: usize, target_from: f32) -> (Self, Vec<Block>) {
        let (cols, block_rows) = (width.div_ceil(sky::BLOCK), height.div_ceil(sky::BLOCK));
        let chunk = (crate::parallel::balanced_chunk_len(width * height) / width.max(1)).div_ceil(sky::BLOCK).max(1);
        let chunks: Vec<(Self, Vec<Block>)> = (0..block_rows.div_ceil(chunk))
            .into_par_iter()
            .map(|k| {
                let first = k * chunk;
                let (mut e, mut blocks) = (Self::default(), vec![Block::default(); (chunk.min(block_rows - first)) * cols]);
                for y in first * sky::BLOCK..((first + chunk) * sky::BLOCK).min(height) {
                    let (row, block_row) = (y * width..(y + 1) * width, (y / sky::BLOCK - first) * cols);
                    for (x, ((&r, &g), &b)) in r[row.clone()].iter().zip(&g[row.clone()]).zip(&b[row]).enumerate() {
                        let v = r.max(g).max(b);
                        if v < target_from {
                            blocks[block_row + x / sky::BLOCK].add([r, g, b]);
                            continue;
                        }
                        e.count(x, y);
                        if v >= CLIPPED {
                            e.clipped += 1;
                            continue;
                        }
                        e.unclipped += 1;
                        e.sums[0] += f64::from(r);
                        e.sums[1] += f64::from(g);
                        e.sums[2] += f64::from(b);
                        e.brightness += f64::from(v);
                    }
                }
                (e, blocks)
            })
            .collect();
        let extent = chunks.iter().fold(Self::default(), |a, (e, _)| Self::merge(a, *e));
        (extent, chunks.into_iter().flat_map(|(_, blocks)| blocks).collect())
    }

    fn total(&self) -> usize {
        self.unclipped + self.clipped
    }

    fn count(&mut self, x: usize, y: usize) {
        if self.total() == 0 {
            (self.min_x, self.max_x, self.min_y, self.max_y) = (x, x, y, y);
        } else {
            self.min_x = self.min_x.min(x);
            self.max_x = self.max_x.max(x);
            self.min_y = self.min_y.min(y);
            self.max_y = self.max_y.max(y);
        }
        self.sum_x += x as f64;
        self.sum_y += y as f64;
    }

    fn merge(a: Self, b: Self) -> Self {
        match (a.total(), b.total()) {
            (0, _) => b,
            (_, 0) => a,
            _ => Self {
                unclipped: a.unclipped + b.unclipped,
                clipped: a.clipped + b.clipped,
                sums: [a.sums[0] + b.sums[0], a.sums[1] + b.sums[1], a.sums[2] + b.sums[2]],
                brightness: a.brightness + b.brightness,
                sum_x: a.sum_x + b.sum_x,
                sum_y: a.sum_y + b.sum_y,
                min_x: a.min_x.min(b.min_x),
                max_x: a.max_x.max(b.max_x),
                min_y: a.min_y.min(b.min_y),
                max_y: a.max_y.max(b.max_y),
            },
        }
    }
}

/// The target's bounding box resampled by whole cells so the disc is about [`SHAPE_SIZE`]
/// across; a cell is target only if every pixel of it is.
struct Cells {
    cols: usize,
    rows: usize,
    cell: usize,
    values: Vec<(f32, bool)>,
}

impl Cells {
    fn of(extent: &Extent, diameter: f32, target_from: f32, brightness: impl Fn(usize, usize) -> f32 + Sync) -> Self {
        let cell = ((diameter / SHAPE_SIZE).round() as usize).max(1);
        let cols = (extent.max_x - extent.min_x + 1) / cell;
        let rows = (extent.max_y - extent.min_y + 1) / cell;
        let values = (0..rows)
            .into_par_iter()
            .flat_map_iter(|cy| {
                let brightness = &brightness;
                (0..cols).map(move |cx| {
                    let (mut sum, mut all) = (0.0f32, true);
                    for y in 0..cell {
                        for x in 0..cell {
                            let v = brightness(extent.min_x + cx * cell + x, extent.min_y + cy * cell + y);
                            sum += v;
                            all &= v >= target_from;
                        }
                    }
                    (sum / (cell * cell) as f32, all)
                })
            })
            .collect();
        Self { cols, rows, cell, values }
    }

    fn at(&self, x: usize, y: usize) -> (f32, bool) {
        self.values[y * self.cols + x]
    }

    fn is_target(&self, x: isize, y: isize) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.cols && (y as usize) < self.rows && self.at(x as usize, y as usize).1
    }

    fn target(&self) -> impl Iterator<Item = (usize, usize, f32)> + '_ {
        (0..self.rows).flat_map(move |y| (0..self.cols).filter_map(move |x| self.at(x, y).1.then_some((x, y, self.at(x, y).0))))
    }

    /// The mean of the 5x5 cells about `(x, y)`, if all of them are target.
    fn local_mean(&self, x: usize, y: usize) -> Option<f32> {
        let (xi, yi) = (x as isize, y as isize);
        let around = || (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (xi + dx, yi + dy)));
        around().all(|(x, y)| self.is_target(x, y)).then(|| {
            around().map(|(x, y)| self.at(x as usize, y as usize).0).sum::<f32>() / 25.0
        })
    }
}

/// Texture, grain and limb ratio of the target, read on its [`Cells`] as shares of its
/// brightness above the sky: a pedestal times the blue gain diluted a dim Moon's texture 10 %
/// and lifted a planet's limb towards its centre.
struct Shape {
    texture: f32,
    grain: f32,
    limb: Option<f32>,
}

impl Shape {
    fn of(
        extent: &Extent,
        diameter: f32,
        (target_from, floor): (f32, f32),
        brightness: impl Fn(usize, usize) -> f32 + Sync,
    ) -> Self {
        let none = Self { texture: 0.0, grain: 0.0, limb: None };
        let cells = Cells::of(extent, diameter, target_from, brightness);
        if cells.cols < 7 || cells.rows < 7 {
            return none;
        }
        let (sum, n) = cells.target().fold((0.0f64, 0usize), |(s, n), (_, _, v)| (s + f64::from(v), n + 1));
        if n == 0 {
            return none;
        }
        let mean = ((sum / n as f64) as f32 - floor).max(f32::MIN_POSITIVE);

        let (mut deviation, mut interior) = (0.0f64, 0usize);
        let mut steps = Vec::new();
        for (x, y, v) in cells.target() {
            if x >= 1 && x + 1 < cells.cols && cells.at(x - 1, y).1 && cells.at(x + 1, y).1 {
                steps.push((cells.at(x - 1, y).0 - 2.0 * v + cells.at(x + 1, y).0).abs());
            }
            if let Some(local) = cells.local_mean(x, y) {
                deviation += f64::from((v - local).abs());
                interior += 1;
            }
        }
        let texture = if interior == 0 { 0.0 } else { (deviation / interior as f64) as f32 / mean };
        let grain = if steps.is_empty() {
            0.0
        } else {
            crate::statistics::MAD_TO_SIGMA * median(&mut steps) / 6.0f32.sqrt() / mean
        };

        let radius = diameter / 2.0 / cells.cell as f32;
        let centre_of = |sum: f64, min: usize| ((sum / extent.total() as f64) as f32 - min as f32) / cells.cell as f32;
        let (cx, cy) = (centre_of(extent.sum_x, extent.min_x), centre_of(extent.sum_y, extent.min_y));
        let (mut centre, mut limb) = (Vec::new(), Vec::new());
        for (x, y, v) in cells.target() {
            let r = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) / radius;
            if r < 0.5 {
                centre.push(v);
            } else if (0.85..0.95).contains(&r) {
                limb.push(v);
            }
        }
        let limb = (centre.len() >= LIMB_CELLS && limb.len() >= LIMB_CELLS)
            .then(|| (median(&mut limb) - floor) / (median(&mut centre) - floor).max(f32::MIN_POSITIVE));
        Self { texture, grain, limb }
    }
}

fn median(values: &mut [f32]) -> f32 {
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, f32::total_cmp).1
}

#[cfg(test)]
#[path = "target_tests.rs"]
mod tests;
