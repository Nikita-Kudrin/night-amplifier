use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;
use tracing::info;

/// Configuration for S-curve contrast adjustment
#[derive(Debug, Clone, Copy)]
pub struct ContrastConfig {
    /// Strength of the S-curve effect (0.0 = no effect, 1.0 = maximum)
    pub strength: f32,
    /// Midpoint of the curve (where the slope is steepest)
    pub midpoint: f32,
}

impl Default for ContrastConfig {
    fn default() -> Self {
        Self {
            strength: 0.8,
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

    let data = frame.data_mut();

    data.par_chunks_mut(3).for_each(|pixel| {
        let r = pixel[0];
        let g = pixel[1];
        let b = pixel[2];

        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

        if luminance <= 1e-8 {
            return;
        }

        let luminance_adjusted = apply_s_curve(luminance, config);
        let scale = luminance_adjusted / luminance;

        pixel[0] = (r * scale).clamp(0.0, 1.0);
        pixel[1] = (g * scale).clamp(0.0, 1.0);
        pixel[2] = (b * scale).clamp(0.0, 1.0);
    });

    Ok(())
}
