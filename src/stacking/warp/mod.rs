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

/// Smallest frame a bilinear warp can sample from.
///
/// The interpolator reads a 2x2 neighbourhood, so the last valid source coordinate is
/// `width - 2`. Below this the row kernels have nothing to interpolate between and the
/// bound itself underflows — see [`warp_planes`].
const MIN_WARP_EXTENT: usize = 2;

/// Warps a frame using an affine transformation with bilinear interpolation.
pub fn warp_frame(frame: &Frame, transform: &AffineTransform, border_value: f32) -> Result<Frame> {
    // `filled`, not `zeros`: the row writers below do cover every pixel, but a frame
    // pre-filled with the border value means a future early-return leaves a sane
    // border rather than silent black. `?` rather than `unwrap` — this function
    // returns `Result`, and the fallible construction used to propagate through
    // `Frame::from_f32_vec`.
    let mut output = Frame::filled(
        frame.width(),
        frame.height(),
        frame.channels(),
        border_value,
    )?;
    warp_planes(frame, transform, &mut output, border_value)?;
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

    warp_planes(frame, transform, output, border_value)
}

/// The plane dispatch both public entry points share.
///
/// It lives in one place because both of them need the dimension guard below, and
/// carrying two copies of it is how one of them would end up without it. `warp_frame`
/// and `warp_frame_into` differ only in where the destination comes from.
///
/// # Why sub-2px frames are rejected rather than clamped
///
/// The row kernels derive their upper bound as `width - 2`, since bilinear interpolation
/// reads `(x0, y0)` through `(x0 + 1, y0 + 1)`. On a `usize` that underflows for
/// `width < 2`: a debug build panics with "attempt to subtract with overflow", and a
/// release build wraps to ~1.8e19, which passes the bounds test and sends
/// `bilinear_interpolate_direct_1ch` past the end of the plane. Saturating the bound to
/// zero instead would return an all-border frame, which is a silently wrong answer
/// rather than a loud one — there is no meaningful bilinear result on a frame with no
/// interior, so the caller should hear about it.
fn warp_planes(
    frame: &Frame,
    transform: &AffineTransform,
    output: &mut Frame,
    border_value: f32,
) -> Result<()> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();

    if width < MIN_WARP_EXTENT || height < MIN_WARP_EXTENT {
        return Err(StackError::InvalidDimensions {
            width,
            height,
            channels,
        });
    }

    // Pre-compute inverse transform coefficients once per frame
    let inv_cache = InverseTransformCache::from_transform(transform);
    let bounds = RowBounds::of(width, height);

    if channels == 3 {
        let (src_r, src_g, src_b) = frame.planes();
        let (r_plane, g_plane, b_plane) = output.planes_mut();
        r_plane
            .par_chunks_mut(width)
            .zip(g_plane.par_chunks_mut(width))
            .zip(b_plane.par_chunks_mut(width))
            .enumerate()
            .for_each(|(dy, ((r_row, g_row), b_row))| {
                warp_row_rgb_planar(
                    &bounds,
                    &inv_cache,
                    dy,
                    src_r,
                    src_g,
                    src_b,
                    r_row,
                    g_row,
                    b_row,
                    border_value,
                );
            });
        return Ok(());
    }

    for c in 0..channels {
        let src_c = frame.channel_data(c);
        let c_plane = output.channel_data_mut(c);
        c_plane
            .par_chunks_mut(width)
            .enumerate()
            .for_each(|(dy, c_row)| {
                warp_row_1ch(&bounds, &inv_cache, dy, src_c, c_row, border_value);
            });
    }

    Ok(())
}

/// Frame geometry and the derived source-coordinate limits, resolved once per warp.
///
/// The row kernels used to re-derive these from the `&Frame` on every row, which is
/// also what kept the underflowing subtraction inside the parallel closure where it
/// could not be guarded once.
struct RowBounds {
    width: usize,
    height: usize,
    max_sx: f32,
    max_sy: f32,
}

