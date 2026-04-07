//! Black point calculation and subtraction functions
//!
//! This module provides functions for calculating and applying black point adjustments
//! to astronomical images based on robust statistics.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::statistics::{compute_image_stats, ChannelStats, ImageStats};
use rayon::prelude::*;

/// Configuration for black point calculation
#[derive(Debug, Clone, Copy)]
pub struct BlackPointConfig {
    /// Sigma factor (c) for black point calculation: BP = Median - (c × MAD-sigma)
    /// Lower values preserve more shadow detail, higher values clip more aggressively.
    /// Typical range: 1.5 to 3.0, default: 2.0
    pub sigma_factor: f32,
}

impl Default for BlackPointConfig {
    fn default() -> Self {
        Self { sigma_factor: 2.0 }
    }
}

impl BlackPointConfig {
    /// Create a new configuration with the specified sigma factor
    pub fn new(sigma_factor: f32) -> Self {
        Self { sigma_factor }
    }

    /// Use a conservative black point (preserves more shadow detail)
    /// Sets sigma_factor to 1.5
    pub fn conservative() -> Self {
        Self { sigma_factor: 1.5 }
    }

    /// Use an aggressive black point (darker sky, more clipping)
    /// Sets sigma_factor to 2.5
    pub fn aggressive() -> Self {
        Self { sigma_factor: 2.5 }
    }
}

/// Calculate the black point for a single channel using robust statistics
///
/// The black point establishes a safe lower limit for the image data, creating
/// a dark sky background without clipping into the noise floor.
///
/// # Formula
/// `BlackPoint = Mode - (c × Sigma)`
///
/// Where:
/// - `Mode` is the robust peak estimate of the sky background
/// - `Sigma` is the MAD-derived noise estimate (σ = 1.4826 × MAD)
/// - `c` is an adjustable constant (typically 1.5 to 2.5)
///
/// # Arguments
/// * `frame` - The image frame to analyze
/// * `channel_index` - The channel to compute the mode for
/// * `stats` - Channel statistics containing sigma
/// * `sigma_factor` - The constant c (1.5 = conservative, 2.5 = aggressive)
///
/// # Returns
/// The calculated black point, clamped to be non-negative
#[inline]
pub fn calculate_black_point(
    frame: &Frame,
    channel_index: usize,
    stats: &ChannelStats,
    sigma_factor: f32,
) -> f32 {
    let mode = estimate_channel_mode(frame, channel_index);
    (mode - sigma_factor * stats.sigma).max(0.0)
}

/// Finds the Mode (peak) of the image histogram for a specific channel
pub fn estimate_channel_mode(frame: &Frame, channel_index: usize) -> f32 {
    let data = frame.data();
    let channels = frame.channels();
    let mut histogram = vec![0u32; 65536];

    let num_pixels = data.len() / channels;
    let step = (num_pixels / 10000).max(1);

    for i in (0..num_pixels).step_by(step) {
        let idx = i * channels + channel_index;
        if idx < data.len() {
            let val = data[idx];
            let bin = (val * 65535.0) as usize;
            histogram[bin.clamp(0, 65535)] += 1;
        }
    }

    let mut max_count = 0;
    let mut peak_bin = 0;

    for (i, &count) in histogram.iter().enumerate().skip(10) {
        if count > max_count {
            max_count = count;
            peak_bin = i;
        }
    }

    peak_bin as f32 / 65535.0
}

/// Finds the Mode (peak) of the image histogram for luminance to accurately find the sky pedestal
/// This prevents large nebulae from skewing the background estimate.
///
/// Uses a smoothed histogram approach to find the true background peak, which is more robust against noise spikes.
pub fn estimate_background_mode(frame: &Frame) -> f32 {
    let data = frame.data();
    let channels = frame.channels();

    // Use 4096 bins for better precision while keeping it efficient
    const NUM_BINS: usize = 4096;
    let mut histogram = vec![0u32; NUM_BINS];

    let num_pixels = data.len() / channels;
    // Sample more pixels for better accuracy (up to 50k)
    let step = (num_pixels / 50000).max(1);

    if channels == 3 {
        for i in (0..num_pixels).step_by(step) {
            let idx = i * 3;
            if idx + 2 < data.len() {
                let lum = 0.2126 * data[idx] + 0.7152 * data[idx + 1] + 0.0722 * data[idx + 2];
                let bin = (lum * (NUM_BINS - 1) as f32) as usize;
                histogram[bin.clamp(0, NUM_BINS - 1)] += 1;
            }
        }
    } else {
        for i in (0..num_pixels).step_by(step) {
            let lum = data[i];
            let bin = (lum * (NUM_BINS - 1) as f32) as usize;
            histogram[bin.clamp(0, NUM_BINS - 1)] += 1;
        }
    }

    // Apply a simple box smoothing (kernel size 5) to reduce noise spikes
    let mut smoothed = vec![0u32; NUM_BINS];
    for i in 2..(NUM_BINS - 2) {
        smoothed[i] = (histogram[i - 2]
            + histogram[i - 1]
            + histogram[i]
            + histogram[i + 1]
            + histogram[i + 2])
            / 5;
    }

    // Find the peak in the lower portion of the histogram (background is typically dark)
    // Only search up to 30% of the histogram range to avoid bright objects
    let search_limit = NUM_BINS * 3 / 10;
    let mut max_count = 0;
    let mut peak_bin = 0;

    // Skip the very first bins (potential sensor artifacts/hot pixels)
    for (i, &count) in smoothed.iter().enumerate().skip(5).take(search_limit) {
        if count > max_count {
            max_count = count;
            peak_bin = i;
        }
    }

    // If no clear peak found in dark region, use median approach
    if max_count == 0 {
        // Fallback to finding median of the histogram
        let total: u32 = histogram.iter().sum();
        let half = total / 2;
        let mut cumsum = 0u32;
        for (i, &count) in histogram.iter().enumerate() {
            cumsum += count;
            if cumsum >= half {
                peak_bin = i;
                break;
            }
        }
    }

    peak_bin as f32 / (NUM_BINS - 1) as f32
}

