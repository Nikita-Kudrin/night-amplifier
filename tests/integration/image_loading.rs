//! Image loading utilities for integration tests.
//!
//! Handles loading TIFF and FITS files and converting them to Frames.

use std::fs;
use std::path::{Path, PathBuf};

use night_amplifier::{Frame, PixelFormat};

use crate::integration::common::{
    find_fixture_sets, LoadedImage, FITS_EXTENSIONS, TIFF_EXTENSIONS,
};

// ============================================================================
// TIFF Loading
// ============================================================================

/// Loads a TIFF file and converts it to a Frame
///
/// For grayscale 16-bit images, this function treats them as potential Bayer data
/// from astronomy cameras (I;16 format from OpenLiveStacker, etc.).
pub fn load_tiff(path: &Path) -> Result<LoadedImage, String> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::ColorType;

    let file =
        fs::File::open(path).map_err(|e| format!("Failed to open TIFF file {:?}: {}", path, e))?;

    let mut decoder = Decoder::new(file)
        .map_err(|e| format!("Failed to create TIFF decoder for {:?}: {}", path, e))?;

    let (width, height) = decoder
        .dimensions()
        .map_err(|e| format!("Failed to get TIFF dimensions for {:?}: {}", path, e))?;

    let width = width as usize;
    let height = height as usize;

    let color_type = decoder
        .colortype()
        .map_err(|e| format!("Failed to get TIFF color type for {:?}: {}", path, e))?;

    let image_data = decoder
        .read_image()
        .map_err(|e| format!("Failed to read TIFF image data from {:?}: {}", path, e))?;

    let (raw_bytes, format, channels, is_bayer) = match (color_type, image_data) {
        // 8-bit grayscale - could be Bayer data
        (ColorType::Gray(8), DecodingResult::U8(data)) => (data, PixelFormat::Bayer8, 1, true),
        // 16-bit grayscale - likely Bayer data from astronomy cameras (I;16 format)
        (ColorType::Gray(16), DecodingResult::U16(data)) => {
            let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_le_bytes()).collect();
            (bytes, PixelFormat::Bayer16, 1, true)
        }
        // 8-bit RGB - already debayered
        (ColorType::RGB(8), DecodingResult::U8(data)) => (data, PixelFormat::Rgb8, 3, false),
        // 16-bit RGB - already debayered
        (ColorType::RGB(16), DecodingResult::U16(data)) => {
            let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_le_bytes()).collect();
            (bytes, PixelFormat::Rgb16, 3, false)
        }
        // 8-bit RGBA (drop alpha)
        (ColorType::RGBA(8), DecodingResult::U8(data)) => {
            let rgb: Vec<u8> = data
                .chunks(4)
                .flat_map(|rgba| &rgba[0..3])
                .copied()
                .collect();
            (rgb, PixelFormat::Rgb8, 3, false)
        }
        // 16-bit RGBA (drop alpha)
        (ColorType::RGBA(16), DecodingResult::U16(data)) => {
            let rgb: Vec<u8> = data
                .chunks(4)
                .flat_map(|rgba| rgba[0..3].iter().flat_map(|&v| v.to_le_bytes()))
                .collect();
            (rgb, PixelFormat::Rgb16, 3, false)
        }
        (ct, _) => {
            return Err(format!(
                "Unsupported TIFF color type {:?} in {:?}",
                ct, path
            ));
        }
    };

    let frame = Frame::from_raw(&raw_bytes, width, height, channels, format)
        .map_err(|e| format!("Failed to create Frame from TIFF {:?}: {}", path, e))?;

    Ok(LoadedImage {
        frame,
        path: path.to_path_buf(),
        width,
        height,
        is_bayer,
    })
}

// ============================================================================
// FITS Loading
// ============================================================================

