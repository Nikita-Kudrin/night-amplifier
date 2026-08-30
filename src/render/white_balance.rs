//! White balance and background neutralization functions
//!
//! This module provides functions for neutralizing color casts from light pollution
//! or sensor bias by aligning per-channel medians.
//!
//! Supports both simple whole-image neutralization and advanced grid-based sampling
//! for robust background estimation in the presence of large nebulae or gradients.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::multiply_scalar_clamp_simd;
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

    // Per plane, then chunked within the plane — see `subtract_black_point` for why
    // the chunk length is not required to divide the plane size.
    let chunk = crate::parallel::balanced_chunk_len(frame.pixel_count());
    let muls = *multipliers;
    let (r, g, b) = frame.planes_mut();

    [(r, muls[0]), (g, muls[1]), (b, muls[2])]
        .into_par_iter()
        .for_each(|(plane, mul)| {
            plane
                .par_chunks_mut(chunk)
                .for_each(|block| multiply_scalar_clamp_simd(block, mul));
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

/// How much of each grid block [`compute_white_balance_grid_with_config`] reads.
///
/// A block median is a *background estimate*, and reading all ~65 000 pixels of a block
/// to produce one number is the same over-sampling [`crate::statistics::StatsConfig`]
/// already caps for whole-frame statistics (100 000 samples per channel by default,
/// against 4.2 M pixels).
///
/// Measured on a 2712x1538x3 frame at grid 16, 20 cores, release: [`Self::exact`]
/// 28 ms, `sampled(4096)` — a stride of 2 over the 169x96 block — 1.7 ms. For reference
/// the `get_pixel`-per-sample, single-threaded predecessor took 120 ms, so `exact` is
/// already the 4.4x win and sampling is a further 16x on top of it.
///
/// The budget is per block, not per frame, so it adapts: a small frame or a fine grid
/// makes the blocks smaller than the budget and takes the exact path on its own. Pick
/// the value against the block size the call site will actually see —
/// `sampled(16_384)` looks aggressive but is a no-op at the resolution above, because
/// the block is only 16 224 pixels.
///
/// The default is [`Self::exact`] deliberately: sampling moves the coefficients (~1e-3
/// on a noise field, more on a structured one) and this runs on the live preview path,
/// so a call site opts into that trade rather than inheriting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhiteBalanceConfig {
    /// Upper bound on samples read per block, per channel.
    ///
    /// `usize::MAX` reads every pixel. Anything smaller is turned into a stride applied
    /// to both axes, so the samples stay spread over the whole block rather than
    /// clustering in the top rows.
    pub max_samples_per_block: usize,
}

impl Default for WhiteBalanceConfig {
    fn default() -> Self {
        Self::exact()
    }
}

impl WhiteBalanceConfig {
    /// Read every pixel of every block.
    pub const fn exact() -> Self {
        Self {
            max_samples_per_block: usize::MAX,
        }
    }

    /// Read at most `max_samples_per_block` samples from each block.
    pub const fn sampled(max_samples_per_block: usize) -> Self {
        Self {
            max_samples_per_block,
        }
    }

    /// Stride that keeps a `block_w` x `block_h` block within the sample budget.
    ///
    /// The same stride is used on both axes, so `ceil(w/s) * ceil(h/s) <= budget` is
    /// satisfied by `s = ceil(sqrt(area / budget))`. Returns 1 for the exact case,
    /// which is the branch that gets the contiguous `extend_from_slice` copy.
    fn stride_for(self, block_w: usize, block_h: usize) -> usize {
        let budget = self.max_samples_per_block.max(1);
        let area = block_w * block_h;
        if budget >= area {
            return 1;
        }
        ((area as f64 / budget as f64).sqrt().ceil() as usize).max(1)
    }
}

/// Compute white balance coefficients based on background sky color using grid-based sampling
///
/// This is more robust than simple medians as it samples local background blocks
/// and uses a percentile-based approach to ignore bright objects like nebulae.
///
/// Reads every pixel of every block; see [`compute_white_balance_grid_with_config`] to
/// trade exactness for speed.
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
    compute_white_balance_grid_with_config(
        frame,
        grid_size,
        percentile,
        WhiteBalanceConfig::exact(),
    )
}