/// Calculate per-channel black points from image statistics
///
/// Returns an array of [R, G, B] black points calculated using the formula:
/// `BP[c] = Mode[c] - (sigma_factor × Sigma[c])`
///
/// # Arguments
/// * `frame` - The image frame to analyze
/// * `stats` - Pre-computed image statistics
/// * `config` - Black point configuration with sigma factor
///
/// # Returns
/// Array of per-channel black points
pub fn calculate_black_points(
    frame: &Frame,
    stats: &ImageStats,
    config: BlackPointConfig,
) -> Result<[f32; 3]> {
    if stats.channels.len() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: stats.channels.len(),
        });
    }

    Ok([
        calculate_black_point(frame, 0, &stats.channels[0], config.sigma_factor),
        calculate_black_point(frame, 1, &stats.channels[1], config.sigma_factor),
        calculate_black_point(frame, 2, &stats.channels[2], config.sigma_factor),
    ])
}

/// Calculate a single (luminance-based) black point for all channels
///
/// Uses the average statistics across channels to compute a single black point.
/// This is useful when you want consistent black level across all channels
/// to avoid color shifts in the shadows.
///
/// # Arguments
/// * `frame` - The image frame to analyze
/// * `stats` - Pre-computed image statistics
/// * `config` - Black point configuration with sigma factor
///
/// # Returns
/// A single black point value to apply to all channels
pub fn calculate_luminance_black_point(
    frame: &Frame,
    stats: &ImageStats,
    config: BlackPointConfig,
) -> f32 {
    let mode = estimate_background_mode(frame);
    let mean_sigma = stats.mean_sigma();
    (mode - config.sigma_factor * mean_sigma).max(0.0)
}

/// Subtract black point from the entire image buffer in-place
///
/// This function subtracts the per-channel black points from every pixel,
/// clamping any negative values to 0.0. After this operation:
/// - Sky background will be near zero (dark)
/// - All pixel values remain in the valid [0.0, 1.0] range
/// - Signal above the black point is preserved
///
/// **Important**: Apply this BEFORE stretching. The stretch function expects
/// data where 0.0 represents the intended black level.
///
/// # Arguments
/// * `frame` - Mutable reference to an RGB frame (will be modified in-place)
/// * `black_points` - Per-channel black points from `calculate_black_points`
pub fn subtract_black_point(frame: &mut Frame, black_points: &[f32; 3]) -> Result<()> {
    if frame.channels() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: frame.channels(),
        });
    }

    let data = frame.data_mut();

    data.par_chunks_mut(3).for_each(|pixel| {
        pixel[0] = (pixel[0] - black_points[0]).clamp(0.0, 1.0);
        pixel[1] = (pixel[1] - black_points[1]).clamp(0.0, 1.0);
        pixel[2] = (pixel[2] - black_points[2]).clamp(0.0, 1.0);
    });

    Ok(())
}

/// Subtract a uniform black point from all channels in-place
///
/// Uses a single black point value for all channels, which preserves
/// color balance in the shadows better than per-channel subtraction.
///
/// # Arguments
/// * `frame` - Mutable reference to a frame (any number of channels)
/// * `black_point` - The black point value to subtract from all pixels
pub fn subtract_black_point_uniform(frame: &mut Frame, black_point: f32) -> Result<()> {
    let data = frame.data_mut();

    data.par_iter_mut().for_each(|v| {
        *v = (*v - black_point).clamp(0.0, 1.0);
    });

    Ok(())
}

/// Convenience function: calculate and subtract black point automatically
///
/// This is a one-shot function that computes statistics, calculates the black
/// point, and applies it in a single call.
///
/// # Arguments
/// * `frame` - Mutable reference to an RGB frame (will be modified in-place)
/// * `config` - Black point configuration (use `BlackPointConfig::default()` for typical use)
///
/// # Returns
/// The calculated per-channel black points (useful for logging/debugging)
pub fn subtract_black_point_auto(frame: &mut Frame, config: BlackPointConfig) -> Result<[f32; 3]> {
    let stats = compute_image_stats(frame)?;
    let black_points = calculate_black_points(frame, &stats, config)?;
    subtract_black_point(frame, &black_points)?;
    Ok(black_points)
}

#[cfg(test)]
mod tests {
    include!("black_point_tests.rs");
}
