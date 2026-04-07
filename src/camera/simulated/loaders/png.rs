//! PNG image format loader
//!
//! Uses zune-png for SIMD-accelerated DEFLATE decompression and scanline
//! unfiltering. Zero-copy for 8-bit paths, big-endian passthrough for 16-bit.

use std::fs;
use std::path::Path;
use tracing::instrument;
use zune_png::zune_core::bit_depth::BitDepth;
use zune_png::zune_core::bytestream::ZCursor;
use zune_png::zune_core::colorspace::ColorSpace;
use zune_png::PngDecoder;

use crate::camera::error::{CameraError, CameraResult};
use crate::{Frame, PixelFormat};

#[instrument(skip(path), fields(path = %path.display()))]
pub fn load_png(path: &Path) -> CameraResult<Frame> {
    let file_bytes = fs::read(path)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to read PNG file: {}", e)))?;

    let mut decoder = PngDecoder::new(ZCursor::new(&file_bytes));
    decoder.decode_headers().map_err(|e| {
        CameraError::ImageReadFailed(format!("Failed to decode PNG headers: {:?}", e))
    })?;

    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| CameraError::ImageReadFailed("Failed to get PNG dimensions".into()))?;

    let colorspace = decoder
        .colorspace()
        .ok_or_else(|| CameraError::ImageReadFailed("Failed to get PNG colorspace".into()))?;

    let depth = decoder
        .depth()
        .ok_or_else(|| CameraError::ImageReadFailed("Failed to get PNG bit depth".into()))?;

    let raw_bytes = decoder
        .decode_raw()
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to decode PNG: {:?}", e)))?;

    let (data, format, channels) = convert_png_data(raw_bytes, colorspace, depth)?;

    Frame::from_raw(&data, width, height, channels, format)
        .map_err(|e| CameraError::ImageReadFailed(format!("Failed to create frame: {}", e)))
}

#[instrument(skip_all, fields(colorspace = ?colorspace, depth = ?depth))]
fn convert_png_data(
    buf: Vec<u8>,
    colorspace: ColorSpace,
    depth: BitDepth,
) -> CameraResult<(Vec<u8>, PixelFormat, usize)> {
    match (colorspace, depth) {
        // 8-bit paths: zero-copy
        (ColorSpace::Luma, BitDepth::Eight) => Ok((buf, PixelFormat::Bayer8, 1)),
        (ColorSpace::RGB, BitDepth::Eight) => Ok((buf, PixelFormat::Rgb8, 3)),

        // 16-bit paths: PNG is natively big-endian, pass through as BE format
        (ColorSpace::Luma, BitDepth::Sixteen) => Ok((buf, PixelFormat::Bayer16Be, 1)),
        (ColorSpace::RGB, BitDepth::Sixteen) => Ok((buf, PixelFormat::Rgb16Be, 3)),

        // Alpha-stripping paths
        (ColorSpace::RGBA, BitDepth::Eight) => {
            let rgb = strip_alpha_8(&buf);
            Ok((rgb, PixelFormat::Rgb8, 3))
        }
        (ColorSpace::RGBA, BitDepth::Sixteen) => {
            let rgb = strip_alpha_16(&buf);
            Ok((rgb, PixelFormat::Rgb16Be, 3))
        }
        (ColorSpace::LumaA, BitDepth::Eight) => {
            let gray = strip_alpha_grayscale_8(&buf);
            Ok((gray, PixelFormat::Bayer8, 1))
        }
        (ColorSpace::LumaA, BitDepth::Sixteen) => {
            let gray = strip_alpha_grayscale_16(&buf);
            Ok((gray, PixelFormat::Bayer16Be, 1))
        }
        (cs, bd) => Err(CameraError::ImageReadFailed(format!(
            "Unsupported PNG format: {:?} {:?}",
            cs, bd
        ))),
    }
}