/// [`compute_white_balance_grid`] with control over how much of each block is read.
///
/// # Why this is parallel per block rather than a nested scan
///
/// The predecessor walked `grid_size^2` blocks in sequence, reaching every sample
/// through `Frame::get_pixel` and allocating three `Vec`s per block — 768 allocations
/// and 12.5 M bounds-checked index computations for a sensor-shaped frame, on one
/// thread, once per preview frame. It measured 120 ms against 15 ms for the whole rest
/// of the preview pipeline put together.
///
/// Planar layout is what makes the fix free. A block's row is a contiguous run inside
/// its plane (`plane[y * width + x0 .. y * width + x1]`), so the gather is an
/// `extend_from_slice` rather than a per-sample gather, and the blocks are independent,
/// so they are one rayon dispatch. `map_init` keeps one scratch buffer per worker
/// instead of one allocation per block.
pub fn compute_white_balance_grid_with_config(
    frame: &Frame,
    grid_size: usize,
    percentile: f32,
    config: WhiteBalanceConfig,
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

    let planes = frame.planes();
    let stride = config.stride_for(block_w, block_h);

    // `collect` preserves order for every rayon iterator, indexed or not, so the block
    // ordering — and therefore the sample vectors — match the sequential version exactly.
    let samples: Vec<[f32; 3]> = (0..grid_size * grid_size)
        .into_par_iter()
        .map_init(BlockScratch::default, |scratch, block| {
            let (gy, gx) = (block / grid_size, block % grid_size);
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

            block_medians(
                planes,
                width,
                (x_start, y_start, x_end, y_end),
                stride,
                scratch,
            )
        })
        // Filter out empty blocks (caused by registration artifacts at borders)
        .filter(|m| m[0] > 0.0 || m[1] > 0.0 || m[2] > 0.0)
        .collect();

    // If all blocks were empty (unlikely but possible), fallback to 1.0
    if samples.is_empty() {
        return Ok([1.0, 1.0, 1.0]);
    }

    let percentile_idx = ((percentile / 100.0) * samples.len() as f32) as usize;
    let percentile_idx = percentile_idx.min(samples.len() - 1);

    let channel_percentile = |c: usize| {
        let mut vals: Vec<f32> = samples.iter().map(|m| m[c]).collect();
        // One order statistic is wanted, so this is O(n) rather than O(n log n), and it
        // keeps the `unwrap_or(Equal)` that makes a NaN sample sort rather than panic.
        vals.select_nth_unstable_by(percentile_idx, |a, b| {
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
        });
        vals[percentile_idx]
    };

    let r_bg = channel_percentile(0);
    let g_bg = channel_percentile(1);
    let b_bg = channel_percentile(2);

    let reference = g_bg.max(1e-6);

    let r_coeff = if r_bg > 1e-6 { reference / r_bg } else { 1.0 };
    let g_coeff = 1.0;
    let b_coeff = if b_bg > 1e-6 { reference / b_bg } else { 1.0 };

    Ok([r_coeff.clamp(0.5, 2.0), g_coeff, b_coeff.clamp(0.5, 2.0)])
}

/// Per-worker scratch for [`block_medians`], reused across every block a worker handles.
#[derive(Default)]
struct BlockScratch {
    r: Vec<f32>,
    g: Vec<f32>,
    b: Vec<f32>,
}

