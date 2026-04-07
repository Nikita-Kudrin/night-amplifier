//! FITS file writing support for astronomical image storage
//!
//! This module provides functionality to write Frame data to FITS (Flexible Image
//! Transport System) format files, the standard format for astronomical imaging.

use std::path::Path;
use tracing::{debug, error, instrument};

use crate::error::{Result, StackError};
use crate::ffi_safety::catch_ffi_panic;
use crate::frame::Frame;
use fitsio::FitsFile;

mod header;
mod metadata;
mod writers;

pub use metadata::FitsMetadata;

use writers::{write_fits_u16_primary, write_mono_fits, write_rgb_fits};

/// Write a Frame to a FITS file
#[instrument(skip(frame, metadata), fields(
    path = %path.as_ref().display(),
    resolution = %format!("{}x{}x{}", frame.width(), frame.height(), frame.channels()),
    format = "f32"
))]
pub fn write_fits(
    frame: &Frame,
    path: impl AsRef<Path>,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    let path = path.as_ref();

    if path.exists() {
        std::fs::remove_file(path).map_err(|e| StackError::ArithmeticError {
            message: format!("Failed to remove existing file: {}", e),
        })?;
    }

    debug!(path = ?path, width = frame.width(), height = frame.height(), channels = frame.channels(), "Writing FITS file");

    let path_buf = path.to_path_buf();
    let mut fptr = catch_ffi_panic("cfitsio::create", || FitsFile::create(&path_buf).open())
        .map_err(StackError::from)?
        .map_err(|e| {
            error!(error = %e, "Failed to create FITS file");
            StackError::ArithmeticError {
                message: format!("Failed to create FITS file: {}", e),
            }
        })?;

    if frame.channels() == 1 {
        write_mono_fits(&mut fptr, frame, metadata)?;
    } else {
        write_rgb_fits(&mut fptr, frame, metadata)?;
    }

    Ok(())
}

/// Write a Frame to a 16-bit FITS file
#[instrument(skip(frame, metadata), fields(
    path = %path.as_ref().display(),
    resolution = %format!("{}x{}x{}", frame.width(), frame.height(), frame.channels()),
    format = "u16"
))]
pub fn write_fits_u16(
    frame: &Frame,
    path: impl AsRef<Path>,
    metadata: Option<&FitsMetadata>,
) -> Result<()> {
    let path = path.as_ref();

    if path.exists() {
        std::fs::remove_file(path).map_err(|e| StackError::ArithmeticError {
            message: format!("Failed to remove existing file: {}", e),
        })?;
    }

    debug!(path = ?path, "Writing 16-bit FITS file to primary HDU");

    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    let data = frame.data();
    let u16_data: Vec<u16> = if channels == 1 {
        data.iter()
            .map(|&v| (v.clamp(0.0, 1.0) * 65535.0) as u16)
            .collect()
    } else {
        let pixels_per_channel = width * height;
        let mut planar = vec![0u16; pixels_per_channel * channels];
        for y in 0..height {
            for x in 0..width {
                let pixel_idx = y * width + x;
                for c in 0..channels {
                    let src_idx = pixel_idx * channels + c;
                    let dst_idx = c * pixels_per_channel + pixel_idx;
                    planar[dst_idx] = (data[src_idx].clamp(0.0, 1.0) * 65535.0) as u16;
                }
            }
        }
        planar
    };

    write_fits_u16_primary(&u16_data, width, height, channels, path, metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fits_metadata_builder() {
        let meta = FitsMetadata::new()
            .with_exposure_us(1_000_000)
            .with_gain(100)
            .with_offset(10)
            .with_camera("Test Camera")
            .with_frame_number(1);

        assert_eq!(meta.exposure_s, Some(1.0));
        assert_eq!(meta.gain, Some(100));
        assert_eq!(meta.offset, Some(10));
        assert_eq!(meta.camera, Some("Test Camera".to_string()));
        assert_eq!(meta.frame_number, Some(1));
    }

    #[test]
    fn test_write_mono_fits() {
        let frame = Frame::filled(100, 100, 1, 0.5).unwrap();
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_mono.fits");

        let meta = FitsMetadata::new()
            .with_exposure_us(1_000_000)
            .with_gain(50);

        let result = write_fits(&frame, &path, Some(&meta));
        assert!(result.is_ok(), "Failed to write mono FITS: {:?}", result);
        assert!(path.exists());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_write_rgb_fits() {
        let frame = Frame::filled(100, 100, 3, 0.5).unwrap();
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_rgb.fits");

        let result = write_fits(&frame, &path, None);
        assert!(result.is_ok(), "Failed to write RGB FITS: {:?}", result);
        assert!(path.exists());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_write_fits_u16() {
        let frame = Frame::filled(100, 100, 1, 0.5).unwrap();
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_u16.fits");

        let result = write_fits_u16(&frame, &path, None);
        assert!(result.is_ok(), "Failed to write u16 FITS: {:?}", result);
        assert!(path.exists());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_fits_metadata_all_fields() {
        let meta = FitsMetadata::new()
            .with_exposure_us(2_000_000)
            .with_gain(150)
            .with_offset(20)
            .with_camera("ASI294MC Pro")
            .with_date_obs("2024-01-15T22:30:00.000")
            .with_frame_number(42)
            .with_stacked_frames(100)
            .with_binning(2);

        assert_eq!(meta.exposure_s, Some(2.0));
        assert_eq!(meta.gain, Some(150));
        assert_eq!(meta.offset, Some(20));
        assert_eq!(meta.camera, Some("ASI294MC Pro".to_string()));
        assert_eq!(meta.date_obs, Some("2024-01-15T22:30:00.000".to_string()));
        assert_eq!(meta.frame_number, Some(42));
        assert_eq!(meta.stacked_frames, Some(100));
        assert_eq!(meta.binning, Some(2));
        assert_eq!(meta.software, Some("Night Amplifier".to_string()));
    }

    #[test]
    fn test_fits_metadata_default() {
        let meta = FitsMetadata::default();
        assert!(meta.exposure_s.is_none());
    }

    #[test]
    fn test_fits_metadata_new_sets_software() {
        let meta = FitsMetadata::new();
        assert_eq!(meta.software, Some("Night Amplifier".to_string()));
    }
}
