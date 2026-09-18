use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::{
    apply_luminance_preserving_simd, apply_luminance_preserving_simd_planar,
};

/// Configuration for S-curve contrast adjustment
#[derive(Debug, Clone, Copy)]
pub struct ContrastConfig {
    /// Strength of the S-curve effect (0.0 = no effect, 1.0 = maximum)
    pub strength: f32,
    /// Midpoint of the curve (where the slope is steepest)
    pub midpoint: f32,
}

/// `strength` is at its maximum and `midpoint` sits *below* the sky, which is what makes
/// that affordable: the sky falls on the curve's compressive half and the target on its
/// expansive one, so the curve brightens the target and darkens the sky in one pass
/// instead of trading one for the other. Measured on four sessions against `strength`
/// 0.8: target core +7-10 %, sky -2 output levels, octave-band sky noise within a few
/// percent everywhere, and the star radial profile slightly *better* (M27's r=13-25 tail
/// fell). Contrast is the only free brightness lever here — the tone curve's own
/// grain split (`AutoStretchConfig::grain_split`) costs target contrast 1:1.
///
/// The midpoint stays at 0.2. Lowering it was measured and rejected: it washes the
/// background out rather than lifting the target (sky 23 -> 32 output levels, p1 11 ->
/// 16 at 0.1), because it moves the sky onto the expansive half.
impl Default for ContrastConfig {
    fn default() -> Self {
        Self {
            strength: 1.0,
            midpoint: 0.2,
        }
    }
}

impl ContrastConfig {
    /// Create a new contrast configuration
    pub fn new(strength: f32, midpoint: f32) -> Self {
        Self {
            strength: strength.clamp(0.0, 1.0),
            midpoint: midpoint.clamp(0.1, 0.9),
        }
    }

    /// Create a subtle contrast boost
    pub fn subtle() -> Self {
        Self::new(0.3, 0.5)
    }

    /// Create a moderate contrast boost
    pub fn moderate() -> Self {
        Self::new(0.5, 0.5)
    }

    /// Create a strong contrast boost
    pub fn strong() -> Self {
        Self::new(0.7, 0.5)
    }

    /// Check if contrast is effectively disabled
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.strength < 1e-6
    }
}

/// Apply S-curve contrast to a single value
#[inline]
pub fn apply_s_curve(value: f32, config: &ContrastConfig) -> f32 {
    if config.is_disabled() {
        return value;
    }

    let x = value.clamp(0.0, 1.0);
    let strength = config.strength;
    let mid = config.midpoint;

    let deviation = x - mid;
    let bell = 4.0 * x * (1.0 - x);
    let adjustment = strength * deviation * bell;
    (x + adjustment).clamp(0.0, 1.0)
}

/// Apply S-curve contrast to a frame in-place (luminance-preserving)
pub fn apply_contrast_frame(frame: &mut Frame, config: &ContrastConfig) -> Result<()> {
    if frame.channels() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: frame.channels(),
        });
    }

    if config.is_disabled() {
        return Ok(());
    }

    let width = frame.width();
    let (r, g, b) = frame.planes_mut();
    apply_luminance_preserving_simd_planar(r, g, b, width, 1.0, |l| apply_s_curve(l, config));

    Ok(())
}

/// Apply S-curve contrast to a flat RGB slice (e.g. a row) in-place
pub fn apply_contrast_slice(row: &mut [f32], config: &ContrastConfig) {
    if config.is_disabled() {
        return;
    }
    apply_luminance_preserving_simd(row, 1.0, |l| apply_s_curve(l, config));
}
