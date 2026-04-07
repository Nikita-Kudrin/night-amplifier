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
use crate::{Frame, PixelFormat};
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

    let (width, height, channels, image_type) = extract_fits_info(&hdu.info)?;

    let fits_data = read_fits_data(&hdu, &mut fitsfile, image_type, width, height, channels)?;

    match fits_data {
        FitsData::Bytes(raw_bytes, format) => {
            Frame::from_raw(&raw_bytes, width, height, channels, format)
                .map_err(|e| CameraError::ImageReadFailed(format!("Failed to create frame: {}", e)))
        }
        FitsData::Frame(frame) => Ok(frame),
    }
}

enum FitsData {
    Bytes(Vec<u8>, PixelFormat),
    Frame(Frame),
}

fn extract_fits_info(info: &HduInfo) -> CameraResult<(usize, usize, usize, ImageType)> {
    match info {
        HduInfo::ImageInfo { shape, image_type } => {
            let (w, h, c) = match shape.as_slice() {
                [h, w] => (*w, *h, 1),
                [c, h, w] if *c == 3 => (*w, *h, 3),
                [h, w, c] if *c == 3 => (*w, *h, 3),
                _ => {
                    return Err(CameraError::ImageReadFailed(format!(
                        "Unsupported FITS shape: {:?}",
                        shape
                    )))
                }
            };
            Ok((w, h, c, image_type.clone()))
        }
        _ => Err(CameraError::ImageReadFailed(
            "FITS file does not contain image data".to_string(),
        )),
    }
}

fn read_fits_data(
    hdu: &fitsio::hdu::FitsHdu,
    fitsfile: &mut FitsFile,
    image_type: ImageType,
    width: usize,
    height: usize,
    channels: usize,
) -> CameraResult<FitsData> {
    match image_type {
        ImageType::UnsignedByte => {
            let data: Vec<u8> =
                catch_ffi_panic("cfitsio::read_image_u8", || hdu.read_image(fitsfile))
                    .map_err(CameraError::from)?
                    .map_err(|e| {
                        CameraError::ImageReadFailed(format!("Failed to read data: {}", e))
                    })?;

            let data = if channels > 1 {
                interleave_planar(&data, width, height, channels)
            } else {
                data
            };

            Ok(FitsData::Bytes(data, PixelFormat::Rgb8))
        }
        ImageType::Short => {
            let data: Vec<i16> =
                catch_ffi_panic("cfitsio::read_image_i16", || hdu.read_image(fitsfile))
                    .map_err(CameraError::from)?
                    .map_err(|e| {
                        CameraError::ImageReadFailed(format!("Failed to read data: {}", e))
                    })?;

            let data = if channels > 1 {
                interleave_planar(&data, width, height, channels)
            } else {
                data
            };

            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                let u = (v as i32 + 32768) as u16;
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            Ok(FitsData::Bytes(bytes, PixelFormat::Rgb16))
        }
        ImageType::UnsignedShort => {
            let data: Vec<u16> =
                catch_ffi_panic("cfitsio::read_image_u16", || hdu.read_image(fitsfile))
                    .map_err(CameraError::from)?
                    .map_err(|e| {
                        CameraError::ImageReadFailed(format!("Failed to read data: {}", e))
                    })?;

            let data = if channels > 1 {
                interleave_planar(&data, width, height, channels)
            } else {
                data
            };

            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            Ok(FitsData::Bytes(bytes, PixelFormat::Rgb16))
        }
        ImageType::Float => {
            let mut data: Vec<f32> =
                catch_ffi_panic("cfitsio::read_image_f32", || hdu.read_image(fitsfile))
                    .map_err(CameraError::from)?
                    .map_err(|e| {
                        CameraError::ImageReadFailed(format!("Failed to read data: {}", e))
                    })?;

            normalize_f32_in_place(&mut data);

            let data = if channels > 1 {
                interleave_planar(&data, width, height, channels)
            } else {
                data
            };

            // Zero-copy conversion directly into Frame
            let frame = Frame::from_f32_vec(data, width, height, channels).map_err(|e| {
                CameraError::ImageReadFailed(format!("Failed to create frame: {}", e))
            })?;
            Ok(FitsData::Frame(frame))
        }
        ImageType::Double => {
            let f64_data: Vec<f64> =
                catch_ffi_panic("cfitsio::read_image_f64", || hdu.read_image(fitsfile))
                    .map_err(CameraError::from)?
                    .map_err(|e| {
                        CameraError::ImageReadFailed(format!("Failed to read data: {}", e))
                    })?;

            let data = normalize_f64_to_f32(&f64_data);

            let data = if channels > 1 {
                interleave_planar(&data, width, height, channels)
            } else {
                data
            };

            let frame = Frame::from_f32_vec(data, width, height, channels).map_err(|e| {
                CameraError::ImageReadFailed(format!("Failed to create frame: {}", e))
            })?;
            Ok(FitsData::Frame(frame))
        }
        _ => Err(CameraError::ImageReadFailed(format!(
            "Unsupported FITS image type: {:?}",
            image_type
        ))),
    }
}

/// Normalizes f32 data to [0.0, 1.0] in place using a single min/max pass.
fn normalize_f32_in_place(data: &mut [f32]) {
    // 1-pass min/max
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for &v in data.iter() {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }

    let range = (max - min).max(1e-10);
    let inv_range = 1.0 / range;
    for v in data.iter_mut() {
        *v = (*v - min) * inv_range;
    }
}

/// Converts f64 data to normalized f32 [0.0, 1.0] in a single pass.
fn normalize_f64_to_f32(data: &[f64]) -> Vec<f32> {
    let (min, max) = data.iter().fold((f64::MAX, f64::MIN), |(cmin, cmax), &v| {
        (cmin.min(v), cmax.max(v))
    });

    let range = (max - min).max(1e-10);
    let inv_range = 1.0 / range;

    let mut f32_data = Vec::with_capacity(data.len());
    for &v in data {
        f32_data.push(((v - min) * inv_range) as f32);
    }
    f32_data
}

/// Converts planar data [R..., G..., B...] to interleaved [RGB, RGB, ...] in parallel.
fn interleave_planar<T>(data: &[T], width: usize, height: usize, channels: usize) -> Vec<T>
where
    T: Copy + Send + Sync + Default,
{
    let mut interleaved = vec![T::default(); data.len()];
    let plane_size = width * height;

    interleaved
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..width {
                let pixel_offset = x * channels;
                let plane_offset = y * width + x;
                for c in 0..channels {
                    row[pixel_offset + c] = data[c * plane_size + plane_offset];
                }
            }
        });

    interleaved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interleave_planar() {
        // 2x2 RGB image in planar format: [R1, R2, R3, R4, G1, G2, G3, G4, B1, B2, B3, B4]
        let planar = vec![
            1.0, 2.0, 3.0, 4.0, // Red
            5.0, 6.0, 7.0, 8.0, // Green
            9.0, 10.0, 11.0, 12.0, // Blue
        ];

        let interleaved = interleave_planar(&planar, 2, 2, 3);

        // Expected interleaved: [R1, G1, B1, R2, G2, B2, R3, G3, B3, R4, G4, B4]
        let expected = vec![
            1.0, 5.0, 9.0, // Pixel (0,0)
            2.0, 6.0, 10.0, // Pixel (1,0)
            3.0, 7.0, 11.0, // Pixel (0,1)
            4.0, 8.0, 12.0, // Pixel (1,1)
        ];

        assert_eq!(interleaved, expected);
    }
}
