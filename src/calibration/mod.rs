//! Image Calibration Module for Dark and Flat Field Correction
//!
//! # Calibration Theory
//!
//! Raw astronomical images contain systematic errors that must be removed:
//!
//! ## Dark Frame Subtraction
//! Camera sensors produce thermal noise ("dark current") even with no light.
//! A **Master Dark** is created by averaging many dark frames taken at the
//! same temperature and exposure time as the light frames.
//!
//! **Math**: `calibrated = raw - dark`
//!
//! To prevent negative values (which would be invalid), we use:
//! `calibrated = max(0, raw - dark)`
//!
//! ## Flat Field Division
//! Optical systems have uneven illumination (vignetting) and dust spots.
//! A **Master Flat** captures this by imaging a uniformly lit surface.
//!
//! **Math**: `calibrated = raw / normalized_flat`
//!
//! The flat is normalized by dividing by its mean value, so:
//! - Areas with average illumination have flat ≈ 1.0 (no change)
//! - Dark corners have flat < 1.0 (brightened after division)
//! - Bright center has flat > 1.0 (dimmed after division)
//!
//! ## Combined Calibration
//! The full calibration sequence is:
//! `calibrated = (raw - dark) / normalized_flat`
//!
//! # Implementation Notes
//!
//! - All operations use SIMD-friendly loops for performance
//! - Rayon is used for parallel processing on multi-core systems
//! - A minimum flat threshold prevents division by near-zero values
//!
//! # Module Organization
//!
//! - `dark` - Master dark frame type
//! - `flat` - Master flat frame type with normalization
//! - `pipeline` - Calibration pipeline combining dark and flat
//! - `builders` - Functions to create master frames from multiple inputs
//! - `simd` - SIMD-optimized helper functions

mod builders;
mod dark;
mod flat;
mod pipeline;
mod simd;

pub use builders::{create_master_dark, create_master_flat};
pub use dark::MasterDark;
pub use flat::{MasterFlat, FLAT_MIN_THRESHOLD};
pub use pipeline::Calibration;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, PixelFormat};

    #[test]
    fn test_dark_subtraction_basic() {
        let mut light = Frame::filled(2, 2, 3, 0.5).unwrap();
        let dark_frame = Frame::filled(2, 2, 3, 0.1).unwrap();
        let dark = MasterDark::new(dark_frame);

        let calibration = Calibration::dark_only(dark);
        calibration.apply(&mut light).unwrap();

        for &v in light.data() {
            assert!((v - 0.4).abs() < 1e-6);
        }
    }

    #[test]
    fn test_dark_subtraction_clamps_negative() {
        let mut light = Frame::filled(2, 2, 3, 0.1).unwrap();
        let dark_frame = Frame::filled(2, 2, 3, 0.3).unwrap();
        let dark = MasterDark::new(dark_frame);

        let calibration = Calibration::dark_only(dark);
        calibration.apply(&mut light).unwrap();

        for &v in light.data() {
            assert!(v >= 0.0);
            assert!((v - 0.0).abs() < 1e-6);
        }
    }

    #[test]
    fn test_flat_normalization() {
        let data = vec![0.8, 0.9, 1.0, 1.1, 1.2, 1.0];
        let frame = Frame::from_f32_vec(data, 2, 1, 3).unwrap();
        let flat = MasterFlat::new(frame).unwrap();

        let mean: f32 = flat.frame().data().iter().sum::<f32>() / flat.frame().data().len() as f32;
        assert!((mean - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_flat_division_corrects_vignetting() {
        let mut light = Frame::filled(2, 2, 1, 1.0).unwrap();

        let flat_data = vec![0.5, 1.0, 1.0, 0.5];
        let flat_frame = Frame::from_f32_vec(flat_data, 2, 2, 1).unwrap();
        let flat = MasterFlat::new(flat_frame).unwrap();

        let calibration = Calibration::flat_only(flat);
        calibration.apply(&mut light).unwrap();

        let data = light.data();
        assert!(data[0] > data[1]);
    }

    #[test]
    fn test_combined_calibration() {
        let mut light = Frame::filled(2, 2, 3, 0.6).unwrap();

        let dark_frame = Frame::filled(2, 2, 3, 0.1).unwrap();
        let dark = MasterDark::new(dark_frame);

        let flat_frame = Frame::filled(2, 2, 3, 1.0).unwrap();
        let flat = MasterFlat::new(flat_frame).unwrap();

        let calibration = Calibration::new(Some(dark), Some(flat));
        calibration.apply(&mut light).unwrap();

        for &v in light.data() {
            assert!((v - 0.5).abs() < 0.01);
        }
    }

    #[test]
    fn test_dimension_mismatch_error() {
        use crate::error::StackError;

        let mut light = Frame::filled(4, 4, 3, 0.5).unwrap();
        let dark_frame = Frame::filled(2, 2, 3, 0.1).unwrap();
        let dark = MasterDark::new(dark_frame);

        let calibration = Calibration::dark_only(dark);
        let result = calibration.apply(&mut light);

        assert!(matches!(
            result,
            Err(StackError::CalibrationDimensionMismatch { .. })
        ));
    }

    #[test]
    fn test_master_dark_creation() {
        let frame1 = Frame::filled(2, 2, 3, 0.1).unwrap();
        let frame2 = Frame::filled(2, 2, 3, 0.2).unwrap();
        let frame3 = Frame::filled(2, 2, 3, 0.3).unwrap();

        let master = create_master_dark(vec![frame1, frame2, frame3]).unwrap();

        for &v in master.frame().data() {
            assert!((v - 0.2).abs() < 1e-6);
        }
    }

    #[test]
    fn test_master_flat_creation() {
        let frame1 = Frame::filled(2, 2, 3, 0.8).unwrap();
        let frame2 = Frame::filled(2, 2, 3, 1.0).unwrap();
        let frame3 = Frame::filled(2, 2, 3, 1.2).unwrap();

        let master = create_master_flat(vec![frame1, frame2, frame3]).unwrap();

        let mean: f32 =
            master.frame().data().iter().sum::<f32>() / master.frame().data().len() as f32;
        assert!((mean - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_from_raw_integration() {
        let light_raw = vec![200u8; 12];
        let dark_raw = vec![20u8; 12];
        let flat_raw = vec![250u8; 12];

        let mut light = Frame::from_raw(&light_raw, 2, 2, 3, PixelFormat::Rgb8).unwrap();
        let dark = MasterDark::from_raw(&dark_raw, 2, 2, 3, PixelFormat::Rgb8).unwrap();
        let flat = MasterFlat::from_raw(&flat_raw, 2, 2, 3, PixelFormat::Rgb8).unwrap();

        let calibration = Calibration::new(Some(dark), Some(flat));
        calibration.apply(&mut light).unwrap();

        for &v in light.data() {
            assert!(v >= 0.0);
            assert!(v <= 2.0);
        }
    }
}
