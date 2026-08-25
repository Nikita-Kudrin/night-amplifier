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
use interpolate::{bilinear_interpolate_direct_1ch, warp_row_rgb};

/// Warps a frame using an affine transformation with bilinear interpolation.
pub fn warp_frame(frame: &Frame, transform: &AffineTransform, border_value: f32) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    // `filled`, not `zeros`: the row writers below do cover every pixel, but a frame
    // pre-filled with the border value means a future early-return leaves a sane
    // border rather than silent black. `?` rather than `unwrap` — this function
    // returns `Result`, and the fallible construction used to propagate through
    // `Frame::from_f32_vec`.
    let mut output = Frame::filled(width, height, channels, border_value)?;
    
    // Pre-compute inverse transform coefficients once per frame
    let inv_cache = InverseTransformCache::from_transform(transform);

    if channels == 3 {
        let (src_r, src_g, src_b) = frame.planes();
        let (r_plane, g_plane, b_plane) = output.planes_mut();
        r_plane.par_chunks_mut(width)
            .zip(g_plane.par_chunks_mut(width))
            .zip(b_plane.par_chunks_mut(width))
            .enumerate()
            .for_each(|(dy, ((r_row, g_row), b_row))| {
                warp_row_rgb_planar(frame, &inv_cache, dy, src_r, src_g, src_b, r_row, g_row, b_row, border_value);
            });
    } else {
        for c in 0..channels {
            let src_c = frame.channel_data(c);
            let c_plane = output.channel_data_mut(c);
            c_plane.par_chunks_mut(width)
                .enumerate()
                .for_each(|(dy, c_row)| {
                    warp_row_1ch(frame, &inv_cache, dy, src_c, c_row, border_value);
                });
        }
    }

    Ok(output)
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

    // Pre-compute inverse transform coefficients once per frame
    let inv_cache = InverseTransformCache::from_transform(transform);

    if channels == 3 {
        let (src_r, src_g, src_b) = frame.planes();
        let (r_plane, g_plane, b_plane) = output.planes_mut();
        r_plane.par_chunks_mut(width)
            .zip(g_plane.par_chunks_mut(width))
            .zip(b_plane.par_chunks_mut(width))
            .enumerate()
            .for_each(|(dy, ((r_row, g_row), b_row))| {
                warp_row_rgb_planar(frame, &inv_cache, dy, src_r, src_g, src_b, r_row, g_row, b_row, border_value);
            });
    } else {
        for c in 0..channels {
            let src_c = frame.channel_data(c);
            let c_plane = output.channel_data_mut(c);
            c_plane.par_chunks_mut(width)
                .enumerate()
                .for_each(|(dy, c_row)| {
                    warp_row_1ch(frame, &inv_cache, dy, src_c, c_row, border_value);
                });
        }
    }

    Ok(())
}

#[inline]
fn warp_row_rgb_planar(
    frame: &Frame,
    inv_cache: &InverseTransformCache,
    dy: usize,
    src_r: &[f32],
    src_g: &[f32],
    src_b: &[f32],
    r_row: &mut [f32],
    g_row: &mut [f32],
    b_row: &mut [f32],
    border_value: f32,
) {
    let width = frame.width();
    let height = frame.height();

    // Pre-compute bounds for valid source coordinates
    let max_sx = (width - 2) as f32;
    let max_sy = (height - 2) as f32;

    // Get starting source coordinates and step values for incremental computation
    let (sx, sy) = inv_cache.inverse_transform_point(0.0, dy as f32);
    let (sx_step, sy_step) = inv_cache.x_step();

    warp_row_rgb(
        src_r, src_g, src_b,
        width, height,
        r_row, g_row, b_row,
        border_value,
        sx, sy,
        sx_step, sy_step,
        max_sx, max_sy,
    );
}

#[inline]
fn warp_row_1ch(
    frame: &Frame,
    inv_cache: &InverseTransformCache,
    dy: usize,
    src_r: &[f32],
    r_row: &mut [f32],
    border_value: f32,
) {
    let width = frame.width();
    let height = frame.height();

    let max_sx = (width - 2) as f32;
    let max_sy = (height - 2) as f32;

    let (mut sx, mut sy) = inv_cache.inverse_transform_point(0.0, dy as f32);
    let (sx_step, sy_step) = inv_cache.x_step();

    for r_px in r_row.iter_mut().take(width) {
        if sx >= 0.0 && sx < max_sx && sy >= 0.0 && sy < max_sy {
            *r_px = bilinear_interpolate_direct_1ch(src_r, width, sx, sy);
        } else {
            *r_px = border_value;
        }
        sx += sx_step;
        sy += sy_step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn create_gradient_frame(width: usize, height: usize) -> Frame {
        let mut frame = Frame::zeros(width, height, 3).unwrap();
        for y in 0..height {
            for x in 0..width {
                let r = x as f32 / width as f32;
                let g = y as f32 / height as f32;
                let b = 0.5;
                frame.set_pixel(x, y, 0, r);
                frame.set_pixel(x, y, 1, g);
                frame.set_pixel(x, y, 2, b);
            }
        }
        frame
    }

    fn create_spot_frame(width: usize, height: usize, spot_x: usize, spot_y: usize) -> Frame {
        let mut frame = Frame::filled(width, height, 3, 0.1).unwrap();

        for dy in 0..height {
            for dx in 0..width {
                let dist_sq =
                    (dx as f32 - spot_x as f32).powi(2) + (dy as f32 - spot_y as f32).powi(2);
                let intensity = (-dist_sq / 50.0).exp();

                let cur_r = frame.get_pixel(dx, dy, 0);
                let cur_g = frame.get_pixel(dx, dy, 1);
                let cur_b = frame.get_pixel(dx, dy, 2);
                
                frame.set_pixel(dx, dy, 0, cur_r + intensity);
                frame.set_pixel(dx, dy, 1, cur_g + intensity);
                frame.set_pixel(dx, dy, 2, cur_b + intensity);
            }
        }

        frame
    }

    #[test]
    fn test_warp_identity() {
        let frame = create_gradient_frame(64, 64);
        let transform = AffineTransform::identity();

        let warped = warp_frame(&frame, &transform, 0.0).unwrap();

        for y in 5..59 {
            for x in 5..59 {
                for c in 0..3 {
                    let orig_val = frame.get_pixel(x, y, c);
                    let warp_val = warped.get_pixel(x, y, c);
                    assert!(
                        (orig_val - warp_val).abs() < 0.01,
                        "Mismatch at ({}, {}, {}): {} vs {}",
                        x,
                        y,
                        c,
                        orig_val,
                        warp_val
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
