//! Image warping using affine transformations with bilinear interpolation.
//!
//! Frames are warped using the inverse transformation approach:
//! - For each output pixel, compute its source location in the input frame
//! - Use bilinear interpolation to sample the input at sub-pixel coordinates
//! - This avoids holes in the output that forward mapping would create

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::registration::AffineTransform;
use rayon::prelude::*;

mod cache;
mod interpolate;

pub(crate) use cache::InverseTransformCache;
use interpolate::{bilinear_interpolate_direct, warp_row_rgb};

/// Warps a frame using an affine transformation with bilinear interpolation.
pub fn warp_frame(frame: &Frame, transform: &AffineTransform, border_value: f32) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    let mut output_data = vec![border_value; width * height * channels];

    // Pre-compute inverse transform coefficients once per frame
    let inv_cache = InverseTransformCache::from_transform(transform);

    output_data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(dy, row)| {
            warp_row(frame, &inv_cache, dy, row, border_value);
        });

    Frame::from_f32_vec(output_data, width, height, channels)
}

/// Warps a frame in-place into a pre-allocated output buffer.
pub fn warp_frame_into(
    frame: &Frame,
    transform: &AffineTransform,
    output: &mut Frame,
    border_value: f32,
) -> Result<()> {
    if !frame.dimensions_match(output) {
        return Err(StackError::CalibrationDimensionMismatch {
            frame_width: frame.width(),
            frame_height: frame.height(),
            cal_width: output.width(),
            cal_height: output.height(),
        });
    }

    let width = frame.width();
    let channels = frame.channels();
    let output_data = output.data_mut();

    // Pre-compute inverse transform coefficients once per frame
    let inv_cache = InverseTransformCache::from_transform(transform);

    output_data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(dy, row)| {
            warp_row(frame, &inv_cache, dy, row, border_value);
        });

    Ok(())
}

/// Warps a single row of the output image using pre-computed inverse transform.
#[inline]
fn warp_row(
    frame: &Frame,
    inv_cache: &InverseTransformCache,
    dy: usize,
    row: &mut [f32],
    border_value: f32,
) {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let src_data = frame.data();

    // Pre-compute bounds for valid source coordinates
    let max_sx = (width - 2) as f32;
    let max_sy = (height - 2) as f32;

    // Get starting source coordinates and step values for incremental computation
    let (mut sx, mut sy) = inv_cache.inverse_transform_point(0.0, dy as f32);
    let (sx_step, sy_step) = inv_cache.x_step();

    // Use specialized RGB path for the common 3-channel case
    if channels == 3 {
        warp_row_rgb(
            src_data,
            width,
            height,
            row,
            border_value,
            sx,
            sy,
            sx_step,
            sy_step,
            max_sx,
            max_sy,
        );
    } else {
        // Generic path for other channel counts
        for dx in 0..width {
            if sx >= 0.0 && sx < max_sx && sy >= 0.0 && sy < max_sy {
                bilinear_interpolate_direct(
                    src_data,
                    width,
                    channels,
                    sx,
                    sy,
                    &mut row[dx * channels..(dx + 1) * channels],
                );
            } else {
                for c in 0..channels {
                    row[dx * channels + c] = border_value;
                }
            }
            sx += sx_step;
            sy += sy_step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn create_gradient_frame(width: usize, height: usize) -> Frame {
        let mut data = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let r = x as f32 / width as f32;
                let g = y as f32 / height as f32;
                let b = 0.5;
                data.push(r);
                data.push(g);
                data.push(b);
            }
        }
        Frame::from_f32_vec(data, width, height, 3).unwrap()
    }

    fn create_spot_frame(width: usize, height: usize, spot_x: usize, spot_y: usize) -> Frame {
        let mut frame = Frame::filled(width, height, 3, 0.1).unwrap();
        let data = frame.data_mut();

        for dy in 0..height {
            for dx in 0..width {
                let dist_sq =
                    (dx as f32 - spot_x as f32).powi(2) + (dy as f32 - spot_y as f32).powi(2);
                let intensity = (-dist_sq / 50.0).exp();

                let idx = (dy * width + dx) * 3;
                data[idx] += intensity;
                data[idx + 1] += intensity;
                data[idx + 2] += intensity;
            }
        }

        frame
    }

    #[test]
    fn test_warp_identity() {
        let frame = create_gradient_frame(64, 64);
        let transform = AffineTransform::identity();

        let warped = warp_frame(&frame, &transform, 0.0).unwrap();

        let orig_data = frame.data();
        let warp_data = warped.data();

        for y in 5..59 {
            for x in 5..59 {
                for c in 0..3 {
                    let idx = (y * 64 + x) * 3 + c;
                    assert!(
                        (orig_data[idx] - warp_data[idx]).abs() < 0.01,
                        "Mismatch at ({}, {}, {}): {} vs {}",
                        x,
                        y,
                        c,
                        orig_data[idx],
                        warp_data[idx]
                    );
                }
            }
        }
    }

    #[test]
    fn test_warp_translation() {
        let frame = create_spot_frame(64, 64, 32, 32);
        let transform = AffineTransform::new(0.0, 1.0, 5.0, 3.0);

        let warped = warp_frame(&frame, &transform, 0.0).unwrap();

        let orig_center_val = frame.get_pixel(32, 32, 0);
        let warp_center_val = warped.get_pixel(37, 35, 0);

        assert!(
            (orig_center_val - warp_center_val).abs() < 0.1,
            "Spot should move with translation: orig={}, warp={}",
            orig_center_val,
            warp_center_val
        );
    }

    #[test]
    fn test_warp_rotation() {
        let frame = create_gradient_frame(64, 64);
        let angle = PI / 6.0;
        let transform = AffineTransform::new(angle, 1.0, 0.0, 0.0);

        let warped = warp_frame(&frame, &transform, 0.0).unwrap();

        assert_eq!(warped.width(), 64);
        assert_eq!(warped.height(), 64);

        let center_val = warped.get_pixel(32, 32, 0);
        assert!(center_val >= 0.0 && center_val <= 1.0);
    }

    #[test]
    fn test_warp_boundary_no_panic() {
        let frame = create_gradient_frame(100, 100);
        let transform = AffineTransform::identity();
        let warped = warp_frame(&frame, &transform, 0.0).unwrap();
        assert_eq!(warped.width(), 100);
        assert_eq!(warped.height(), 100);

        let transform = AffineTransform::new(0.0, 1.0, 0.5, 0.5);
        let warped = warp_frame(&frame, &transform, 0.0).unwrap();
        assert_eq!(warped.width(), 100);

        let transform = AffineTransform::new(0.0, 1.0, -0.5, -0.5);
        let warped = warp_frame(&frame, &transform, 0.0).unwrap();
        assert_eq!(warped.width(), 100);
    }
}
