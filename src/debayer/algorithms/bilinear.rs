//! Bilinear interpolation debayering algorithm
//!
//! Simple averaging of neighboring pixels. Fast and suitable for live stacking
//! where speed matters more than maximum quality.

use rayon::prelude::*;

use crate::debayer::CfaPattern;
use crate::error::Result;
use crate::frame::Frame;

use super::{
    get_raw, get_rb_orientation, interpolate_blue_at_green, interpolate_diagonal,
    interpolate_green_cardinal, interpolate_red_at_green,
};

/// Perform bilinear debayering on a single-channel Bayer frame
pub fn debayer_bilinear(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    let mut output = vec![0.0f32; width * height * 3];

    output
        .par_chunks_mut(width * 3)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let out_idx = x * 3;
                row[out_idx] = r;
                row[out_idx + 1] = g;
                row[out_idx + 2] = b;
            }
        });

    Frame::from_f32_vec(output, width, height, 3)
}

/// Perform bilinear debayering directly to a 8-bit RGB vector
/// Bypasses intermediate f32 Frame allocations for encoding/streaming
pub fn debayer_bilinear_to_rgb8(frame: &Frame, pattern: CfaPattern) -> Result<Vec<u8>> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    // Allocate an uninitialized vector of the exact size needed, or just collect from par_chunks
    // We will use collect to avoid zero-initialization overhead, similar to to_rgb8_fast
    let output: Vec<u8> = (0..height)
        .into_par_iter()
        .flat_map_iter(|y| {
            let mut row = Vec::with_capacity(width * 3);
            for x in 0..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                row.push((r.max(0.0).min(1.0) * 255.0 + 0.5) as u8);
                row.push((g.max(0.0).min(1.0) * 255.0 + 0.5) as u8);
                row.push((b.max(0.0).min(1.0) * 255.0 + 0.5) as u8);
            }
            row
        })
        .collect();

    Ok(output)
}

/// Bilinear interpolation at a single pixel
pub(crate) fn bilinear_at(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let color = pattern.color_at(x, y);
    let xi = x as isize;
    let yi = y as isize;
    let this = data[y * width + x];

    match color {
        0 => interpolate_at_red(data, width, height, xi, yi, this),
        1 => interpolate_at_green(data, width, height, x, y, xi, yi, this, pattern),
        2 => interpolate_at_blue(data, width, height, xi, yi, this),
        _ => unreachable!(),
    }
}

/// Interpolate at a red pixel position
#[inline]
fn interpolate_at_red(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    this: f32,
) -> (f32, f32, f32) {
    let g = interpolate_green_cardinal(data, width, height, xi, yi);
    let b = interpolate_diagonal(data, width, height, xi, yi);
    (this, g, b)
}

/// Interpolate at a blue pixel position
#[inline]
fn interpolate_at_blue(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    this: f32,
) -> (f32, f32, f32) {
    let g = interpolate_green_cardinal(data, width, height, xi, yi);
    let r = interpolate_diagonal(data, width, height, xi, yi);
    (r, g, this)
}

/// Interpolate at a green pixel position
fn interpolate_at_green(
    data: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    xi: isize,
    yi: isize,
    this: f32,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let x_odd = x & 1;
    let y_odd = y & 1;

    let (r, b) = match pattern {
        CfaPattern::Rggb => interpolate_rb_rggb(data, width, height, xi, yi, y_odd),
        CfaPattern::Bggr => interpolate_rb_bggr(data, width, height, xi, yi, y_odd),
        CfaPattern::Grbg => interpolate_rb_grbg(data, width, height, xi, yi, x_odd, y_odd, this),
        CfaPattern::Gbrg => interpolate_rb_gbrg(data, width, height, xi, yi, x_odd, y_odd, this),
    };

    (r, this, b)
}

#[inline]
fn interpolate_rb_rggb(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    y_odd: usize,
) -> (f32, f32) {
    if y_odd == 0 {
        let r = interpolate_red_at_green(data, width, height, xi, yi, true);
        let b = interpolate_blue_at_green(data, width, height, xi, yi, false);
        (r, b)
    } else {
        let b = interpolate_blue_at_green(data, width, height, xi, yi, true);
        let r = interpolate_red_at_green(data, width, height, xi, yi, false);
        (r, b)
    }
}

#[inline]
fn interpolate_rb_bggr(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    y_odd: usize,
) -> (f32, f32) {
    if y_odd == 0 {
        let b = interpolate_blue_at_green(data, width, height, xi, yi, true);
        let r = interpolate_red_at_green(data, width, height, xi, yi, false);
        (r, b)
    } else {
        let r = interpolate_red_at_green(data, width, height, xi, yi, true);
        let b = interpolate_blue_at_green(data, width, height, xi, yi, false);
        (r, b)
    }
}

#[inline]
fn interpolate_rb_grbg(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    x_odd: usize,
    y_odd: usize,
    this: f32,
) -> (f32, f32) {
    if x_odd == 1 {
        if y_odd == 0 {
            // This is R position
            let b = interpolate_diagonal(data, width, height, xi, yi);
            (this, b)
        } else {
            // Green in B row
            let (r_horiz, b_horiz) = get_rb_orientation(CfaPattern::Grbg, xi as usize, yi as usize);
            let b = interpolate_blue_at_green(data, width, height, xi, yi, b_horiz);
            let r = interpolate_red_at_green(data, width, height, xi, yi, r_horiz);
            (r, b)
        }
    } else if y_odd == 0 {
        // Green in R row
        let r = interpolate_red_at_green(data, width, height, xi, yi, true);
        let b = interpolate_blue_at_green(data, width, height, xi, yi, false);
        (r, b)
    } else {
        // B position
        let r = interpolate_diagonal(data, width, height, xi, yi);
        (r, this)
    }
}

#[inline]
fn interpolate_rb_gbrg(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    x_odd: usize,
    y_odd: usize,
    this: f32,
) -> (f32, f32) {
    if x_odd == 1 {
        if y_odd == 0 {
            // B position
            let r = interpolate_diagonal(data, width, height, xi, yi);
            (r, this)
        } else {
            // Green in R row
            let r = interpolate_red_at_green(data, width, height, xi, yi, true);
            let b = interpolate_blue_at_green(data, width, height, xi, yi, false);
            (r, b)
        }
    } else if y_odd == 0 {
        // Green in B row
        let b = interpolate_blue_at_green(data, width, height, xi, yi, true);
        let r = interpolate_red_at_green(data, width, height, xi, yi, false);
        (r, b)
    } else {
        // R position
        let b = interpolate_diagonal(data, width, height, xi, yi);
        (this, b)
    }
}
