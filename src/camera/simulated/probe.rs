//! Image dimension and format probing

use std::fs;
use std::path::Path;

use crate::camera::error::{CameraError, CameraResult};
use crate::camera::types::SensorType;
use crate::CfaPattern;

use super::registry::{FITS_EXTENSIONS, PNG_EXTENSIONS, TIFF_EXTENSIONS};

/// Probe an image file to get its dimensions and sensor type
pub fn probe_image_dimensions(
    path: &Path,
) -> CameraResult<(u32, u32, SensorType, Option<CfaPattern>)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if TIFF_EXTENSIONS.contains(&ext.as_str()) {
        probe_tiff_dimensions(path)
    } else if FITS_EXTENSIONS.contains(&ext.as_str()) {
        probe_fits_dimensions(path)
    } else if PNG_EXTENSIONS.contains(&ext.as_str()) {
        probe_png_dimensions(path)
    } else {
        Err(CameraError::ImageReadFailed(format!(
            "Unsupported file format: {}",
            ext
        )))
    }
}

fn probe_tiff_dimensions(path: &Path) -> CameraResult<(u32, u32, SensorType, Option<CfaPattern>)> {
    use tiff::decoder::Decoder;
    use tiff::ColorType;

    let file = fs::File::open(path)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to open TIFF: {}", e)))?;

    let mut decoder = Decoder::new(file)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to decode TIFF: {}", e)))?;

    let (width, height) = decoder
        .dimensions()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get dimensions: {}", e)))?;

    let color_type = decoder
        .colortype()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get color type: {}", e)))?;

    let (sensor_type, bayer_pattern) = match color_type {
        ColorType::Gray(_) => (SensorType::Color, Some(CfaPattern::Rggb)), // Assume Bayer
        ColorType::RGB(_) | ColorType::RGBA(_) => (SensorType::Color, None),
        _ => (SensorType::Mono, None),
    };

    Ok((width, height, sensor_type, bayer_pattern))
}

fn probe_fits_dimensions(path: &Path) -> CameraResult<(u32, u32, SensorType, Option<CfaPattern>)> {
    use fitsio::hdu::HduInfo;
    use fitsio::FitsFile;

    let path_str = path
        .to_str()
        .ok_or_else(|| CameraError::ImageReadFailed("Invalid path".to_string()))?;

    let mut fitsfile = FitsFile::open(path_str)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to open FITS: {}", e)))?;

    // Try all HDUs until we find one with image data
    let mut dimensions = None;
    for hdu_idx in 0..fitsfile
        .num_hdus()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get HDU count: {}", e)))?
    {
        if let Ok(hdu) = fitsfile.hdu(hdu_idx) {
            if let HduInfo::ImageInfo { shape, .. } = &hdu.info {
                if !shape.is_empty() {
                    dimensions = Some(shape.clone());
                    break;
                }
            }
        }
    }

    let (width, height, channels) = match dimensions {
        Some(shape) => match shape.as_slice() {
            [h, w] => (*w as u32, *h as u32, 1),
            [c, h, w] if *c == 3 => (*w as u32, *h as u32, 3),
            [h, w, c] if *c == 3 => (*w as u32, *h as u32, 3),
            _ => {
                return Err(CameraError::ImageReadFailed(format!(
                    "Unsupported FITS shape: {:?}",
                    shape
                )))
            }
        },
        _ => {
            return Err(CameraError::ImageReadFailed(
                "FITS file does not contain image data in any HDU".to_string(),
            ))
        }
    };

    let (sensor_type, bayer_pattern) = if channels == 1 {
        (SensorType::Color, Some(CfaPattern::Rggb)) // Assume Bayer for mono
    } else {
        (SensorType::Color, None)
    };

    Ok((width, height, sensor_type, bayer_pattern))
}

fn probe_png_dimensions(path: &Path) -> CameraResult<(u32, u32, SensorType, Option<CfaPattern>)> {
    use zune_png::zune_core::bytestream::ZCursor;
    use zune_png::zune_core::colorspace::ColorSpace;
    use zune_png::PngDecoder;

    let file_bytes = fs::read(path)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to read PNG: {}", e)))?;

    let mut decoder = PngDecoder::new(ZCursor::new(&file_bytes));
    decoder
        .decode_headers()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to read PNG info: {:?}", e)))?;

    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| CameraError::ImageReadFailed("Failed to get PNG dimensions".into()))?;

    let colorspace = decoder
        .colorspace()
        .ok_or_else(|| CameraError::ImageReadFailed("Failed to get PNG colorspace".into()))?;

    // Grayscale images from astronomy cameras are typically raw Bayer data
    // Assume RGGB pattern (most common). The debayer auto-detection will refine this.
    let (sensor_type, bayer_pattern) = match colorspace {
        ColorSpace::Luma | ColorSpace::LumaA => (SensorType::Color, Some(CfaPattern::Rggb)),
        _ => (SensorType::Color, None),
    };

    Ok((width as u32, height as u32, sensor_type, bayer_pattern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitsio::images::{ImageDescription, ImageType};
    use fitsio::FitsFile;
    use tempfile::tempdir;

    #[test]
    fn test_probe_fits_with_empty_primary() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_extension.fits");
        let path_str = path.to_str().unwrap();

        // Create FITS with empty primary and an image extension
        {
            let mut fptr = FitsFile::create(path_str).open().unwrap();
            let description = ImageDescription {
                data_type: ImageType::Short,
                dimensions: &[10, 20], // height, width
            };
            // Create image extension (name "EXT1")
            fptr.create_image("EXT1".to_string(), &description).unwrap();
        }

        let (width, height, sensor_type, bayer_pattern) = probe_fits_dimensions(&path).unwrap();
        assert_eq!(width, 20);
        assert_eq!(height, 10);
        assert_eq!(sensor_type, SensorType::Color); // Default for mono in probe
        assert_eq!(bayer_pattern, Some(CfaPattern::Rggb));
    }

    #[test]
    fn test_probe_fits_primary() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_primary.fits");
        let path_str = path.to_str().unwrap();

        // Create FITS with data in primary HDU
        {
            let mut fptr = FitsFile::create(path_str).open().unwrap();
            let description = ImageDescription {
                data_type: ImageType::Short,
                dimensions: &[30, 40], // height, width
            };
            // Empty name usually targets primary if it's the first one,
            // but fitsio behavior can vary. Let's use the manual writer logic
            // if we want to be sure, or just test if our probe handles it.
            fptr.create_image("".to_string(), &description).unwrap();
        }

        let (width, height, _, _) = probe_fits_dimensions(&path).unwrap();
        assert_eq!(width, 40);
        assert_eq!(height, 30);
    }
}
