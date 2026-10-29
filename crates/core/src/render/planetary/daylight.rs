//! A colour camera's response to sunlight, which every planetary target is lit by.
//!
//! Background neutralisation balances a sky, and a planet's is black: its tiny pedestal's
//! colour became the gains, and Jupiter rendered green. Raw, an IMX533 shows the Moon
//! yellow-green (blue at 0.48 of green). Grey-world on the target fixed the Moon but turned Mars
//! khaki; one per-camera balance taken on a neutral target (the Moon, a white-light Sun) keeps
//! every target's own colour, an H-alpha disc's red included.

use super::PlanetaryLevels;
use crate::frame::Frame;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

/// Least target pixels a calibration averages: under it the mean is a few pixels' noise.
const MIN_CALIBRATION_PIXELS: usize = 1_000;

/// A sample at or above this tells nothing of its colour: a channel clipped at the sensor.
const CLIPPED: f32 = 0.999;

/// Gains on a camera's raw red, green and blue, the smallest 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct DaylightBalance {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Default for DaylightBalance {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl DaylightBalance {
    /// No balance: the camera's raw colour. What a camera that was never calibrated has.
    pub const IDENTITY: Self = Self { red: 1.0, green: 1.0, blue: 1.0 };

    /// Every gain's range once the smallest is 1.
    pub const GAIN_RANGE: RangeInclusive<f32> = 1.0..=4.0;

    /// Normalised so the smallest gain is 1, each within [`Self::GAIN_RANGE`]; a gain that is
    /// not a positive finite number reads 1. The smallest at 1 is what keeps a highlight
    /// clipped at the sensor neutral: [`Self::apply`] clips every channel at 1.0.
    pub fn sanitized(self) -> Self {
        let gain = |g: f32| if g.is_finite() && g > 0.0 { g } else { 1.0 };
        let (red, green, blue) = (gain(self.red), gain(self.green), gain(self.blue));
        let least = red.min(green).min(blue);
        let normalised = |g: f32| (g / least).clamp(*Self::GAIN_RANGE.start(), *Self::GAIN_RANGE.end());
        Self { red: normalised(red), green: normalised(green), blue: normalised(blue) }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// The gains on a three-channel `frame`, each channel clipped at 1.0: a pixel clipped at
    /// the sensor in every channel stays white, where unclipped it took the gains' tint.
    pub fn apply(&self, frame: &mut Frame) {
        if self.is_identity() || frame.channels() != 3 {
            return;
        }
        let (r, g, b) = frame.planes_mut();
        [(r, self.red), (g, self.green), (b, self.blue)]
            .into_par_iter()
            .filter(|(_, gain)| *gain != 1.0)
            .for_each(|(plane, gain)| {
                let chunk = crate::parallel::balanced_chunk_len(plane.len());
                plane.par_chunks_mut(chunk).for_each(|run| {
                    run.iter_mut().for_each(|v| *v = (*v * gain).min(1.0));
                });
            });
    }

    /// This balance corrected so `frame`'s target averages grey: `frame` is what this balance
    /// rendered, before its black point came off. That black is taken off here, and the clip
    /// test reads the samples as the sensor gave them: taken off first, a clipped channel read
    /// 1 - black and passed it (blue x1.82 for x2.07 under a 0.02 sky).
    pub fn calibrated(&self, frame: &Frame) -> Result<Self, CalibrationRefusal> {
        if frame.channels() != 3 {
            return Err(CalibrationRefusal::NoTarget);
        }
        let PlanetaryLevels { black, target_from, .. } = PlanetaryLevels::solve(frame);
        let (r, g, b) = frame.planes();
        let above_black = |v: f32| f64::from((v - black).max(0.0));
        let (count, sums) = (0..r.len())
            .into_par_iter()
            .filter(|&i| {
                let v = r[i].max(g[i]).max(b[i]);
                v >= target_from && v < CLIPPED
            })
            .map(|i| (1usize, [above_black(r[i]), above_black(g[i]), above_black(b[i])]))
            .reduce(|| (0, [0.0; 3]), |(n, a), (m, b)| (n + m, [a[0] + b[0], a[1] + b[1], a[2] + b[2]]));
        if count < MIN_CALIBRATION_PIXELS || sums.iter().any(|&s| s <= 0.0) {
            return Err(CalibrationRefusal::NoTarget);
        }
        let towards_green = |sum: f64, gain: f32| gain * (sums[1] / sum) as f32;
        let corrected = Self {
            red: towards_green(sums[0], self.red),
            green: self.green,
            blue: towards_green(sums[2], self.blue),
        };
        let gains = [corrected.red, corrected.green, corrected.blue];
        let spread = gains.iter().copied().fold(0.0, f32::max) / gains.iter().copied().fold(f32::INFINITY, f32::min);
        if spread > *Self::GAIN_RANGE.end() {
            return Err(CalibrationRefusal::NotNeutral);
        }
        Ok(corrected.sanitized())
    }
}

/// Why a frame cannot calibrate a [`DaylightBalance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationRefusal {
    /// Mono, or under [`MIN_CALIBRATION_PIXELS`] of unclipped target: a speck, or a clipped disc.
    NoTarget,
    /// Greying the target needs a gain past [`DaylightBalance::GAIN_RANGE`]: an H-alpha Sun or a
    /// planet's own colour, not sunlight. Clamped, it saved gains pinned at the range's end.
    NotNeutral,
}
