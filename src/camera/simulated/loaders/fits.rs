//! FITS image format loader
//!
//! Optimized to leverage zero-copy f32 frame construction for Float formats
//! and pre-allocated capacity for integer types.

use std::path::Path;

use fitsio::hdu::HduInfo;
use fitsio::images::ImageType;
use fitsio::FitsFile;

use crate::camera::error::{CameraError, CameraResult};
use crate::ffi_safety::catch_ffi_panic;
use crate::Frame;
use rayon::prelude::*;

pub fn load_fits(path: &Path) -> CameraResult<Frame> {
    let path_str = path
        .to_str()
        .ok_or_else(|| CameraError::ImageReadFailed("Invalid path".to_string()))?;

    let path_owned = path_str.to_string();
    let mut fitsfile = catch_ffi_panic("cfitsio::open", || FitsFile::open(&path_owned))
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to open FITS: {}", e)))?;

    let mut valid_hdu_idx = None;
    for hdu_idx in 0..fitsfile
        .num_hdus()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get HDU count: {}", e)))?
    {
        if let Ok(hdu) = fitsfile.hdu(hdu_idx) {
            if let HduInfo::ImageInfo { shape, .. } = &hdu.info {
                if !shape.is_empty() {
                    valid_hdu_idx = Some(hdu_idx);
                    break;
                }
            }
        }
    }

    let hdu_idx = valid_hdu_idx.ok_or_else(|| {
        CameraError::ImageReadFailed("FITS file does not contain image data in any HDU".to_string())
    })?;

    let hdu = fitsfile.hdu(hdu_idx).map_err(|e| {
        CameraError::ImageReadFailed(format!("Failed to re-open HDU {}: {}", hdu_idx, e))
    })?;

    let (shape, image_type) = extract_fits_info(&hdu.info)?;

    read_fits_data(&hdu, &mut fitsfile, image_type, shape)
}

fn extract_fits_info(info: &HduInfo) -> CameraResult<(crate::fits::FitsShape, ImageType)> {
    match info {
        HduInfo::ImageInfo { shape, image_type } => {
            let parsed = crate::fits::interpret_shape(shape).ok_or_else(|| {
                CameraError::ImageReadFailed(format!("Unsupported FITS shape: {:?}", shape))
            })?;
            Ok((parsed, *image_type))
        }
        _ => Err(CameraError::ImageReadFailed(
            "FITS file does not contain image data".to_string(),
        )),
    }
}

/// Reads the HDU payload and normalises it into a planar [`Frame`].
///
/// FITS colour is plane-major and so is `Frame`, so a [`FitsColourLayout::Planar`]
/// source needs no reordering at all. Only the interleaved form (NAXIS1 = 3) is
/// scattered across planes.
///
/// The integer arms used to route through `FitsData::Bytes` -> `Frame::from_raw`, which
/// de-interleaves — so a planar source had to be interleaved first, and the two
/// conversions then cancelled. Correct, but two full-buffer passes and two extra
/// allocations per loaded frame to arrive where the data already was.
fn read_fits_data(
    hdu: &fitsio::hdu::FitsHdu,
    fitsfile: &mut FitsFile,
    image_type: ImageType,
    shape: crate::fits::FitsShape,
) -> CameraResult<Frame> {
    macro_rules! read {
        ($label:literal, $ty:ty) => {{
            let data: Vec<$ty> = catch_ffi_panic($label, || hdu.read_image(fitsfile))
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::ImageReadFailed(format!("Failed to read data: {}", e)))?;
            data
        }};
    }

    match image_type {
        ImageType::UnsignedByte => {
            let data = read!("cfitsio::read_image_u8", u8);
            planar_frame(&data, shape, |v| v as f32 * (1.0 / 255.0))
        }
        ImageType::Short => {
            let data = read!("cfitsio::read_image_i16", i16);
            planar_frame(&data, shape, |v| {
                (v as i32 + 32768) as f32 * (1.0 / 65535.0)
            })
        }
        ImageType::UnsignedShort => {
            let data = read!("cfitsio::read_image_u16", u16);
            planar_frame(&data, shape, |v| v as f32 * (1.0 / 65535.0))
        }
        ImageType::Long => {
            let data = read!("cfitsio::read_image_i32", i32);
            // Scaled against the data's own magnitude: FITS integers on this axis carry
            // no declared full-well. Kept bit-for-bit equivalent to the previous
            // `(v / max * 32767 + 32768) / 65535`, which mapped signed input onto the
            // upper half of the u16 range. `unsigned_abs` rather than `abs` because
            // `i32::MIN.abs()` overflows, and `.max(1)` because all-zero data used to
            // divide by zero.
            let max_val = data
                .iter()
                .map(|&v| v.unsigned_abs())
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            let inv = 1.0 / max_val;
            planar_frame(&data, shape, move |v| {
                (v as f32 * inv * 32767.0 + 32768.0) * (1.0 / 65535.0)
            })
        }
        ImageType::Float => {
            let data = read!("cfitsio::read_image_f32", f32);
            let (min, max) = data
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let inv_range = 1.0 / (max - min).max(1e-10);
            planar_frame(&data, shape, move |v| (v - min) * inv_range)
        }
        ImageType::Double => {
            let data = read!("cfitsio::read_image_f64", f64);
            // Span accumulated in f64, as before: narrowing first would lose the range
            // on high-dynamic-range doubles.
            let (min, max) = data
                .iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let inv_range = 1.0 / (max - min).max(1e-10);
            planar_frame(&data, shape, move |v| ((v - min) * inv_range) as f32)
        }
        _ => Err(CameraError::ImageReadFailed(format!(
            "Unsupported FITS image type: {:?}",
            image_type
        ))),
    }
}

