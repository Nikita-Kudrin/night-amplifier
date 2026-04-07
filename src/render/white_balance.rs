//! White balance and background neutralization functions
//!
//! This module provides functions for neutralizing color casts from light pollution
//! or sensor bias by aligning per-channel medians.
//!
//! Supports both simple whole-image neutralization and advanced grid-based sampling
//! for robust background estimation in the presence of large nebulae or gradients.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::statistics::{compute_image_stats, fast_median, ImageStats};
use rayon::prelude::*;

/// Compute white balance multipliers from image statistics
///
/// This function calculates scaling multipliers that align the per-channel medians
/// to the average median, effectively neutralizing color casts from light pollution
/// or sensor bias.
///
/// # Arguments
/// * `stats` - Pre-computed image statistics with per-channel medians
///
/// # Returns
/// Array of [R, G, B] multipliers.
pub fn compute_neutralization_multipliers(stats: &ImageStats) -> Result<[f32; 3]> {
    if stats.channels.len() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: stats.channels.len(),
        });
    }

    let r_median = stats.channels[0].median;
    let g_median = stats.channels[1].median;
    let b_median = stats.channels[2].median;

    let avg_median = (r_median + g_median + b_median) / 3.0;
    let epsilon = 1e-6;

    let r_mult = if r_median > epsilon {
        avg_median / r_median
    } else {
        1.0
    };
    let g_mult = if g_median > epsilon {
        avg_median / g_median
    } else {
        1.0
    };
    let b_mult = if b_median > epsilon {
        avg_median / b_median
    } else {
        1.0
    };

    Ok([
        r_mult.clamp(0.5, 2.0),
        g_mult.clamp(0.5, 2.0),
        b_mult.clamp(0.5, 2.0),
    ])
}

/// Neutralize the sky background color in-place
pub fn neutralize_background(frame: &mut Frame, multipliers: &[f32; 3]) -> Result<()> {
    if frame.channels() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: frame.channels(),
        });
    }

    let data = frame.data_mut();

    data.par_chunks_mut(3).for_each(|pixel| {
        pixel[0] = (pixel[0] * multipliers[0]).clamp(0.0, 1.0);
        pixel[1] = (pixel[1] * multipliers[1]).clamp(0.0, 1.0);
        pixel[2] = (pixel[2] * multipliers[2]).clamp(0.0, 1.0);
    });

    Ok(())
}

/// Convenience function: neutralize background using computed image statistics
pub fn neutralize_background_auto(frame: &mut Frame) -> Result<[f32; 3]> {
    let stats = compute_image_stats(frame)?;
    let multipliers = compute_neutralization_multipliers(&stats)?;
    neutralize_background(frame, &multipliers)?;
    Ok(multipliers)
}

/// Compute white balance coefficients based on background sky color using grid-based sampling
///
/// This is more robust than simple medians as it samples local background blocks
/// and uses a percentile-based approach to ignore bright objects like nebulae.
///
/// # Arguments
/// * `frame` - The input RGB frame
/// * `grid_size` - Number of blocks per axis (e.g. 16 results in 256 samples)
/// * `percentile` - Background percentile to use (typ. 10.0-25.0)
///
/// # Returns
/// [R, G, B] multipliers
pub fn compute_white_balance_grid(
    frame: &Frame,
    grid_size: usize,
    percentile: f32,
) -> Result<[f32; 3]> {
    if frame.channels() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: frame.channels(),
        });
    }

    let width = frame.width();
    let height = frame.height();
    let grid_size = grid_size.max(1);

    let block_w = width / grid_size;
    let block_h = height / grid_size;

    if block_w == 0 || block_h == 0 {
        return Ok([1.0, 1.0, 1.0]);
    }

    let mut r_samples: Vec<f32> = Vec::with_capacity(grid_size * grid_size);
    let mut g_samples: Vec<f32> = Vec::with_capacity(grid_size * grid_size);
    let mut b_samples: Vec<f32> = Vec::with_capacity(grid_size * grid_size);

    for gy in 0..grid_size {
        for gx in 0..grid_size {
            let x_start = gx * block_w;
            let y_start = gy * block_h;
            let x_end = if gx == grid_size - 1 {
                width
            } else {
                x_start + block_w
            };
            let y_end = if gy == grid_size - 1 {
                height
            } else {
                y_start + block_h
            };

            let (r_med, g_med, b_med) = block_medians(frame, x_start, y_start, x_end, y_end);
            r_samples.push(r_med);
            g_samples.push(g_med);
            b_samples.push(b_med);
        }
    }

    let percentile_idx = ((percentile / 100.0) * r_samples.len() as f32) as usize;
    let percentile_idx = percentile_idx.min(r_samples.len().saturating_sub(1));

    r_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    g_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    b_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let r_bg = r_samples[percentile_idx];
    let g_bg = g_samples[percentile_idx];
    let b_bg = b_samples[percentile_idx];

    let reference = g_bg.max(1e-6);

    let r_coeff = if r_bg > 1e-6 { reference / r_bg } else { 1.0 };
    let g_coeff = 1.0;
    let b_coeff = if b_bg > 1e-6 { reference / b_bg } else { 1.0 };

    Ok([r_coeff.clamp(0.5, 2.0), g_coeff, b_coeff.clamp(0.5, 2.0)])
}

/// Median calculation helper for a small block
fn block_medians(
    frame: &Frame,
    x_start: usize,
    y_start: usize,
    x_end: usize,
    y_end: usize,
) -> (f32, f32, f32) {
    let capacity = (x_end - x_start) * (y_end - y_start);
    let mut r_vals = Vec::with_capacity(capacity);
    let mut g_vals = Vec::with_capacity(capacity);
    let mut b_vals = Vec::with_capacity(capacity);

    for y in y_start..y_end {
        for x in x_start..x_end {
            r_vals.push(frame.get_pixel(x, y, 0));
            g_vals.push(frame.get_pixel(x, y, 1));
            b_vals.push(frame.get_pixel(x, y, 2));
        }
    }

    (
        fast_median(&mut r_vals),
        fast_median(&mut g_vals),
        fast_median(&mut b_vals),
    )
}
