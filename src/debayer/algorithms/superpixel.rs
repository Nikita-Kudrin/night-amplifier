//! Superpixel debayering: one RGB pixel per 2x2 CFA quad
//!
//! No interpolation at all — the red sample *is* the red channel, the blue
//! sample *is* the blue channel, and the two greens are averaged. Output is half
//! the sensor's width and height.
//!
//! Three properties matter for an eyepiece view:
//!
//! - **No interpolated chroma.** Bilinear and VNG both synthesise two of every
//!   three colour samples from neighbours, which correlates the noise between
//!   channels and turns luminance noise into colour mottle. There is nothing to
//!   correlate here.
//! - **A defect stays one pixel.** Any hot sample that survives
//!   [`crate::cfa::hot_pixels`] lands in exactly one output pixel instead of
//!   being spread into a coloured 3x3 cross.
//! - **The green average is a free 1.4x on the green channel**, which carries
//!   most of the luminance.
//!
//! It costs resolution, which is why it is an option rather than the default: an
//! IMX533's 3008² becomes 1504², still above a 1440² eyepiece screen, but an
//! IMX464's 2712x1538 becomes 1356x769, which is below it.

use rayon::prelude::*;

use crate::debayer::CfaPattern;
use crate::error::{Result, StackError};
use crate::frame::Frame;

/// Output rows handled by one rayon task.
const ROWS_PER_TASK: usize = 8;

/// Positions of the four colours inside one 2x2 quad, as `dy * 2 + dx`.
struct QuadLayout {
    red: usize,
    green: (usize, usize),
    blue: usize,
}

impl QuadLayout {
    const fn for_pattern(pattern: CfaPattern) -> Self {
        match pattern {
            CfaPattern::Rggb => Self {
                red: 0,
                green: (1, 2),
                blue: 3,
            },
            CfaPattern::Bggr => Self {
                red: 3,
                green: (1, 2),
                blue: 0,
            },
            CfaPattern::Grbg => Self {
                red: 1,
                green: (0, 3),
                blue: 2,
            },
            CfaPattern::Gbrg => Self {
                red: 2,
                green: (0, 3),
                blue: 1,
            },
        }
    }
}

/// Bin each 2x2 CFA quad into one RGB pixel.
///
/// The output is `width / 2` by `height / 2`; an odd trailing row or column has
/// no complete quad and is dropped.
pub fn debayer_superpixel(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let (out_width, out_height) = (width / 2, height / 2);
    if out_width == 0 || out_height == 0 {
        return Err(StackError::InvalidDimensions {
            width: out_width,
            height: out_height,
            channels: 3,
        });
    }

    let layout = QuadLayout::for_pattern(pattern);
    let input = frame.data();
    let area = out_width * out_height;

    let mut output = vec![0.0f32; area * 3];
    let (r_plane, rest) = output.split_at_mut(area);
    let (g_plane, b_plane) = rest.split_at_mut(area);

    r_plane
        .par_chunks_mut(out_width * ROWS_PER_TASK)
        .zip(g_plane.par_chunks_mut(out_width * ROWS_PER_TASK))
        .zip(b_plane.par_chunks_mut(out_width * ROWS_PER_TASK))
        .enumerate()
        .for_each(|(task, ((r_rows, g_rows), b_rows))| {
            let first_row = task * ROWS_PER_TASK;
            let rows = r_rows
                .chunks_mut(out_width)
                .zip(g_rows.chunks_mut(out_width))
                .zip(b_rows.chunks_mut(out_width));

            for (offset, ((r, g), b)) in rows.enumerate() {
                let y = (first_row + offset) * 2;
                let top = &input[y * width..][..width];
                let bottom = &input[(y + 1) * width..][..width];

                for x in 0..out_width {
                    let quad = [top[x * 2], top[x * 2 + 1], bottom[x * 2], bottom[x * 2 + 1]];
                    r[x] = quad[layout.red];
                    g[x] = (quad[layout.green.0] + quad[layout.green.1]) * 0.5;
                    b[x] = quad[layout.blue];
                }
            }
        });

    Frame::from_f32_vec(output, out_width, out_height, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paints each CFA colour a distinct constant, so a routing mistake shows up
    /// as a channel carrying the wrong constant rather than as a plausible image.
    fn constant_colour_mosaic(pattern: CfaPattern, width: usize, height: usize) -> Frame {
        let levels = [0.2f32, 0.5, 0.8];
        let mut frame = Frame::zeros(width, height, 1).unwrap();
        for y in 0..height {
            for x in 0..width {
                frame.set_pixel(x, y, 0, levels[pattern.color_at(x, y)]);
            }
        }
        frame
    }

    #[test]
    fn every_pattern_routes_its_quad_to_the_right_channels() {
        for pattern in CfaPattern::all() {
            let frame = constant_colour_mosaic(pattern, 16, 12);
            let out = debayer_superpixel(&frame, pattern).unwrap();

            assert_eq!((out.width(), out.height(), out.channels()), (8, 6, 3));
            for y in 0..out.height() {
                for x in 0..out.width() {
                    assert_eq!(out.get_pixel(x, y, 0), 0.2, "{pattern:?} red at {x},{y}");
                    assert_eq!(out.get_pixel(x, y, 1), 0.5, "{pattern:?} green at {x},{y}");
                    assert_eq!(out.get_pixel(x, y, 2), 0.8, "{pattern:?} blue at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn green_is_the_mean_of_the_two_green_samples() {
        let mut frame = Frame::zeros(2, 2, 1).unwrap();
        // RGGB: R G / G B
        frame.set_pixel(0, 0, 0, 0.1);
        frame.set_pixel(1, 0, 0, 0.4);
        frame.set_pixel(0, 1, 0, 0.6);
        frame.set_pixel(1, 1, 0, 0.9);

        let out = debayer_superpixel(&frame, CfaPattern::Rggb).unwrap();

        assert_eq!(out.get_pixel(0, 0, 0), 0.1);
        assert!((out.get_pixel(0, 0, 1) - 0.5).abs() < 1e-6);
        assert_eq!(out.get_pixel(0, 0, 2), 0.9);
    }

    #[test]
    fn an_odd_trailing_row_and_column_are_dropped() {
        let frame = constant_colour_mosaic(CfaPattern::Rggb, 9, 7);
        let out = debayer_superpixel(&frame, CfaPattern::Rggb).unwrap();
        assert_eq!((out.width(), out.height()), (4, 3));
    }

    #[test]
    fn a_frame_with_no_complete_quad_is_an_error() {
        let frame = Frame::zeros(1, 8, 1).unwrap();
        assert!(debayer_superpixel(&frame, CfaPattern::Rggb).is_err());
    }

    #[test]
    fn a_gradient_survives_the_binning_in_position() {
        let (width, height) = (32, 32);
        let mut frame = Frame::zeros(width, height, 1).unwrap();
        for y in 0..height {
            for x in 0..width {
                frame.set_pixel(x, y, 0, x as f32 / width as f32);
            }
        }
        let out = debayer_superpixel(&frame, CfaPattern::Rggb).unwrap();

        // Red comes from column 2x, blue from column 2x+1.
        for x in 0..out.width() {
            assert!((out.get_pixel(x, 0, 0) - (2 * x) as f32 / width as f32).abs() < 1e-6);
            assert!((out.get_pixel(x, 0, 2) - (2 * x + 1) as f32 / width as f32).abs() < 1e-6);
        }
    }
}