impl RowBounds {
    /// Requires `width >= MIN_WARP_EXTENT` and `height >= MIN_WARP_EXTENT`; `warp_planes`
    /// is the only constructor call and checks that first.
    fn of(width: usize, height: usize) -> Self {
        debug_assert!(width >= MIN_WARP_EXTENT && height >= MIN_WARP_EXTENT);
        Self {
            width,
            height,
            max_sx: (width - MIN_WARP_EXTENT) as f32,
            max_sy: (height - MIN_WARP_EXTENT) as f32,
        }
    }
}

#[inline]
fn warp_row_rgb_planar(
    bounds: &RowBounds,
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
    // Get starting source coordinates and step values for incremental computation
    let (sx, sy) = inv_cache.inverse_transform_point(0.0, dy as f32);
    let (sx_step, sy_step) = inv_cache.x_step();

    warp_row_rgb(
        src_r,
        src_g,
        src_b,
        bounds.width,
        bounds.height,
        r_row,
        g_row,
        b_row,
        border_value,
        sx,
        sy,
        sx_step,
        sy_step,
        bounds.max_sx,
        bounds.max_sy,
    );
}

#[inline]
fn warp_row_1ch(
    bounds: &RowBounds,
    inv_cache: &InverseTransformCache,
    dy: usize,
    src_r: &[f32],
    r_row: &mut [f32],
    border_value: f32,
) {
    let (mut sx, mut sy) = inv_cache.inverse_transform_point(0.0, dy as f32);
    let (sx_step, sy_step) = inv_cache.x_step();

    for r_px in r_row.iter_mut().take(bounds.width) {
        if sx >= 0.0 && sx < bounds.max_sx && sy >= 0.0 && sy < bounds.max_sy {
            *r_px = bilinear_interpolate_direct_1ch(src_r, bounds.width, sx, sy);
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

#[cfg(test)]
mod dimension_guard_tests {
    use super::*;

    /// `(width - 2)` on a `usize`. Before the guard these panicked with "attempt to
    /// subtract with overflow" in a debug build, and in release wrapped to ~1.8e19,
    /// which passes the bounds test and indexes past the end of the plane.
    #[test]
    fn frames_with_no_interior_are_rejected_rather_than_underflowing() {
        for (w, h) in [(1usize, 1usize), (1, 8), (8, 1), (2, 1), (1, 2)] {
            for channels in [1usize, 3] {
                let frame = Frame::filled(w, h, channels, 0.5).unwrap();
                let err = warp_frame(&frame, &AffineTransform::identity(), 0.0);
                assert!(
                    matches!(err, Err(StackError::InvalidDimensions { .. })),
                    "{w}x{h}x{channels} should be rejected, got {err:?}"
                );

                let mut out = Frame::filled(w, h, channels, 0.0).unwrap();
                let err = warp_frame_into(&frame, &AffineTransform::identity(), &mut out, 0.0);
                assert!(
                    matches!(err, Err(StackError::InvalidDimensions { .. })),
                    "warp_frame_into {w}x{h}x{channels} should be rejected, got {err:?}"
                );
            }
        }
    }

    /// The smallest frame that still has something to interpolate must go through.
    #[test]
    fn the_minimum_warpable_frame_is_accepted() {
        for channels in [1usize, 3] {
            let frame = Frame::filled(2, 2, channels, 0.5).unwrap();
            assert!(warp_frame(&frame, &AffineTransform::identity(), 0.0).is_ok());
        }
    }

    /// `warp_frame` and `warp_frame_into` now share one dispatch; pin that they agree,
    /// since that is the property the extraction is supposed to guarantee.
    #[test]
    fn both_entry_points_produce_the_same_pixels() {
        let mut frame = Frame::zeros(37, 23, 3).unwrap();
        for y in 0..23 {
            for x in 0..37 {
                for c in 0..3 {
                    frame.set_pixel(x, y, c, (x + y * 2 + c * 7) as f32 / 100.0);
                }
            }
        }
        let transform = AffineTransform::new(0.15, 1.05, 2.5, -1.5);

        let allocated = warp_frame(&frame, &transform, 0.25).unwrap();

        let mut into = Frame::filled(37, 23, 3, 0.25).unwrap();
        warp_frame_into(&frame, &transform, &mut into, 0.25).unwrap();

        assert_eq!(allocated.data(), into.data());
    }
}
