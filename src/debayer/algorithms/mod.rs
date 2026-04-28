//! Debayering algorithm implementations
//!
//! This module contains the different interpolation algorithms for converting
//! Bayer pattern data to RGB images.

mod bilinear;
mod vng;

pub use bilinear::{debayer_bilinear, debayer_bilinear_to_rgb8};
pub use vng::debayer_vng;

use crate::debayer::CfaPattern;

/// Get raw pixel value with bounds checking (returns 0 for out-of-bounds)
#[inline]
pub(crate) fn get_raw(data: &[f32], width: usize, height: usize, x: isize, y: isize) -> f32 {
    if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
        return 0.0;
    }
    data[y as usize * width + x as usize]
}

/// Interpolate missing red channel at a green pixel position
#[inline]
pub(crate) fn interpolate_red_at_green(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    red_horizontal: bool,
) -> f32 {
    if red_horizontal {
        (get_raw(data, width, height, xi - 1, yi) + get_raw(data, width, height, xi + 1, yi)) / 2.0
    } else {
        (get_raw(data, width, height, xi, yi - 1) + get_raw(data, width, height, xi, yi + 1)) / 2.0
    }
}

/// Interpolate missing blue channel at a green pixel position
#[inline]
pub(crate) fn interpolate_blue_at_green(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
    blue_horizontal: bool,
) -> f32 {
    if blue_horizontal {
        (get_raw(data, width, height, xi - 1, yi) + get_raw(data, width, height, xi + 1, yi)) / 2.0
    } else {
        (get_raw(data, width, height, xi, yi - 1) + get_raw(data, width, height, xi, yi + 1)) / 2.0
    }
}

/// Interpolate green channel from 4 cardinal neighbors
#[inline]
pub(crate) fn interpolate_green_cardinal(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
) -> f32 {
    (get_raw(data, width, height, xi - 1, yi)
        + get_raw(data, width, height, xi + 1, yi)
        + get_raw(data, width, height, xi, yi - 1)
        + get_raw(data, width, height, xi, yi + 1))
        / 4.0
}

/// Interpolate a channel from 4 diagonal neighbors
#[inline]
pub(crate) fn interpolate_diagonal(
    data: &[f32],
    width: usize,
    height: usize,
    xi: isize,
    yi: isize,
) -> f32 {
    (get_raw(data, width, height, xi - 1, yi - 1)
        + get_raw(data, width, height, xi + 1, yi - 1)
        + get_raw(data, width, height, xi - 1, yi + 1)
        + get_raw(data, width, height, xi + 1, yi + 1))
        / 4.0
}

/// Determine R/B horizontal orientation at a green pixel for a given pattern
#[inline]
pub(crate) fn get_rb_orientation(pattern: CfaPattern, x: usize, y: usize) -> (bool, bool) {
    let x_odd = x & 1;
    let y_odd = y & 1;

    match pattern {
        CfaPattern::Rggb => {
            if y_odd == 0 {
                (true, false) // R horizontal, B vertical
            } else {
                (false, true) // R vertical, B horizontal
            }
        }
        CfaPattern::Bggr => {
            if y_odd == 0 {
                (false, true) // B horizontal, R vertical
            } else {
                (true, false) // R horizontal, B vertical
            }
        }
        CfaPattern::Grbg => {
            if x_odd == 0 {
                if y_odd == 0 {
                    (true, false) // G in R row
                } else {
                    (false, true) // G in B row
                }
            } else {
                (false, false) // This is R or B position, not G
            }
        }
        CfaPattern::Gbrg => {
            if x_odd == 0 {
                if y_odd == 0 {
                    (false, true) // G in B row
                } else {
                    (true, false) // G in R row
                }
            } else {
                (false, false) // This is R or B position, not G
            }
        }
    }
}
