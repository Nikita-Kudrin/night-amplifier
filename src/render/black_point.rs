//! Black point calculation and subtraction functions
//!
//! This module provides functions for calculating and applying black point adjustments
//! to astronomical images based on robust statistics.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::subtract_scalar_clamp_simd;
use crate::statistics::{compute_image_stats, select_median, select_nth, ChannelStats, ImageStats};
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

/// Calculate the black point for a single channel using robust statistics: a safe
/// lower limit for the image data, giving a dark sky background without clipping
/// into the noise floor. `BlackPoint = Mode - (c × Sigma)`, where `Mode` is the
/// robust sky-background peak estimate, `Sigma = 1.4826 × MAD`, and `c` is
/// `sigma_factor` (1.5 conservative .. 2.5 aggressive). Result is clamped to
/// non-negative.
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

    // Skip the very first bins (potential sensor artifacts/hot pixels).
    //
    // Ties break on the raw histogram, and that is load-bearing rather than tidy: the
    // five-wide box above turns a single-bin spike into a five-bin *plateau*, so taking
    // the first strict maximum lands two bins below the sky. A deep stack is exactly that
    // spike — sigma 1.3 ADU inside a 16 ADU bin — and two bins is far enough that
    // `refine_peak`'s window misses the samples entirely and falls back to the bin value
    // it exists to replace, 43 ADU low. On a plateau the raw counts are unambiguous: the
    // true bin holds every sample and its neighbours hold none. Swept across a bin in
    // tenths this takes the worst error from 43.21 ADU to 0.07.
    for (i, &count) in smoothed.iter().enumerate().skip(5).take(search_limit) {
        if count > max_count || (count == max_count && histogram[i] > histogram[peak_bin]) {
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

    // Refine the binned peak against the samples themselves.
    //
    // A bin is 1/4095 of full scale — 16 ADU of a 16-bit frame — while a 71-frame
    // stack's sky sigma is 2.2 ADU. The whole sky distribution fits in a fifth of a
    // bin, so the binned peak is a step function of stack depth: it sat on bin 10 for
    // 50 frames and snapped to bin 11 at 71, moving the black point (`mode - k*sigma`)
    // by 16 ADU against a target only 30 ADU above sky. Half the nebula went below
    // black in one frame. Bin selection stays (it is what rejects nebulosity); only
    // the value returned is refined, by re-binning the samples around the winner.
    let bin_width = 1.0 / (NUM_BINS - 1) as f32;
    let window_lo = (peak_bin as f32 - 1.5) * bin_width;
    let peak = refine_peak(&luminance_samples, window_lo, WINDOW_BINS as f32 * bin_width)
        .unwrap_or(peak_bin as f32 / (NUM_BINS - 1) as f32);

    // Then take the level from the samples the peak points at, rather than from the
    // peak itself. Both histogram passes pick a *bin*, and on a deep stack the sky is
    // narrow enough that the winning sub-bin stops moving with the data: on a 106-sub
    // IMX533 session the peak returned bit-identical values at 32, 64 and 106 frames
    // and sat 1.2 ADU above the sky, having jumped 2.1 ADU between 16 and 32 while the
    // sky itself moved 0.2. Displayed, that was the background dropping four output
    // levels in one stack update — in a dark eyepiece, the whole field pumping. A
    // median over a window is continuous in the samples, so it cannot snap.
    let mode = clipped_centre(&luminance_samples, peak).unwrap_or(peak);

    BackgroundEstimate {
        mode,
        luminance_samples,
    }
}

/// Half-width of the window the sky level is taken from, in robust sigmas of the
/// luminance samples. Wide enough to hold the sky's own distribution, tight enough to
/// leave the stars and the target outside it.
const CENTRE_WINDOW_SIGMAS: f32 = 2.5;

/// Samples that must fall in that window before its median is trusted over the peak.
const CENTRE_MIN_SAMPLES: usize = 64;

/// Samples the spread and the centre are read from. Both converge long before this:
/// 8 192 samples place a median to `1.25 * sigma / sqrt(n)`, 0.02 ADU on a deep stack.
/// Above it the two selections cost more than the refinement they follow.
const CENTRE_MAX_SAMPLES: usize = 8192;

/// Quantile of the one-sided spread the window is scaled from.
///
/// The **lower quartile**, not the median, and that is what makes the window robust in
/// both directions. The samples below the peak are ordered by how far below they sit,
/// so a population darker than the sky — a corner the background model over-subtracted
/// and clamped to zero, a vignette, a partly illuminated frame — piles up at the *far*
/// end of that list. A median only survives while such a population is under half of
/// it: measured on a sky with a share of the frame clipped to zero, the estimate went
/// -1.43 ADU off at 40 %, -4.95 at 50 % and -150.73 at 60 % — the sky level itself, the
/// window having grown wide enough to swallow the zeros and put its median among them.
/// The quartile survives to 60 % and costs nothing where there is no contaminant:
/// swept over sky sigmas from 0.06 to 8.2 histogram bins the estimate stays within
/// 0.01 sigma of the sky, the same as the median gave.
///
/// A half-normal's quartile is `0.3186 * sigma` (its median is `0.6745`, the MAD
/// constant), hence [`CENTRE_QUARTILE_TO_SIGMA`].
const CENTRE_SPREAD_QUANTILE: f32 = 0.25;

/// Turns that quartile into a sigma: `1 / 0.31864`.
const CENTRE_QUARTILE_TO_SIGMA: f32 = 3.1383;

/// Median of the samples within `CENTRE_WINDOW_SIGMAS` of `peak`.
///
/// The spread comes from the samples **below** the peak only. A spread over every
/// sample is robust while what contaminates it is a minority, and a target filling the
/// frame is not: the window then sizes itself around the *target's* spread, swallows
/// it, and the median lands on the target. Measured on a halo covering 69 % of the
/// frame the answer went 1.4 ADU off the sky to 7.3; on a 75 % ramp, 349. A target is
/// brighter than the sky it sits on, so the sky's lower half is the half it cannot
/// reach. What *can* reach it is a population darker than the sky — see
/// [`CENTRE_SPREAD_QUANTILE`], which is why the spread is a quartile rather than a
/// median.
///
/// Selection, not `statistics::fast_median`, which `par_sort_unstable`s anything above
/// 4 096 — three of those per frame took `estimate_background_mode` from 0.50 ms to
/// 1.62 ms, above the sort `refine_peak` exists to avoid.
fn clipped_centre(samples: &[f32], peak: f32) -> Option<f32> {
    if samples.len() < CENTRE_MIN_SAMPLES {
        return None;
    }
    let stride = (samples.len() / CENTRE_MAX_SAMPLES).max(1);
    let mut below: Vec<f32> = samples
        .iter()
        .step_by(stride)
        .filter(|&&v| v <= peak)
        .map(|&v| peak - v)
        .collect();
    if below.len() < CENTRE_MIN_SAMPLES {
        return None;
    }
    let quartile = (below.len() as f32 * CENTRE_SPREAD_QUANTILE) as usize;
    let sigma = select_nth(&mut below, quartile) * CENTRE_QUARTILE_TO_SIGMA;
    if !(sigma > 0.0) || !sigma.is_finite() {
        return None;
    }

    let window = CENTRE_WINDOW_SIGMAS * sigma;
    let mut kept: Vec<f32> = samples
        .iter()
        .step_by(stride)
        .copied()
        .filter(|v| (v - peak).abs() <= window)
        .collect();
    if kept.len() < CENTRE_MIN_SAMPLES {
        return None;
    }
    Some(select_median(&mut kept))
}

/// Coarse bins spanned by the refinement window: the winning bin, one below, two above.
const WINDOW_BINS: usize = 4;

/// Sub-bins across that window. 512 over four coarse bins resolves 0.13 ADU of a 16-bit
/// frame, so even a 71-frame sky (sigma ~2.2 ADU) is spread over ~17 of them — enough to
/// locate a peak, where the coarse histogram had the whole distribution inside one bin.
const REFINE_BINS: usize = 512;

/// Fewer samples than this in the window and the sub-histogram is noise; the caller
/// falls back to the coarse bin centre.
const REFINE_MIN_SAMPLES: u32 = 64;

/// Mode of the samples falling in `[lo, lo + width)`, to sub-bin precision.
///
/// A second histogram rather than a sort: sorting the ~50 000 samples that land in one
/// coarse bin and taking their half-sample mode gave the same answer but measured
/// 1.40 ms against 0.39 ms for the binned original (`black_point_benchmark`), and this
/// runs on every preview frame. Re-binning is one pass and a fixed 512-entry scan.
///
/// The peak is smoothed over five sub-bins and interpolated parabolically, so the result
/// moves continuously with the sky rather than snapping — which is the entire point of
/// the refinement.
fn refine_peak(samples: &[f32], lo: f32, width: f32) -> Option<f32> {
    if width <= 0.0 {
        return None;
    }
    let scale = REFINE_BINS as f32 / width;
    let mut hist = [0u32; REFINE_BINS];
    let mut total = 0u32;
    for &v in samples {
        let offset = v - lo;
        if offset < 0.0 {
            continue;
        }
        let bin = (offset * scale) as usize;
        if bin < REFINE_BINS {
            hist[bin] += 1;
            total += 1;
        }
    }
    if total < REFINE_MIN_SAMPLES {
        return None;
    }

    // Same five-wide box the coarse pass uses, for the same reason: a single sub-bin
    // spike is noise, not the sky.
    let smoothed: Vec<u32> = (0..REFINE_BINS)
        .map(|i| {
            let start = i.saturating_sub(2);
            let end = (i + 2).min(REFINE_BINS - 1);
            hist[start..=end].iter().sum::<u32>() / (end - start + 1) as u32
        })
        .collect();

    let peak = (0..REFINE_BINS).max_by_key(|&i| smoothed[i])?;
    let offset = if peak > 0 && peak < REFINE_BINS - 1 {
        let (a, b, c) = (
            smoothed[peak - 1] as f32,
            smoothed[peak] as f32,
            smoothed[peak + 1] as f32,
        );
        let denom = a - 2.0 * b + c;
        if denom.abs() > f32::EPSILON {
            (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
        } else {
            0.0
        }
    } else {
        0.0
    };

    Some(lo + (peak as f32 + 0.5 + offset) / scale)
}

/// Calculate per-channel black points from image statistics: returns [R, G, B],
/// each `BP[c] = Mode[c] - (sigma_factor × Sigma[c])`.
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

/// Calculate a single (luminance-based) black point for all channels, from the
/// average statistics across channels — gives a consistent black level and avoids
/// colour shifts in the shadows.
pub fn calculate_luminance_black_point(
    frame: &Frame,
    stats: &ImageStats,
    config: BlackPointConfig,
) -> f32 {
    let mode = estimate_background_mode(frame).mode;
    let mean_sigma = stats.mean_sigma();
    (mode - config.sigma_factor * mean_sigma).max(0.0)
}

/// Subtract black point from the entire image buffer in-place: per-channel black
/// points subtracted from every pixel, clamped to [0.0, 1.0] — sky background goes
/// near zero, signal above it is preserved. **Apply BEFORE stretching**: the stretch
/// function expects data where 0.0 is the intended black level.
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

/// Convenience one-shot: computes statistics, calculates the black point, and
/// applies it in a single call. Returns the per-channel black points (for
/// logging/debugging).
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
