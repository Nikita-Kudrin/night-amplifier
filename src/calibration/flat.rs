//! Master Flat frame for vignetting and dust correction

use crate::error::{Result, StackError};
use crate::frame::{Frame, PixelFormat};

use super::simd::{clamp_min_simd, multiply_scalar_simd, sum_f32_simd};

/// Minimum acceptable value in a flat field to prevent division artifacts
pub const FLAT_MIN_THRESHOLD: f32 = 0.01;

/// Master Flat frame for vignetting and dust correction
///
/// Created by averaging multiple flat frames taken of a uniformly illuminated
/// surface (twilight sky, light box, or white t-shirt over the telescope).
#[derive(Debug, Clone)]
pub struct MasterFlat {
    /// Normalized flat field (mean = 1.0)
    frame: Frame,
    /// Original mean value before normalization
    mean: f32,
}

impl MasterFlat {
    /// Creates a new MasterFlat from a Frame
    ///
    /// The flat field is automatically normalized so its mean equals 1.0.
    /// This ensures that applying the flat maintains overall image brightness.
    ///
    /// # Normalization Math
    /// For each pixel: `normalized[i] = original[i] / mean(original)`
    ///
    /// After normalization:
    /// - Pixels with average illumination ≈ 1.0
    /// - Darker areas (vignetting) < 1.0 → division brightens them
    /// - Brighter areas > 1.0 → division dims them
    ///
    /// # Performance
    /// Uses SIMD for sum computation and normalization multiplication.
    pub fn new(mut frame: Frame) -> Result<Self> {
        let data = frame.data();
        let len = data.len();

        let sum = sum_f32_simd(data);
        let mean = sum / len as f32;

        if mean < FLAT_MIN_THRESHOLD {
            return Err(StackError::InvalidFlatField { count: len });
        }

        let inv_mean = 1.0 / mean;
        multiply_scalar_simd(frame.data_mut(), inv_mean);
        clamp_min_simd(frame.data_mut(), FLAT_MIN_THRESHOLD);

        Ok(Self { frame, mean })
    }

    /// Creates a MasterFlat from raw image data
    pub fn from_raw(
        raw: &[u8],
        width: usize,
        height: usize,
        channels: usize,
        format: PixelFormat,
    ) -> Result<Self> {
        let frame = Frame::from_raw(raw, width, height, channels, format)?;
        Self::new(frame)
    }

    /// Returns the normalized flat field frame
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Returns the original mean value before normalization
    pub fn original_mean(&self) -> f32 {
        self.mean
    }

    /// Returns image dimensions (width, height, channels)
    pub fn dimensions(&self) -> (usize, usize, usize) {
        (
            self.frame.width(),
            self.frame.height(),
            self.frame.channels(),
        )
    }
}
