//! Image loading utilities for integration tests.
//!
//! Handles loading TIFF and FITS files and converting them to Frames.

use std::fs;
use std::path::{Path, PathBuf};

use night_amplifier::{Frame, PixelFormat};

use crate::integration::common::{
    find_fixture_sets, LoadedImage, FITS_EXTENSIONS, PNG_EXTENSIONS, TIFF_EXTENSIONS,
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
// PNG Loading
// ============================================================================

/// Loads a PNG file and converts it to a Frame.
///
/// Several of the bundled fixture sets are 16-bit greyscale PNGs of undebayered
/// sensor output, the same thing the TIFF loader treats as CFA data. Without
/// this they are invisible to `find_fixture_sets`, so tests that iterate the
/// fixtures silently skip three of the four managed sets.
pub fn load_png(path: &Path) -> Result<LoadedImage, String> {
    let img = image::open(path).map_err(|e| format!("Failed to open PNG {:?}: {}", path, e))?;
    let (width, height) = (img.width() as usize, img.height() as usize);

    let (raw_bytes, format, channels, is_bayer) = match img {
        image::DynamicImage::ImageLuma16(g) => {
            let bytes: Vec<u8> = g.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect();
            (bytes, PixelFormat::Bayer16, 1, true)
        }
        image::DynamicImage::ImageLuma8(g) => (g.into_raw(), PixelFormat::Bayer8, 1, true),
        image::DynamicImage::ImageRgb8(rgb) => (rgb.into_raw(), PixelFormat::Rgb8, 3, false),
        image::DynamicImage::ImageRgb16(rgb) => {
            let bytes: Vec<u8> = rgb.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect();
            (bytes, PixelFormat::Rgb16, 3, false)
        }
        other => {
            let rgb = other.to_rgb8();
            (rgb.into_raw(), PixelFormat::Rgb8, 3, false)
        }
    };

    let frame = Frame::from_raw(&raw_bytes, width, height, channels, format)
        .map_err(|e| format!("Failed to create Frame from PNG {:?}: {}", path, e))?;

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

/// Loads a FITS file and converts it to a Frame.
///
/// Delegates to `night_amplifier::fits::read_frame`, the same reader the application
/// uses. The 100-line copy this replaced had to re-interleave a planar FITS before
/// handing it to `Frame::from_raw` — reinstating, in the harness, exactly the double
/// conversion the production reader was written to remove. A fixture loader that does
/// not agree with the application is not testing the application.
pub fn load_fits(path: &Path) -> Result<LoadedImage, String> {
    let frame = night_amplifier::fits::read_frame(path)
        .map_err(|e| format!("Failed to load FITS {:?}: {}", path, e))?;

    Ok(LoadedImage {
        width: frame.width(),
        height: frame.height(),
        // FITS greyscale from an astronomy camera is usually undebayered CFA data.
        is_bayer: frame.channels() == 1,
        frame,
        path: path.to_path_buf(),
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
    } else if PNG_EXTENSIONS.contains(&ext.as_str()) {
        load_png(path)
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
    let file = fs::File::create(output_path)
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

        // Every integration test that writes a fixture passes through here, which
        // makes this the cheapest place to catch a planar buffer being read as
        // interleaved: that collapses the channels toward grey.
        let spread = crate::integration::common::mean_chroma_spread_rgb8(
            &rgb8,
            frame.width(),
            frame.height(),
        );
        let frame_spread = crate::integration::common::mean_chroma_spread_frame(frame);
        if frame_spread > crate::integration::common::MIN_CHROMA_SPREAD {
            crate::integration::common::assert_has_chroma(
                spread,
                &format!(
                    "render_to_rgb8 for {:?} (source frame spread {frame_spread:.2})",
                    output_path
                ),
            );
        }

        encoder
            .write_image::<RGB8>(frame.width() as u32, frame.height() as u32, &rgb8)
            .map_err(|e| format!("Failed to write RGB TIFF image: {}", e))?;
    }

    Ok(output_path.to_path_buf())
}