/// Scatters `samples` into a planar [`Frame`], normalising with `to_norm`.
///
/// One pass, one allocation, whichever layout the source uses.
fn planar_frame<T: Copy + Send + Sync>(
    samples: &[T],
    shape: crate::fits::FitsShape,
    to_norm: impl Fn(T) -> f32 + Send + Sync,
) -> CameraResult<Frame> {
    use crate::fits::FitsColourLayout;

    let crate::fits::FitsShape {
        width,
        height,
        channels,
        layout,
    } = shape;
    let area = width * height;
    let expected = area * channels;

    if samples.len() < expected {
        return Err(CameraError::ImageReadFailed(format!(
            "FITS payload holds {} samples, expected {} for {}x{}x{}",
            samples.len(),
            expected,
            width,
            height,
            channels
        )));
    }

    let mut data = vec![0.0f32; expected];
    match layout {
        // NAXIS1 = 3: channels are the fastest-varying axis, so this is the one layout
        // that has to be scattered.
        FitsColourLayout::Interleaved => {
            data.par_chunks_mut(area)
                .enumerate()
                .for_each(|(c, plane)| {
                    for (i, slot) in plane.iter_mut().enumerate() {
                        *slot = to_norm(samples[i * channels + c]);
                    }
                });
        }
        // Already plane-major, which is exactly what `Frame` wants.
        FitsColourLayout::Mono | FitsColourLayout::Planar => {
            data.par_chunks_mut(area)
                .zip(samples[..expected].par_chunks(area))
                .for_each(|(plane, src)| {
                    for (slot, &v) in plane.iter_mut().zip(src) {
                        *slot = to_norm(v);
                    }
                });
        }
    }

    Frame::from_f32_vec(data, width, height, channels)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to create frame: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fits::{FitsColourLayout, FitsShape};

    fn shape(layout: FitsColourLayout) -> FitsShape {
        FitsShape {
            width: 2,
            height: 2,
            channels: 3,
            layout,
        }
    }

    /// A NAXIS3 = 3 payload is already plane-major, so it must pass through untouched.
    #[test]
    fn planar_payload_passes_through() {
        let planar: Vec<u16> = vec![
            1, 2, 3, 4, // Red
            5, 6, 7, 8, // Green
            9, 10, 11, 12, // Blue
        ];

        let frame = planar_frame(&planar, shape(FitsColourLayout::Planar), |v| v as f32).unwrap();

        assert_eq!(frame.get_pixel(0, 0, 0), 1.0);
        assert_eq!(frame.get_pixel(1, 0, 0), 2.0);
        assert_eq!(frame.get_pixel(0, 0, 1), 5.0);
        assert_eq!(frame.get_pixel(1, 1, 2), 12.0);
    }

    /// A NAXIS1 = 3 payload is interleaved and must be scattered across the planes.
    #[test]
    fn interleaved_payload_is_scattered_into_planes() {
        let interleaved: Vec<u16> = vec![
            1, 5, 9, // Pixel (0,0)
            2, 6, 10, // Pixel (1,0)
            3, 7, 11, // Pixel (0,1)
            4, 8, 12, // Pixel (1,1)
        ];

        let frame = planar_frame(&interleaved, shape(FitsColourLayout::Interleaved), |v| {
            v as f32
        })
        .unwrap();

        assert_eq!(frame.get_pixel(0, 0, 0), 1.0);
        assert_eq!(frame.get_pixel(1, 0, 0), 2.0);
        assert_eq!(frame.get_pixel(0, 0, 1), 5.0);
        assert_eq!(frame.get_pixel(1, 1, 2), 12.0);
    }

    /// A truncated payload must be reported, not silently read past.
    #[test]
    fn short_payload_is_rejected() {
        let too_short: Vec<u16> = vec![1, 2, 3];
        assert!(planar_frame(&too_short, shape(FitsColourLayout::Planar), |v| v as f32).is_err());
    }

    #[test]
    fn test_load_nonexistent_fits() {
        let result = load_fits(Path::new("this_file_does_not_exist_xyz.fits"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CameraError::ImageReadFailed(_)
        ));
    }

    #[test]
    fn test_load_corrupted_fits() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("test_corrupted_data.fits");
        {
            let mut file = std::fs::File::create(&path).unwrap();
            // Write a tiny incomplete header
            file.write_all(b"SIMPLE  =                    T").unwrap();
        }

        let result = load_fits(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(path);
    }
}