/// Loads a FITS file and converts it to a Frame
pub fn load_fits(path: &Path) -> Result<LoadedImage, String> {
    use fitsio::hdu::HduInfo;
    use fitsio::images::ImageType;
    use fitsio::FitsFile;

    let path_str = path
        .to_str()
        .ok_or_else(|| format!("Invalid path: {:?}", path))?;

    let mut fitsfile = FitsFile::open(path_str)
        .map_err(|e| format!("Failed to open FITS file {:?}: {}", path, e))?;

    let hdu = fitsfile
        .primary_hdu()
        .map_err(|e| format!("Failed to get primary HDU from {:?}: {}", path, e))?;

    let (width, height, channels, image_type) = match &hdu.info {
        HduInfo::ImageInfo { shape, image_type } => {
            let (w, h, c) = match shape.as_slice() {
                // 2D image (grayscale): [height, width]
                [h, w] => (*w, *h, 1),
                // 3D image (color): [channels, height, width] or [height, width, channels]
                [c, h, w] if *c == 3 => (*w, *h, 3),
                [h, w, c] if *c == 3 => (*w, *h, 3),
                _ => return Err(format!("Unsupported FITS shape {:?} in {:?}", shape, path)),
            };
            (w, h, c, image_type.clone())
        }
        _ => return Err(format!("FITS file {:?} does not contain image data", path)),
    };

    let (raw_bytes, format) = match image_type {
        ImageType::UnsignedByte => {
            let data: Vec<u8> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read u8 FITS data from {:?}: {}", path, e))?;
            (data, PixelFormat::Rgb8)
        }
        ImageType::Short => {
            let data: Vec<i16> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read i16 FITS data from {:?}: {}", path, e))?;
            // Convert to u16 by adding offset
            let bytes: Vec<u8> = data
                .iter()
                .map(|&v| (v as i32 + 32768) as u16)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            (bytes, PixelFormat::Rgb16)
        }
        ImageType::UnsignedShort => {
            let data: Vec<u16> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read u16 FITS data from {:?}: {}", path, e))?;
            let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_le_bytes()).collect();
            (bytes, PixelFormat::Rgb16)
        }
        ImageType::Long => {
            let data: Vec<i32> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read i32 FITS data from {:?}: {}", path, e))?;
            // Scale to 16-bit
            let max_val = data.iter().map(|&v| v.abs()).max().unwrap_or(1) as f64;
            let bytes: Vec<u8> = data
                .iter()
                .map(|&v| ((v as f64 / max_val * 32767.0 + 32768.0) as u16).min(65535))
                .flat_map(|v| v.to_le_bytes())
                .collect();
            (bytes, PixelFormat::Rgb16)
        }
        ImageType::Float => {
            let data: Vec<f32> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read f32 FITS data from {:?}: {}", path, e))?;
            // Normalize and convert to 16-bit
            let min = data.iter().cloned().fold(f32::MAX, f32::min);
            let max = data.iter().cloned().fold(f32::MIN, f32::max);
            let range = (max - min).max(1e-10);
            let bytes: Vec<u8> = data
                .iter()
                .map(|&v| (((v - min) / range) * 65535.0) as u16)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            (bytes, PixelFormat::Rgb16)
        }
        ImageType::Double => {
            let data: Vec<f64> = hdu
                .read_image(&mut fitsfile)
                .map_err(|e| format!("Failed to read f64 FITS data from {:?}: {}", path, e))?;
            // Normalize and convert to 16-bit
            let min = data.iter().cloned().fold(f64::MAX, f64::min);
            let max = data.iter().cloned().fold(f64::MIN, f64::max);
            let range = (max - min).max(1e-10);
            let bytes: Vec<u8> = data
                .iter()
                .map(|&v| (((v - min) / range) * 65535.0) as u16)
                .flat_map(|v| v.to_le_bytes())
                .collect();
            (bytes, PixelFormat::Rgb16)
        }
        _ => {
            return Err(format!(
                "Unsupported FITS image type {:?} in {:?}",
                image_type, path
            ))
        }
    };

    // FITS grayscale data is likely Bayer from astronomy cameras
    let is_bayer = channels == 1;

    let frame = Frame::from_raw(&raw_bytes, width, height, channels, format)
        .map_err(|e| format!("Failed to create Frame from FITS {:?}: {}", path, e))?;

    Ok(LoadedImage {
        frame,
        path: path.to_path_buf(),
        width,
        height,
        is_bayer,
    })
}

// ============================================================================
// Generic Loading
// ============================================================================

/// Loads an image file (TIFF or FITS) into a Frame
pub fn load_image(path: &Path) -> Result<LoadedImage, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if TIFF_EXTENSIONS.contains(&ext.as_str()) {
        load_tiff(path)
    } else if FITS_EXTENSIONS.contains(&ext.as_str()) {
        load_fits(path)
    } else {
        Err(format!("Unsupported file extension: {}", ext))
    }
}

/// Loads all images from a list of paths
pub fn load_images_from_paths(paths: &[PathBuf]) -> Vec<LoadedImage> {
    paths
        .iter()
        .filter_map(|path| match load_image(path) {
            Ok(img) => {
                let bayer_str = if img.is_bayer { " [Bayer]" } else { "" };
                println!(
                    "  Loaded: {:?} ({}x{}, {} ch{}){}",
                    path.file_name().unwrap_or_default(),
                    img.width,
                    img.height,
                    img.frame.channels(),
                    if img.frame.channels() == 1 { "" } else { "s" },
                    bayer_str
                );
                Some(img)
            }
            Err(e) => {
                eprintln!("  Warning: {}", e);
                None
            }
        })
        .collect()
}

/// Loads all images from all fixture subdirectories
pub fn load_all_fixture_images() -> Vec<LoadedImage> {
    let fixture_sets = find_fixture_sets();
    let all_files: Vec<PathBuf> = fixture_sets.into_iter().flat_map(|set| set.files).collect();
    load_images_from_paths(&all_files)
}

// ============================================================================
// Saving
// ============================================================================

/// Saves processed frame as TIFF to a specific directory
pub fn save_processed_frame_to_dir(
    frame: &Frame,
    output_dir: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    // Build output path
    let output_path = output_dir.join(format!("{}.tiff", name));

    save_frame_to_path(frame, &output_path)
}

/// Saves a frame to a specific path
fn save_frame_to_path(frame: &Frame, output_path: &Path) -> Result<PathBuf, String> {
    use night_amplifier::render_to_rgb8;
    use tiff::encoder::{
        colortype::{Gray8, RGB8},
        TiffEncoder,
    };

    // Create TIFF file
    let file = fs::File::create(&output_path)
        .map_err(|e| format!("Failed to create output file {:?}: {}", output_path, e))?;

    let mut encoder =
        TiffEncoder::new(file).map_err(|e| format!("Failed to create TIFF encoder: {}", e))?;

    if frame.channels() == 1 {
        // Grayscale: convert f32 [0,1] to u8
        let gray8: Vec<u8> = frame
            .data()
            .iter()
            .map(|&v| (v.clamp(0.0, 1.0) * 255.0) as u8)
            .collect();

        encoder
            .write_image::<Gray8>(frame.width() as u32, frame.height() as u32, &gray8)
            .map_err(|e| format!("Failed to write grayscale TIFF image: {}", e))?;
    } else {
        // RGB: use renderer
        let rgb8 = render_to_rgb8(frame).map_err(|e| format!("Failed to render frame: {}", e))?;

        encoder
            .write_image::<RGB8>(frame.width() as u32, frame.height() as u32, &rgb8)
            .map_err(|e| format!("Failed to write RGB TIFF image: {}", e))?;
    }

    Ok(output_path.to_path_buf())
}