/// Median of one grid block, per channel.
///
/// `planes` are the frame's three planes and `width` its row stride, so a block row is
/// the contiguous run `plane[y * width + x0 .. y * width + x1]`.
fn block_medians(
    planes: (&[f32], &[f32], &[f32]),
    width: usize,
    bounds: (usize, usize, usize, usize),
    stride: usize,
    scratch: &mut BlockScratch,
) -> [f32; 3] {
    let (x_start, y_start, x_end, y_end) = bounds;
    let (rp, gp, bp) = planes;

    scratch.r.clear();
    scratch.g.clear();
    scratch.b.clear();

    if stride == 1 {
        for y in y_start..y_end {
            let row = y * width;
            scratch.r.extend_from_slice(&rp[row + x_start..row + x_end]);
            scratch.g.extend_from_slice(&gp[row + x_start..row + x_end]);
            scratch.b.extend_from_slice(&bp[row + x_start..row + x_end]);
        }
    } else {
        for y in (y_start..y_end).step_by(stride) {
            let row = y * width;
            for x in (x_start..x_end).step_by(stride) {
                scratch.r.push(rp[row + x]);
                scratch.g.push(gp[row + x]);
                scratch.b.push(bp[row + x]);
            }
        }
    }

    [
        fast_median(&mut scratch.r),
        fast_median(&mut scratch.g),
        fast_median(&mut scratch.b),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct value per (x, y, channel) with a colour cast and a gradient, so a
    /// coefficient that came from the wrong plane, the wrong block or the wrong rows
    /// cannot match. Built with `set_pixel` so the fixture cannot encode the layout.
    fn cast_gradient_frame(width: usize, height: usize) -> Frame {
        let mut frame = Frame::zeros(width, height, 3).unwrap();
        let mut seed = 0x5EED_1234u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        for y in 0..height {
            for x in 0..width {
                // A light-pollution-shaped gradient plus per-channel offsets, so the
                // three channels have genuinely different backgrounds.
                let grad = 0.02 + 0.06 * (x as f32 / width as f32) + 0.03 * (y as f32 / height as f32);
                let noise = rand() * 0.004;
                frame.set_pixel(x, y, 0, grad * 1.30 + noise);
                frame.set_pixel(x, y, 1, grad + noise);
                frame.set_pixel(x, y, 2, grad * 0.80 + noise);
            }
        }
        // A bright object the percentile is supposed to reject.
        for y in height / 3..height / 2 {
            for x in width / 3..width / 2 {
                frame.set_pixel(x, y, 0, 0.9);
                frame.set_pixel(x, y, 1, 0.9);
                frame.set_pixel(x, y, 2, 0.9);
            }
        }
        frame
    }

    /// The pre-rewrite algorithm, written straight: a sequential scan reaching every
    /// sample through `get_pixel`, a `sort_by` for the percentile. Independent of the
    /// parallel planar implementation, so an indexing or ordering slip in either shows
    /// up as a disagreement.
    fn reference_white_balance_grid(frame: &Frame, grid_size: usize, percentile: f32) -> [f32; 3] {
        let (width, height) = (frame.width(), frame.height());
        let grid_size = grid_size.max(1);
        let (block_w, block_h) = (width / grid_size, height / grid_size);
        if block_w == 0 || block_h == 0 {
            return [1.0, 1.0, 1.0];
        }

        let mut samples: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for gy in 0..grid_size {
            for gx in 0..grid_size {
                let x_start = gx * block_w;
                let y_start = gy * block_h;
                let x_end = if gx == grid_size - 1 { width } else { x_start + block_w };
                let y_end = if gy == grid_size - 1 { height } else { y_start + block_h };

                let mut med = [0.0f32; 3];
                for (c, m) in med.iter_mut().enumerate() {
                    let mut vals = Vec::new();
                    for y in y_start..y_end {
                        for x in x_start..x_end {
                            vals.push(frame.get_pixel(x, y, c));
                        }
                    }
                    *m = fast_median(&mut vals);
                }

                if med[0] > 0.0 || med[1] > 0.0 || med[2] > 0.0 {
                    for c in 0..3 {
                        samples[c].push(med[c]);
                    }
                }
            }
        }

        if samples[0].is_empty() {
            return [1.0, 1.0, 1.0];
        }

        let idx = (((percentile / 100.0) * samples[0].len() as f32) as usize)
            .min(samples[0].len() - 1);
        for s in samples.iter_mut() {
            s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        }

        let (r_bg, g_bg, b_bg) = (samples[0][idx], samples[1][idx], samples[2][idx]);
        let reference = g_bg.max(1e-6);
        [
            if r_bg > 1e-6 { reference / r_bg } else { 1.0 }.clamp(0.5, 2.0),
            1.0,
            if b_bg > 1e-6 { reference / b_bg } else { 1.0 }.clamp(0.5, 2.0),
        ]
    }

    /// The contract the planar/parallel rewrite has to hold: identical output, not
    /// merely close output. Exercised over grid sizes whose blocks do and do not divide
    /// the frame, since the last row and column of blocks absorb the remainder.
    #[test]
    fn grid_white_balance_matches_the_sequential_reference() {
        for (w, h) in [(256usize, 192usize), (250, 190), (97, 61)] {
            let frame = cast_gradient_frame(w, h);
            for grid in [4usize, 8, 16] {
                for pct in [10.0f32, 25.0, 50.0] {
                    let got = compute_white_balance_grid(&frame, grid, pct).unwrap();
                    let want = reference_white_balance_grid(&frame, grid, pct);
                    assert_eq!(
                        got, want,
                        "{w}x{h} grid={grid} pct={pct}: {got:?} != reference {want:?}"
                    );
                }
            }
        }
    }

    /// The rewrite is one rayon dispatch over blocks; how rayon splits it must not
    /// change the answer.
    #[test]
    fn grid_white_balance_is_invariant_to_thread_count() {
        let frame = cast_gradient_frame(250, 190);
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| compute_white_balance_grid(&frame, 16, 25.0).unwrap())
        };
        assert_eq!(run(1), run(8));
    }

    /// A cast the coefficients must actually correct: red high, blue low, green the
    /// reference. Pins the direction, which an equivalence test alone cannot.
    #[test]
    fn grid_white_balance_corrects_the_cast_it_measures() {
        let frame = cast_gradient_frame(256, 192);
        let [r, g, b] = compute_white_balance_grid(&frame, 16, 25.0).unwrap();
        assert_eq!(g, 1.0, "green is the reference channel");
        assert!(r < 1.0, "red is 1.3x green in the fixture, so it must be scaled down: {r}");
        assert!(b > 1.0, "blue is 0.8x green in the fixture, so it must be scaled up: {b}");
    }

    /// `sampled` reads a strided subset. It is allowed to move the coefficients, but
    /// only slightly, and it must stay on the same side of 1.0 as the exact answer.
    #[test]
    fn sampled_config_stays_close_to_exact() {
        let frame = cast_gradient_frame(256, 192);
        let exact = compute_white_balance_grid(&frame, 16, 25.0).unwrap();

        // A 16x12 block at grid 16; a budget of 48 forces a stride of 2.
        let sampled = compute_white_balance_grid_with_config(
            &frame,
            16,
            25.0,
            WhiteBalanceConfig::sampled(48),
        )
        .unwrap();

        for c in 0..3 {
            assert!(
                (exact[c] - sampled[c]).abs() < 0.05,
                "channel {c}: sampled {} strayed from exact {}",
                sampled[c],
                exact[c]
            );
        }
        assert!(sampled[0] < 1.0 && sampled[2] > 1.0, "sampled lost the cast direction: {sampled:?}");
    }

    /// A budget at or above the block area must take the contiguous path and produce
    /// exactly the exact answer, so `sampled` degrades gracefully rather than switching
    /// algorithms at an arbitrary threshold.
    #[test]
    fn a_budget_larger_than_the_block_is_the_exact_path() {
        let frame = cast_gradient_frame(256, 192);
        let exact = compute_white_balance_grid(&frame, 16, 25.0).unwrap();
        let generous =
            compute_white_balance_grid_with_config(&frame, 16, 25.0, WhiteBalanceConfig::sampled(1 << 20))
                .unwrap();
        assert_eq!(exact, generous);
        assert_eq!(WhiteBalanceConfig::sampled(1 << 20).stride_for(16, 12), 1);
    }

    #[test]
    fn grid_white_balance_rejects_non_rgb() {
        let frame = Frame::filled(32, 32, 1, 0.1).unwrap();
        assert!(compute_white_balance_grid(&frame, 16, 25.0).is_err());
    }

    /// A grid finer than the frame has zero-sized blocks; that is a no-op, not a panic.
    #[test]
    fn a_grid_finer_than_the_frame_is_neutral() {
        let frame = cast_gradient_frame(8, 8);
        assert_eq!(compute_white_balance_grid(&frame, 16, 25.0).unwrap(), [1.0, 1.0, 1.0]);
    }

    /// An all-zero frame filters every block out and must fall back rather than divide
    /// by the empty percentile.
    #[test]
    fn an_empty_frame_falls_back_to_neutral() {
        let frame = Frame::zeros(64, 64, 3).unwrap();
        assert_eq!(compute_white_balance_grid(&frame, 8, 25.0).unwrap(), [1.0, 1.0, 1.0]);
    }
}
