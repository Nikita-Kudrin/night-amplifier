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
    let clamped_x = x.clamp(0, width as isize - 1) as usize;
    let clamped_y = y.clamp(0, height as isize - 1) as usize;
    data[clamped_y * width + clamped_x]
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
        (get_raw(data, width, height, xi - 1, yi) + get_raw(data, width, height, xi + 1, yi)) * 0.5
    } else {
        (get_raw(data, width, height, xi, yi - 1) + get_raw(data, width, height, xi, yi + 1)) * 0.5
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
        (get_raw(data, width, height, xi - 1, yi) + get_raw(data, width, height, xi + 1, yi)) * 0.5
    } else {
        (get_raw(data, width, height, xi, yi - 1) + get_raw(data, width, height, xi, yi + 1)) * 0.5
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
        * 0.25
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
        * 0.25
}

/// Determine R/B horizontal orientation at a green pixel for a given pattern
#[inline]
pub(crate) fn get_rb_orientation(pattern: CfaPattern, _x: usize, y: usize) -> (bool, bool) {
    let y_odd = y & 1;

    match pattern {
        CfaPattern::Rggb | CfaPattern::Grbg => {
            if y_odd == 0 {
                (true, false) // R is horizontal, B is vertical
            } else {
                (false, true) // B is horizontal, R is vertical
            }
        }
        CfaPattern::Bggr | CfaPattern::Gbrg => {
            if y_odd == 0 {
                (false, true) // B is horizontal, R is vertical
            } else {
                (true, false) // R is horizontal, B is vertical
            }
        }
    }
}
