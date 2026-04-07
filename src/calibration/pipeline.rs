//! Calibration pipeline for applying dark and flat corrections

use crate::error::{Result, StackError};
use crate::frame::Frame;
use tracing::instrument;

use super::dark::MasterDark;
use super::flat::MasterFlat;
use super::simd::{divide_simd, subtract_clamp_zero_simd};

/// Calibration pipeline for applying dark and flat corrections
///
/// # Example
/// ```
/// use night_amplifier::{Frame, PixelFormat, Calibration, MasterDark, MasterFlat};
///
/// // Create calibration frames (normally loaded from files)
/// let dark_data = vec![10u8; 12]; // 2x2 RGB dark frame
/// let flat_data = vec![200u8; 12]; // 2x2 RGB flat frame
///
/// let dark = MasterDark::from_raw(&dark_data, 2, 2, 3, PixelFormat::Rgb8).unwrap();
/// let flat = MasterFlat::from_raw(&flat_data, 2, 2, 3, PixelFormat::Rgb8).unwrap();
///
/// let calibration = Calibration::new(Some(dark), Some(flat));
///
/// // Apply to a light frame
/// let light_data = vec![128u8; 12];
/// let mut light = Frame::from_raw(&light_data, 2, 2, 3, PixelFormat::Rgb8).unwrap();
/// calibration.apply(&mut light).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct Calibration {
    dark: Option<MasterDark>,
    flat: Option<MasterFlat>,
}

impl Calibration {
    /// Creates a new Calibration with optional dark and flat frames
    pub fn new(dark: Option<MasterDark>, flat: Option<MasterFlat>) -> Self {
        Self { dark, flat }
    }

    /// Creates a Calibration with only dark frame correction
    pub fn dark_only(dark: MasterDark) -> Self {
        Self {
            dark: Some(dark),
            flat: None,
        }
    }

    /// Creates a Calibration with only flat field correction
    pub fn flat_only(flat: MasterFlat) -> Self {
        Self {
            dark: None,
            flat: Some(flat),
        }
    }

    /// Applies calibration to a frame in-place
    ///
    /// # Calibration Sequence
    /// 1. **Dark Subtraction**: `frame = max(0, frame - dark)`
    ///    - Removes thermal noise and bias
    ///    - Uses max(0, ...) to prevent negative values
    ///
    /// 2. **Flat Division**: `frame = frame / flat`
    ///    - Corrects vignetting and dust spots
    ///    - Flat is pre-normalized (mean = 1.0) to preserve brightness
    ///
    /// # Errors
    /// Returns an error if the calibration frame dimensions don't match.
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        has_dark = self.dark.is_some(),
        has_flat = self.flat.is_some()
    ))]
    pub fn apply(&self, frame: &mut Frame) -> Result<()> {
        if let Some(ref dark) = self.dark {
            self.apply_dark(frame, dark)?;
        }

        if let Some(ref flat) = self.flat {
            self.apply_flat(frame, flat)?;
        }

        Ok(())
    }

    /// Applies dark subtraction to a frame
    ///
    /// # Math
    /// For each pixel: `result = max(0, frame - dark)`
    ///
    /// Using max(0, ...) prevents negative values that could occur when:
    /// - Random noise causes dark pixel > light pixel
    /// - The dark frame was taken at a different temperature
    fn apply_dark(&self, frame: &mut Frame, dark: &MasterDark) -> Result<()> {
        if !frame.dimensions_match(dark.frame()) {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: dark.frame().width(),
                cal_height: dark.frame().height(),
            });
        }

        let dark_data = dark.frame().data();
        let frame_data = frame.data_mut();

        subtract_clamp_zero_simd(frame_data, dark_data);

        Ok(())
    }

    /// Applies flat field correction to a frame
    ///
    /// # Math
    /// For each pixel: `result = frame / flat`
    ///
    /// Since flat is normalized (mean = 1.0):
    /// - Overall brightness is preserved
    /// - Dark corners (flat < 1.0) are brightened
    /// - Bright center (flat > 1.0) is dimmed
    fn apply_flat(&self, frame: &mut Frame, flat: &MasterFlat) -> Result<()> {
        if !frame.dimensions_match(flat.frame()) {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: flat.frame().width(),
                cal_height: flat.frame().height(),
            });
        }

        let flat_data = flat.frame().data();
        let frame_data = frame.data_mut();

        divide_simd(frame_data, flat_data);

        Ok(())
    }

    /// Returns whether this calibration has a dark frame
    pub fn has_dark(&self) -> bool {
        self.dark.is_some()
    }

    /// Returns whether this calibration has a flat frame
    pub fn has_flat(&self) -> bool {
        self.flat.is_some()
    }

    /// Returns a reference to the dark frame if present
    pub fn dark(&self) -> Option<&MasterDark> {
        self.dark.as_ref()
    }

    /// Returns a reference to the flat frame if present
    pub fn flat(&self) -> Option<&MasterFlat> {
        self.flat.as_ref()
    }
}
