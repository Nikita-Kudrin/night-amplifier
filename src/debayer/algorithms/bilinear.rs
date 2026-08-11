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

pub trait BilinearPattern: Send + Sync {
    const PATTERN: CfaPattern;
}

pub struct PatternRggb;
impl BilinearPattern for PatternRggb { const PATTERN: CfaPattern = CfaPattern::Rggb; }
pub struct PatternBggr;
impl BilinearPattern for PatternBggr { const PATTERN: CfaPattern = CfaPattern::Bggr; }
pub struct PatternGrbg;
impl BilinearPattern for PatternGrbg { const PATTERN: CfaPattern = CfaPattern::Grbg; }
pub struct PatternGbrg;
impl BilinearPattern for PatternGbrg { const PATTERN: CfaPattern = CfaPattern::Gbrg; }

/// Perform bilinear debayering on a single-channel Bayer frame
pub fn debayer_bilinear(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    match pattern {
        CfaPattern::Rggb => debayer_bilinear_impl::<PatternRggb>(frame),
        CfaPattern::Bggr => debayer_bilinear_impl::<PatternBggr>(frame),
        CfaPattern::Grbg => debayer_bilinear_impl::<PatternGrbg>(frame),
        CfaPattern::Gbrg => debayer_bilinear_impl::<PatternGbrg>(frame),
    }
}

/// Perform bilinear debayering directly to a 8-bit RGB vector
/// Bypasses intermediate f32 Frame allocations for encoding/streaming
pub fn debayer_bilinear_to_rgb8(frame: &Frame, pattern: CfaPattern) -> Result<Vec<u8>> {
    match pattern {
        CfaPattern::Rggb => debayer_bilinear_to_rgb8_impl::<PatternRggb>(frame),
        CfaPattern::Bggr => debayer_bilinear_to_rgb8_impl::<PatternBggr>(frame),
        CfaPattern::Grbg => debayer_bilinear_to_rgb8_impl::<PatternGrbg>(frame),
        CfaPattern::Gbrg => debayer_bilinear_to_rgb8_impl::<PatternGbrg>(frame),
    }
}

fn debayer_bilinear_impl<T: BilinearPattern>(frame: &Frame) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    let mut output = vec![0.0f32; width * height * 3];

    output
        .par_chunks_mut(width * 3 * 2)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y_chunk, rows)| {
            let y_start = y_chunk * 2;
            let chunk_height = rows.len() / (width * 3);

            for i in 0..chunk_height {
                let y = y_start + i;
                let out_row = &mut rows[i * width * 3..(i + 1) * width * 3];

                if y == 0 || y == height - 1 {
                    for x in 0..width {
                        let (r, g, b) = bilinear_at(input, width, height, x, y, T::PATTERN);
                        let out_idx = x * 3;
                        out_row[out_idx] = r;
                        out_row[out_idx + 1] = g;
                        out_row[out_idx + 2] = b;
                    }
                } else {
                    let prev_row = &input[(y - 1) * width..y * width];
                    let curr_row = &input[y * width..(y + 1) * width];
                    let next_row = &input[(y + 1) * width..(y + 2) * width];

                    // Left border
                    let (r, g, b) = bilinear_at(input, width, height, 0, y, T::PATTERN);
                    out_row[0] = r;
                    out_row[1] = g;
                    out_row[2] = b;

                    // Interior
                    for x in 1..width - 1 {
                        let (r, g, b) = bilinear_at_interior(prev_row, curr_row, next_row, x, y, T::PATTERN);
                        let out_idx = x * 3;
                        out_row[out_idx] = r;
                        out_row[out_idx + 1] = g;
                        out_row[out_idx + 2] = b;
                    }

                    // Right border
                    let (r, g, b) = bilinear_at(input, width, height, width - 1, y, T::PATTERN);
                    let out_idx = (width - 1) * 3;
                    out_row[out_idx] = r;
                    out_row[out_idx + 1] = g;
                    out_row[out_idx + 2] = b;
                }
            }
        });

    Frame::from_f32_vec(output, width, height, 3)
}

