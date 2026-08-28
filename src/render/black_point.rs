//! Black point calculation and subtraction functions
//!
//! This module provides functions for calculating and applying black point adjustments
//! to astronomical images based on robust statistics.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::subtract_scalar_clamp_simd;
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
///
/// The scan is bounded to the bins that actually received a sample. Only ~10 000 samples
/// go into a 65 536-bin histogram, so walking all of it spent six times more work
/// deciding that empty bins were empty than it did filling them — three times per frame,
/// once per channel. The answer is unchanged: an untouched bin holds 0, and `count >
/// max_count` starting from 0 can never select one.
pub fn estimate_channel_mode(frame: &Frame, channel_index: usize) -> f32 {
    let data = frame.channel_data(channel_index);
    let mut histogram = vec![0u32; 65536];

    let step = (data.len() / 10000).max(1);

    let mut lowest_bin = usize::MAX;
    let mut highest_bin = 0usize;

    for i in (0..data.len()).step_by(step) {
        let val = data[i];
        let bin = ((val * 65535.0) as usize).clamp(0, 65535);
        histogram[bin] += 1;
        lowest_bin = lowest_bin.min(bin);
        highest_bin = highest_bin.max(bin);
    }

    let mut max_count = 0;
    let mut peak_bin = 0;

    // `skip(10)` in bin terms: the first ten bins are excluded as sensor floor.
    let scan_start = lowest_bin.max(10);
    if scan_start <= highest_bin {
        for (offset, &count) in histogram[scan_start..=highest_bin].iter().enumerate() {
            if count > max_count {
                max_count = count;
                peak_bin = scan_start + offset;
            }
        }
    }

    peak_bin as f32 / 65535.0
}

/// Background mode plus the luminance samples it was derived from.
///
/// Carrying the samples lets callers compute further luminance statistics — notably
/// `estimate_signal_fraction` — without walking the full frame a second time. The
/// samples are what the mode was actually measured on, so derived statistics are
/// consistent with it by construction.
#[derive(Debug, Clone)]
pub struct BackgroundEstimate {
    /// Mode (peak) of the luminance histogram: the sky pedestal.
    pub mode: f32,
    /// Sampled luminances, in frame order. Roughly 50k entries.
    pub luminance_samples: Vec<f32>,
}

/// Finds the Mode (peak) of the image histogram for luminance to accurately find the sky pedestal
/// This prevents large nebulae from skewing the background estimate.
///
/// Uses a smoothed histogram approach to find the true background peak, which is more robust against noise spikes.
/// Returns the mode together with the luminance samples it was computed from, so callers
/// needing further luminance statistics do not have to traverse the frame again.
pub fn estimate_background_mode(frame: &Frame) -> BackgroundEstimate {
    let channels = frame.channels();
    let num_pixels = frame.width() * frame.height();

    // Use 4096 bins for better precision while keeping it efficient
    const NUM_BINS: usize = 4096;
    let mut histogram = vec![0u32; NUM_BINS];

    // Sample more pixels for better accuracy (up to 50k)
    let step = (num_pixels / 50000).max(1);
    let mut luminance_samples = Vec::with_capacity(num_pixels / step + 1);

    if channels == 3 {
        let (r, g, b) = frame.planes();
        for i in (0..num_pixels).step_by(step) {
            let lum = 0.2126 * r[i] + 0.7152 * g[i] + 0.0722 * b[i];
            let bin = (lum * (NUM_BINS - 1) as f32) as usize;
            histogram[bin.clamp(0, NUM_BINS - 1)] += 1;
            luminance_samples.push(lum);
        }
    } else {
        let data = frame.channel_data(0);
        for i in (0..num_pixels).step_by(step) {
            let lum = data[i];
            let bin = (lum * (NUM_BINS - 1) as f32) as usize;
            histogram[bin.clamp(0, NUM_BINS - 1)] += 1;
            luminance_samples.push(lum);
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

    BackgroundEstimate {
        mode: peak_bin as f32 / (NUM_BINS - 1) as f32,
        luminance_samples,
    }
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

    let mut bps = [0.0; 3];
    bps.par_iter_mut().enumerate().for_each(|(i, bp)| {
        *bp = calculate_black_point(frame, i, &stats.channels[i], config.sigma_factor);
    });

    Ok(bps)
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
    let mode = estimate_background_mode(frame).mode;
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

    // Per plane, then chunked within the plane. Recovering the channel from a flat
    // chunk index instead would require the chunk length to divide the plane size, and
    // hunting for such a length is what the old `plane_chunk_len` did — at up to 4.9 ms
    // per call, and with no parallelism left when the plane size had no convenient
    // divisor.
    let chunk = crate::parallel::balanced_chunk_len(frame.pixel_count());
    let offsets = *black_points;
    let (r, g, b) = frame.planes_mut();

    [(r, offsets[0]), (g, offsets[1]), (b, offsets[2])]
        .into_par_iter()
        .for_each(|(plane, offset)| {
            plane
                .par_chunks_mut(chunk)
                .for_each(|block| subtract_scalar_clamp_simd(block, offset));
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
    // The autostretch solver returns exactly 0.0 whenever the adjusted median is
    // negligible, which is common, and subtracting zero from in-range pixel data is a
    // no-op. Guard on equality only: a negative black point still has to run, since
    // there it brightens the frame rather than doing nothing.
    if black_point == 0.0 {
        return Ok(());
    }

    // A uniform offset over a planar buffer is one flat elementwise pass: the plane
    // boundaries do not matter, so there is nothing to split by channel. Planar layout
    // makes this simpler than interleaved did, not more complex.
    let chunk = crate::parallel::balanced_chunk_len(frame.sample_count());
    frame
        .data_mut()
        .par_chunks_mut(chunk)
        .for_each(|c| subtract_scalar_clamp_simd(c, black_point));

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