/// Strip alpha from 8-bit RGBA → RGB with pre-allocated capacity
fn strip_alpha_8(buf: &[u8]) -> Vec<u8> {
    let pixel_count = buf.len() / 4;
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in buf.chunks_exact(4) {
        rgb.extend_from_slice(&chunk[..3]);
    }
    rgb
}

/// Strip alpha from 16-bit RGBA, keeping native big-endian byte order
fn strip_alpha_16(buf: &[u8]) -> Vec<u8> {
    let pixel_count = buf.len() / 8; // 4 channels × 2 bytes
    let mut rgb = Vec::with_capacity(pixel_count * 6); // 3 channels × 2 bytes
    for chunk in buf.chunks_exact(8) {
        rgb.extend_from_slice(&chunk[..6]); // R(2) + G(2) + B(2), skip A(2)
    }
    rgb
}

/// Strip alpha from 8-bit GrayscaleAlpha → Grayscale
fn strip_alpha_grayscale_8(buf: &[u8]) -> Vec<u8> {
    let pixel_count = buf.len() / 2;
    let mut gray = Vec::with_capacity(pixel_count);
    for chunk in buf.chunks_exact(2) {
        gray.push(chunk[0]);
    }
    gray
}

/// Strip alpha from 16-bit GrayscaleAlpha, keeping native big-endian byte order
fn strip_alpha_grayscale_16(buf: &[u8]) -> Vec<u8> {
    let pixel_count = buf.len() / 4; // 2 channels × 2 bytes
    let mut gray = Vec::with_capacity(pixel_count * 2);
    for chunk in buf.chunks_exact(4) {
        gray.extend_from_slice(&chunk[..2]); // Gray(2), skip A(2)
    }
    gray
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_png(
        width: u32,
        height: u32,
        color_type: png::ColorType,
        bit_depth: png::BitDepth,
        data: &[u8],
    ) -> Vec<u8> {
        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(Cursor::new(&mut output), width, height);
            encoder.set_color(color_type);
            encoder.set_depth(bit_depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        output
    }

    fn load_png_from_bytes(data: &[u8]) -> CameraResult<Frame> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), data).unwrap();
        load_png(tmp.path())
    }

    #[test]
    fn test_rgb8_zero_copy_path() {
        let pixels: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let png_data = encode_png(2, 2, png::ColorType::Rgb, png::BitDepth::Eight, &pixels);
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.width(), 2);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.channels(), 3);
        assert!((frame.get_pixel(0, 0, 0) - 1.0).abs() < 1e-6); // R=255
        assert!((frame.get_pixel(0, 0, 1) - 0.0).abs() < 1e-6); // G=0
    }

    #[test]
    fn test_rgb16_be_passthrough() {
        // 1x1 pixel, RGB16 big-endian: R=32768, G=0, B=65535
        let pixels: Vec<u8> = vec![0x80, 0x00, 0x00, 0x00, 0xFF, 0xFF];
        let png_data = encode_png(1, 1, png::ColorType::Rgb, png::BitDepth::Sixteen, &pixels);
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 3);
        assert!((frame.get_pixel(0, 0, 0) - 32768.0 / 65535.0).abs() < 0.001);
        assert!((frame.get_pixel(0, 0, 1) - 0.0).abs() < 1e-6);
        assert!((frame.get_pixel(0, 0, 2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_grayscale_8_zero_copy() {
        let pixels: Vec<u8> = vec![0, 128, 255, 64];
        let png_data = encode_png(
            2,
            2,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &pixels,
        );
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 1);
        assert!((frame.get_pixel(0, 0, 0) - 0.0).abs() < 1e-6);
        assert!((frame.get_pixel(1, 0, 0) - 128.0 / 255.0).abs() < 0.001);
    }

    #[test]
    fn test_grayscale_16_be_passthrough() {
        let pixels: Vec<u8> = vec![0x80, 0x00]; // 32768 big-endian
        let png_data = encode_png(
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            &pixels,
        );
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 1);
        assert!((frame.get_pixel(0, 0, 0) - 32768.0 / 65535.0).abs() < 0.001);
    }

    #[test]
    fn test_rgba8_alpha_strip() {
        // 1x1 RGBA pixel: R=200, G=100, B=50, A=255
        let pixels: Vec<u8> = vec![200, 100, 50, 255];
        let png_data = encode_png(1, 1, png::ColorType::Rgba, png::BitDepth::Eight, &pixels);
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 3);
        assert!((frame.get_pixel(0, 0, 0) - 200.0 / 255.0).abs() < 0.001);
        assert!((frame.get_pixel(0, 0, 1) - 100.0 / 255.0).abs() < 0.001);
        assert!((frame.get_pixel(0, 0, 2) - 50.0 / 255.0).abs() < 0.001);
    }

    #[test]
    fn test_rgba16_alpha_strip_be() {
        // 1x1 RGBA16 pixel (big-endian): R=0x8000, G=0x0000, B=0xFFFF, A=0xFFFF
        let pixels: Vec<u8> = vec![0x80, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        let png_data = encode_png(1, 1, png::ColorType::Rgba, png::BitDepth::Sixteen, &pixels);
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 3);
        assert!((frame.get_pixel(0, 0, 0) - 32768.0 / 65535.0).abs() < 0.001);
        assert!((frame.get_pixel(0, 0, 1) - 0.0).abs() < 1e-6);
        assert!((frame.get_pixel(0, 0, 2) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_grayscale_alpha_8_strip() {
        let pixels: Vec<u8> = vec![128, 255]; // Gray=128, A=255
        let png_data = encode_png(
            1,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            &pixels,
        );
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 1);
        assert!((frame.get_pixel(0, 0, 0) - 128.0 / 255.0).abs() < 0.001);
    }

    #[test]
    fn test_grayscale_alpha_16_strip_be() {
        // Gray=0x8000 (32768 BE), A=0xFFFF
        let pixels: Vec<u8> = vec![0x80, 0x00, 0xFF, 0xFF];
        let png_data = encode_png(
            1,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Sixteen,
            &pixels,
        );
        let frame = load_png_from_bytes(&png_data).unwrap();

        assert_eq!(frame.channels(), 1);
        assert!((frame.get_pixel(0, 0, 0) - 32768.0 / 65535.0).abs() < 0.001);
    }

    #[test]
    fn test_strip_alpha_8_multi_pixel() {
        let input = vec![10, 20, 30, 255, 40, 50, 60, 128];
        let result = strip_alpha_8(&input);
        assert_eq!(result, vec![10, 20, 30, 40, 50, 60]);
    }

    #[test]
    fn test_strip_alpha_16_multi_pixel() {
        let input = vec![
            0x00, 0x0A, 0x00, 0x14, 0x00, 0x1E, 0xFF, 0xFF, // pixel 1
            0x00, 0x28, 0x00, 0x32, 0x00, 0x3C, 0x00, 0x00, // pixel 2
        ];
        let result = strip_alpha_16(&input);
        assert_eq!(
            result,
            vec![0x00, 0x0A, 0x00, 0x14, 0x00, 0x1E, 0x00, 0x28, 0x00, 0x32, 0x00, 0x3C]
        );
    }

    #[test]
    fn test_strip_alpha_grayscale_8_multi_pixel() {
        let input = vec![100, 255, 200, 128];
        let result = strip_alpha_grayscale_8(&input);
        assert_eq!(result, vec![100, 200]);
    }

    #[test]
    fn test_strip_alpha_grayscale_16_multi_pixel() {
        let input = vec![0x80, 0x00, 0xFF, 0xFF, 0x40, 0x00, 0x00, 0x00];
        let result = strip_alpha_grayscale_16(&input);
        assert_eq!(result, vec![0x80, 0x00, 0x40, 0x00]);
    }
}
