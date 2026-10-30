//! A colour camera's response to sunlight, which every planetary target is lit by.
//!
//! Background neutralisation balances a sky, and a planet's is black: its tiny pedestal's
//! colour became the gains, and Jupiter rendered green. Raw, an IMX533 shows the Moon
//! yellow-green (blue at 0.49 of green). Grey-world on the target fixed the Moon but turned Mars
//! khaki; one per-camera balance keeps every target's own colour, an H-alpha disc's red
//! included. It starts as [`DaylightBalance::PRIOR`] and is read off the Moon whenever
//! Planetary shows one (`super::target`): no button, no star to point at.

use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

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
    /// No balance: the camera's raw colour. What a mono camera has, and what every
    /// non-planetary render goes through.
    pub const IDENTITY: Self = Self { red: 1.0, green: 1.0, blue: 1.0 };

    /// What a colour camera renders through until the Moon has been seen: ZWO's factory
    /// default is x1.04 / x1.90, an IMX533 measured x1.06-1.14 / x2.03-2.05. Within 25 % of
    /// any Sony colour sensor behind a UV/IR cut, where identity is 100 % off in blue.
    pub const PRIOR: Self = Self { red: 1.10, green: 1.0, blue: 1.95 };

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

    /// Whether any gain differs from `other`'s by more than `share` of it.
    pub fn differs_from(&self, other: &Self, share: f32) -> bool {
        [(self.red, other.red), (self.green, other.green), (self.blue, other.blue)]
            .into_iter()
            .any(|(a, b)| (a - b).abs() > share * b)
    }

    /// The gains on a three-channel `frame`, each channel clipped at 1.0: a pixel clipped at
    /// the sensor in every channel stays white, where unclipped it took the gains' tint.
    pub fn apply(&self, frame: &mut crate::frame::Frame) {
        use rayon::prelude::*;
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

    /// This balance with its red and blue gains times `red` and `blue` (green held), or
    /// `None` where the result would need gains past [`Self::GAIN_RANGE`]: clamped, a
    /// balance pinned at the range's end once tinted every later target.
    pub fn corrected(&self, red: f32, blue: f32) -> Option<Self> {
        let corrected = Self { red: self.red * red, green: self.green, blue: self.blue * blue };
        let gains = [corrected.red, corrected.green, corrected.blue];
        if gains.iter().any(|g| !g.is_finite() || *g <= 0.0) {
            return None;
        }
        let spread = gains.iter().copied().fold(0.0, f32::max) / gains.iter().copied().fold(f32::INFINITY, f32::min);
        (spread <= *Self::GAIN_RANGE.end()).then(|| corrected.sanitized())
    }
}

/// Where a camera's [`DaylightBalance`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum DaylightBalanceSource {
    /// Nothing measured yet: the balance is [`DaylightBalance::PRIOR`].
    #[default]
    Prior,
    /// Read off the Moon through this camera.
    Moon,
}
