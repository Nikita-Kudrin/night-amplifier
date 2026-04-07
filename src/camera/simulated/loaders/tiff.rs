//! TIFF image format loader
//! Optimized to use pre-allocated capacities to prevent vector reallocation

use std::fs;
use std::path::Path;

use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::ColorType;

use crate::camera::error::{CameraError, CameraResult};
use crate::{Frame, PixelFormat};

pub fn load_tiff(path: &Path) -> CameraResult<Frame> {
    let file = fs::File::open(path)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to open TIFF: {}", e)))?;

    let mut decoder = Decoder::new(file)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to decode TIFF: {}", e)))?
        .with_limits(Limits::unlimited());

    let (width, height) = decoder
        .dimensions()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get dimensions: {}", e)))?;

    let width = width as usize;
    let height = height as usize;

    let color_type = decoder
        .colortype()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to get color type: {}", e)))?;

    let image_data = decoder
        .read_image()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to read image: {}", e)))?;

    let (raw_bytes, format, channels) = convert_tiff_data(color_type, image_data)?;

    Frame::from_raw(&raw_bytes, width, height, channels, format)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to create frame: {}", e)))
}

fn convert_tiff_data(
    color_type: ColorType,
    image_data: DecodingResult,
) -> CameraResult<(Vec<u8>, PixelFormat, usize)> {
    match (color_type, image_data) {
        (ColorType::Gray(8), DecodingResult::U8(data)) => Ok((data, PixelFormat::Bayer8, 1)),
        (ColorType::Gray(16), DecodingResult::U16(data)) => {
            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            Ok((bytes, PixelFormat::Bayer16, 1))
        }
        (ColorType::RGB(8), DecodingResult::U8(data)) => Ok((data, PixelFormat::Rgb8, 3)),
        (ColorType::RGB(16), DecodingResult::U16(data)) => {
            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            Ok((bytes, PixelFormat::Rgb16, 3))
        }
        (ColorType::RGBA(8), DecodingResult::U8(data)) => {
            let expected_len = (data.len() / 4) * 3;
            let mut rgb = Vec::with_capacity(expected_len);
            for chunk in data.chunks_exact(4) {
                rgb.extend_from_slice(&chunk[0..3]);
            }
            Ok((rgb, PixelFormat::Rgb8, 3))
        }
        (ColorType::RGBA(16), DecodingResult::U16(data)) => {
            let expected_len = (data.len() / 4) * 3 * 2;
            let mut rgb = Vec::with_capacity(expected_len);
            for chunk in data.chunks_exact(4) {
                for &v in &chunk[0..3] {
                    rgb.extend_from_slice(&v.to_le_bytes());
                }
            }
            Ok((rgb, PixelFormat::Rgb16, 3))
        }
        // 32-bit float RGB - convert to 16-bit by scaling from [0.0, 1.0] range
        (ColorType::RGB(32), DecodingResult::F32(data)) => {
            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                let scaled = (v.clamp(0.0, 1.0) * 65535.0) as u16;
                bytes.extend_from_slice(&scaled.to_le_bytes());
            }
            Ok((bytes, PixelFormat::Rgb16, 3))
        }
        // 32-bit float grayscale - convert to 16-bit
        (ColorType::Gray(32), DecodingResult::F32(data)) => {
            let mut bytes = Vec::with_capacity(data.len() * 2);
            for &v in &data {
                let scaled = (v.clamp(0.0, 1.0) * 65535.0) as u16;
                bytes.extend_from_slice(&scaled.to_le_bytes());
            }
            Ok((bytes, PixelFormat::Bayer16, 1))
        }
        // 32-bit float RGBA - strip alpha and convert to 16-bit
        (ColorType::RGBA(32), DecodingResult::F32(data)) => {
            let pixel_count = data.len() / 4;
            let mut bytes = Vec::with_capacity(pixel_count * 3 * 2);
            for chunk in data.chunks_exact(4) {
                for &v in &chunk[0..3] {
                    let scaled = (v.clamp(0.0, 1.0) * 65535.0) as u16;
                    bytes.extend_from_slice(&scaled.to_le_bytes());
                }
            }
            Ok((bytes, PixelFormat::Rgb16, 3))
        }
        (ct, _) => Err(CameraError::ImageReadFailed(format!(
            "Unsupported TIFF color type: {:?}",
            ct
        ))),
    }
}