fn debayer_bilinear_to_rgb8_impl<T: BilinearPattern>(frame: &Frame) -> Result<Vec<u8>> {
    let width = frame.width();
    let height = frame.height();
    let input = frame.data();

    let mut output = vec![0u8; width * height * 3];

    output
        .par_chunks_mut(width * 3 * 2)
        .with_min_len(32)
        .enumerate()
        .for_each(|(y_chunk, rows)| {
            let y_start = y_chunk * 2;
            let chunk_height = rows.len() / (width * 3);

            for i in 0..chunk_height {
                let y = y_start + i;
                let out_row = &mut rows[i * width * 3..(i + 1) * width * 3];

                if y == 0 || y == height - 1 {
                    for x in 0..width {
                        let (r, g, b) = bilinear_at(input, width, height, x, y, T::PATTERN);
                        let out_idx = x * 3;
                        out_row[out_idx] = (r.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        out_row[out_idx + 1] = (g.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        out_row[out_idx + 2] = (b.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    }
                } else {
                    let prev_row = &input[(y - 1) * width..y * width];
                    let curr_row = &input[y * width..(y + 1) * width];
                    let next_row = &input[(y + 1) * width..(y + 2) * width];

                    // Left border
                    let (r, g, b) = bilinear_at(input, width, height, 0, y, T::PATTERN);
                    out_row[0] = (r.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    out_row[1] = (g.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    out_row[2] = (b.max(0.0).min(1.0) * 255.0 + 0.5) as u8;

                    // Interior
                    for x in 1..width - 1 {
                        let (r, g, b) = bilinear_at_interior(prev_row, curr_row, next_row, x, y, T::PATTERN);
                        let out_idx = x * 3;
                        out_row[out_idx] = (r.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        out_row[out_idx + 1] = (g.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                        out_row[out_idx + 2] = (b.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    }

                    // Right border
                    let (r, g, b) = bilinear_at(input, width, height, width - 1, y, T::PATTERN);
                    let out_idx = (width - 1) * 3;
                    out_row[out_idx] = (r.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    out_row[out_idx + 1] = (g.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                    out_row[out_idx + 2] = (b.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                }
            }
        });

    Ok(output)
}

/// Bilinear interpolation at a single pixel
#[inline]
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
#[inline]
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
    let (r_horiz, b_horiz) = get_rb_orientation(pattern, x, y);
    let r = interpolate_red_at_green(data, width, height, xi, yi, r_horiz);
    let b = interpolate_blue_at_green(data, width, height, xi, yi, b_horiz);
    (r, this, b)
}

// ==========================================
// Interior fast-path versions
// ==========================================

#[inline(always)]
fn bilinear_at_interior(
    prev: &[f32],
    curr: &[f32],
    next: &[f32],
    x: usize,
    y: usize,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let color = pattern.color_at(x, y);
    let this = curr[x];

    match color {
        0 => interpolate_at_red_interior(prev, curr, next, x, this),
        1 => interpolate_at_green_interior(prev, curr, next, x, y, this, pattern),
        2 => interpolate_at_blue_interior(prev, curr, next, x, this),
        _ => unreachable!(),
    }
}

#[inline(always)]
fn interpolate_green_cardinal_interior(prev: &[f32], curr: &[f32], next: &[f32], x: usize) -> f32 {
    (curr[x - 1] + curr[x + 1] + prev[x] + next[x]) * 0.25
}

#[inline(always)]
fn interpolate_diagonal_interior(prev: &[f32], curr: &[f32], next: &[f32], x: usize) -> f32 {
    (prev[x - 1] + prev[x + 1] + next[x - 1] + next[x + 1]) * 0.25
}

#[inline(always)]
fn interpolate_red_at_green_interior(
    prev: &[f32], curr: &[f32], next: &[f32], x: usize, red_horizontal: bool
) -> f32 {
    if red_horizontal {
        (curr[x - 1] + curr[x + 1]) * 0.5
    } else {
        (prev[x] + next[x]) * 0.5
    }
}

#[inline(always)]
fn interpolate_blue_at_green_interior(
    prev: &[f32], curr: &[f32], next: &[f32], x: usize, blue_horizontal: bool
) -> f32 {
    if blue_horizontal {
        (curr[x - 1] + curr[x + 1]) * 0.5
    } else {
        (prev[x] + next[x]) * 0.5
    }
}

#[inline(always)]
fn interpolate_at_red_interior(
    prev: &[f32],
    curr: &[f32],
    next: &[f32],
    x: usize,
    this: f32,
) -> (f32, f32, f32) {
    let g = interpolate_green_cardinal_interior(prev, curr, next, x);
    let b = interpolate_diagonal_interior(prev, curr, next, x);
    (this, g, b)
}

#[inline(always)]
fn interpolate_at_blue_interior(
    prev: &[f32],
    curr: &[f32],
    next: &[f32],
    x: usize,
    this: f32,
) -> (f32, f32, f32) {
    let g = interpolate_green_cardinal_interior(prev, curr, next, x);
    let r = interpolate_diagonal_interior(prev, curr, next, x);
    (r, g, this)
}

#[inline(always)]
fn interpolate_at_green_interior(
    prev: &[f32],
    curr: &[f32],
    next: &[f32],
    x: usize,
    y: usize,
    this: f32,
    pattern: CfaPattern,
) -> (f32, f32, f32) {
    let (r_horiz, b_horiz) = get_rb_orientation(pattern, x, y);
    let r = interpolate_red_at_green_interior(prev, curr, next, x, r_horiz);
    let b = interpolate_blue_at_green_interior(prev, curr, next, x, b_horiz);
    (r, this, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_frame() -> Frame {
        let width = 64;
        let height = 64;
        let mut data = Vec::with_capacity(width * height);
        // Create some pseudo-random pattern
        for i in 0..(width * height) {
            let val = ((i * 17) % 256) as f32 / 255.0;
            data.push(val);
        }
        Frame::from_f32_vec(data, width, height, 1).unwrap()
    }

    fn debayer_bilinear_reference(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
        let width = frame.width();
        let height = frame.height();
        let input = frame.data();
        let mut output = vec![0.0f32; width * height * 3];

        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let out_idx = (y * width + x) * 3;
                output[out_idx] = r;
                output[out_idx + 1] = g;
                output[out_idx + 2] = b;
            }
        }
        Frame::from_f32_vec(output, width, height, 3)
    }

    fn debayer_bilinear_to_rgb8_reference(frame: &Frame, pattern: CfaPattern) -> Result<Vec<u8>> {
        let width = frame.width();
        let height = frame.height();
        let input = frame.data();
        let mut output = vec![0u8; width * height * 3];

        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = bilinear_at(input, width, height, x, y, pattern);
                let out_idx = (y * width + x) * 3;
                output[out_idx] = (r.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                output[out_idx + 1] = (g.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
                output[out_idx + 2] = (b.max(0.0).min(1.0) * 255.0 + 0.5) as u8;
            }
        }
        Ok(output)
    }

    #[test]
    fn test_debayer_bilinear_bit_identical() {
        let frame = create_test_frame();
        
        for pattern in CfaPattern::all() {
            let optimized = debayer_bilinear(&frame, pattern).unwrap();
            let reference = debayer_bilinear_reference(&frame, pattern).unwrap();
            
            assert_eq!(optimized.data().len(), reference.data().len());
            for i in 0..optimized.data().len() {
                assert_eq!(
                    optimized.data()[i], 
                    reference.data()[i], 
                    "Mismatch at index {} for pattern {:?}", i, pattern
                );
            }
        }
    }

    #[test]
    fn test_debayer_bilinear_to_rgb8_bit_identical() {
        let frame = create_test_frame();
        
        for pattern in CfaPattern::all() {
            let optimized = debayer_bilinear_to_rgb8(&frame, pattern).unwrap();
            let reference = debayer_bilinear_to_rgb8_reference(&frame, pattern).unwrap();
            
            assert_eq!(optimized.len(), reference.len());
            for i in 0..optimized.len() {
                assert_eq!(
                    optimized[i], 
                    reference[i], 
                    "Mismatch at index {} for pattern {:?}", i, pattern
                );
            }
        }
    }
}
