//! Row and column fixed-pattern noise removal on the raw mosaic: sensor readout gives
//! every row and column a small offset (5.9/6.7 ADU per row/column on IMX533, 39/31 on
//! IMX464) that does **not** average down with stacking — after 35 subs random noise
//! is 14 ADU, the pattern still 6, ~40% of what's left. Nothing downstream removes it:
//! background extraction models a smooth gradient, a wavelet denoiser only smears it.
//!
//! Correction: per colour site, high-pass each row's median and subtract, then the
//! same by column on the row-corrected data. **The high-pass is load-bearing** —
//! subtracting against a whole-site reference (the first version) removed real
//! low-frequency structure too, draining 5.2% of the Dumbbell's flux; high-passed, the
//! same measurement moves 0.008%. Sites are corrected independently — the four Bayer
//! sites sit at different levels.

use rayon::prelude::*;

use crate::error::{Result, StackError};
use crate::statistics::fast_median;

use super::{CfaFrame, CfaStage};

/// Columns one rayon task gathers at a time.
///
/// A median wants its column contiguous, but reading a column straight out of a
/// planar frame strides the whole width. Transposing a block at a time keeps the
/// gather sequential in the source and the scratch inside L2: 32 columns of a
/// 3008-row frame is 192 KB.
const COLUMN_BLOCK: usize = 32;

/// The largest CFA period this module handles — 2x2 Bayer, or 1x1 for mono.
const MAX_STEP: usize = 2;

/// Half-width, in lines of the same colour site, of the window each site's line levels
/// are high-passed against (sixteen sensor lines either side). Narrow on purpose:
/// widening does *not* remove more readout offset (line-to-line excess already reaches
/// the noise floor from width 8 to 192 on the IMX533 fixture) but does subtract more
/// real structure — at 48 it measurably *added* spread to one site, inventing the
/// defect it exists to remove. The floor is the other end: the window needs enough
/// lines for the mean to be a stable local estimate, which sixteen is and two isn't.
const OFFSET_SMOOTHING_RADIUS: usize = 8;

/// What [`remove_fpn`] took out, in normalized units.
///
/// Smaller than the figures the plan's fixture table quotes, and expected to be:
/// those are the *total* per-line excess, while this is only the line-to-line
/// part the correction now removes. On the IMX533 fixture it comes out at ~2.9
/// ADU RMS per row and ~2.4 per column.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FpnStats {
    /// RMS of the row offsets that were subtracted.
    pub row_rms: f32,
    /// RMS of the column offsets that were subtracted.
    pub column_rms: f32,
}

/// A registered [`CfaStage`] wrapper around [`remove_fpn`].
#[derive(Debug, Clone, Copy, Default)]
pub struct FpnFilter;

impl CfaStage for FpnFilter {
    fn name(&self) -> &'static str {
        "row_column_fpn"
    }

    fn apply(&self, frame: &mut CfaFrame) -> Result<()> {
        let stats = remove_fpn(frame)?;
        tracing::debug!(
            row_rms = stats.row_rms,
            column_rms = stats.column_rms,
            "Row/column FPN removed"
        );
        Ok(())
    }
}

/// Flatten per-row and per-column readout offsets, one colour site at a time.
pub fn remove_fpn(cfa: &mut CfaFrame) -> Result<FpnStats> {
    let Some(planes) = cfa.planes() else {
        return Err(StackError::ChannelMismatch {
            expected: 1,
            actual: cfa.frame().channels(),
        });
    };

    let (width, height, step) = (planes.width, planes.height, planes.step);
    if width < step || height < step {
        return Ok(FpnStats::default());
    }

    let row_offsets = axis_offsets(&row_medians(cfa.frame().data(), width, height, step), step);
    let row_rms = rms(&row_offsets, step);
    apply_row_offsets(cfa.frame_mut(), &row_offsets, step);

    let column_offsets = axis_offsets(
        &column_medians(cfa.frame().data(), width, height, step),
        step,
    );
    let column_rms = rms(&column_offsets, step);
    apply_column_offsets(cfa.frame_mut(), &column_offsets, step);

    Ok(FpnStats {
        row_rms,
        column_rms,
    })
}

/// Median of each row, per x-parity. Index `[y][x0]`.
fn row_medians(data: &[f32], width: usize, height: usize, step: usize) -> Vec<[f32; MAX_STEP]> {
    (0..height)
        .into_par_iter()
        .map_init(Vec::new, |scratch: &mut Vec<f32>, y| {
            let row = &data[y * width..][..width];
            let mut out = [0.0f32; MAX_STEP];
            for (x0, slot) in out.iter_mut().enumerate().take(step) {
                scratch.clear();
                scratch.extend(row[x0..].iter().step_by(step).copied());
                *slot = fast_median(scratch);
            }
            out
        })
        .collect()
}

/// Median of each column, per y-parity. Index `[x][y0]`.
fn column_medians(data: &[f32], width: usize, height: usize, step: usize) -> Vec<[f32; MAX_STEP]> {
    let per_parity: Vec<Vec<f32>> = (0..step)
        .map(|y0| column_medians_for_parity(data, width, height, step, y0))
        .collect();

    (0..width)
        .map(|x| {
            let mut out = [0.0f32; MAX_STEP];
            for (slot, parity) in out.iter_mut().zip(per_parity.iter()) {
                *slot = parity[x];
            }
            out
        })
        .collect()
}

/// Median of each column over the rows of one y-parity, indexed by `x`.
fn column_medians_for_parity(
    data: &[f32],
    width: usize,
    height: usize,
    step: usize,
    y0: usize,
) -> Vec<f32> {
    let rows: Vec<usize> = (y0..height).step_by(step).collect();
    let blocks: Vec<usize> = (0..width).step_by(COLUMN_BLOCK).collect();

    blocks
        .into_par_iter()
        .flat_map_iter(|xa| {
            let xb = (xa + COLUMN_BLOCK).min(width);
            let mut scratch = vec![0.0f32; (xb - xa) * rows.len()];
            for (j, &y) in rows.iter().enumerate() {
                let row = &data[y * width..][..width];
                for (k, &v) in row[xa..xb].iter().enumerate() {
                    scratch[k * rows.len() + j] = v;
                }
            }
            scratch
                .chunks_mut(rows.len())
                .map(fast_median)
                .collect::<Vec<_>>()
                .into_iter()
        })
        .collect()
}

/// Turn per-line medians into the *line-to-line* part of each site's offset.
/// `medians[i][p]` is the median of line `i` on the site whose *other* parity is `p`,
/// so each site's own sequence is every `step`-th entry; it's high-passed against a
/// centred moving average of radius [`OFFSET_SMOOTHING_RADIUS`], and the residual is
/// what gets subtracted. Being a residual around a local mean, it's centred by
/// construction — same guarantee the whole-site reference this replaced gave — so the
/// correction still can't shift the frame's overall level or change what autostretch
/// solves for.
fn axis_offsets(medians: &[[f32; MAX_STEP]], step: usize) -> Vec<[f32; MAX_STEP]> {
    let mut offsets = vec![[0.0f32; MAX_STEP]; medians.len()];
    let mut sequence: Vec<f32> = Vec::new();

    for p in 0..step {
        for parity in 0..step {
            let lines: Vec<usize> = (parity..medians.len()).step_by(step).collect();
            sequence.clear();
            sequence.extend(lines.iter().map(|&i| medians[i][p]));

            for (k, &line) in lines.iter().enumerate() {
                offsets[line][p] = line_excess(&sequence, k, OFFSET_SMOOTHING_RADIUS);
            }
        }
    }

    offsets
}

/// How far line `k` sits above its own neighbourhood: `x[k]` minus a centred moving
/// average of even order `2 * radius`, shrinking symmetrically at the borders.
///
/// The even order is the point, not an implementation detail — it annihilates a
/// period-2 component exactly, and odd/even line readout (the classic CMOS defect) is
/// period 2. A median filter is worse than useless: an odd-width median has a strict
/// alternation as a *root*, reproducing the pattern with zero residual; an odd-width
/// mean leaves about half of it behind. Summed as differences from `x[k]`, not as
/// `x[k] - mean(window)`, so an already-level line yields exactly zero rather than an
/// ULP of it — keeping the correction a genuine no-op where there's no pattern.
fn line_excess(sequence: &[f32], k: usize, radius: usize) -> f32 {
    let radius = radius.min(k).min(sequence.len() - 1 - k);
    if radius == 0 {
        return 0.0;
    }

    let centre = sequence[k];
    let interior: f32 = sequence[k - radius + 1..=k + radius - 1]
        .iter()
        .map(|v| centre - v)
        .sum();
    let ends = 0.5 * ((centre - sequence[k - radius]) + (centre - sequence[k + radius]));
    (interior + ends) / (2 * radius) as f32
}

fn rms(offsets: &[[f32; MAX_STEP]], step: usize) -> f32 {
    let count = offsets.len() * step;
    if count == 0 {
        return 0.0;
    }
    let sum: f32 = offsets
        .iter()
        .flat_map(|o| o.iter().take(step))
        .map(|v| v * v)
        .sum();
    (sum / count as f32).sqrt()
}

fn apply_row_offsets(frame: &mut crate::frame::Frame, offsets: &[[f32; MAX_STEP]], step: usize) {
    let width = frame.width();
    frame
        .data_mut()
        .par_chunks_mut(width)
        .zip(offsets.par_iter())
        .for_each(|(row, offset)| subtract_periodic(row, &offset[..step]));
}

fn apply_column_offsets(frame: &mut crate::frame::Frame, offsets: &[[f32; MAX_STEP]], step: usize) {
    let width = frame.width();
    // One dense row of offsets per y-parity: the apply then reads straight
    // through both slices instead of indexing a strided table per sample.
    let dense: Vec<Vec<f32>> = (0..step)
        .map(|y0| offsets.iter().map(|o| o[y0]).collect())
        .collect();

    frame
        .data_mut()
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (v, offset) in row.iter_mut().zip(dense[y % step].iter()) {
                *v = (*v - offset).clamp(0.0, 1.0);
            }
        });
}

/// Subtract a repeating offset pattern of period `offsets.len()` from a row.
fn subtract_periodic(row: &mut [f32], offsets: &[f32]) {
    if offsets.len() == 1 {
        let offset = offsets[0];
        for v in row.iter_mut() {
            *v = (*v - offset).clamp(0.0, 1.0);
        }
        return;
    }

    let (even, odd) = (offsets[0], offsets[1]);
    let (pairs, remainder) = row.as_chunks_mut::<2>();
    for pair in pairs {
        pair[0] = (pair[0] - even).clamp(0.0, 1.0);
        pair[1] = (pair[1] - odd).clamp(0.0, 1.0);
    }
    if let [last] = remainder {
        *last = (*last - even).clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debayer::CfaPattern;
    use crate::frame::Frame;

    fn noisy_sky(width: usize, height: usize, level: f32, noise: f32) -> Frame {
        let mut frame = Frame::zeros(width, height, 1).unwrap();
        let mut seed = 0x1234_5678u32;
        for y in 0..height {
            for x in 0..width {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let unit = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
                frame.set_pixel(x, y, 0, level + unit * noise);
            }
        }
        frame
    }

    /// Spread of the *differences between adjacent lines of one colour site* —
    /// fixed-pattern noise proper, what this filter exists to remove. Two things are
    /// load-bearing: it's per **site** (a whole-row median mixes the two x-parities,
    /// whose differing offsets would read as residual no per-site correction could
    /// remove), and it's the line-to-line half, not the total (the correction is a
    /// high-pass, so asserting on the total means asserting real gradients get
    /// flattened too — the behaviour that drained 5% of the Dumbbell's flux).
    ///
    /// Doesn't go to zero: the column pass runs on the row-corrected frame and
    /// perturbs the row medians in turn, so roughly seven-fold is what both passes
    /// together actually reach.
    fn line_to_line_spread(
        frame: &Frame,
        horizontal: bool,
        origin: (usize, usize),
        step: usize,
    ) -> f32 {
        let (width, height) = (frame.width(), frame.height());
        let (x0, y0) = origin;
        let outer: Vec<usize> = if horizontal {
            (y0..height).step_by(step).collect()
        } else {
            (x0..width).step_by(step).collect()
        };
        let inner: Vec<usize> = if horizontal {
            (x0..width).step_by(step).collect()
        } else {
            (y0..height).step_by(step).collect()
        };

        let levels: Vec<f32> = outer
            .iter()
            .map(|&i| {
                let mut line: Vec<f32> = inner
                    .iter()
                    .map(|&j| {
                        let (x, y) = if horizontal { (j, i) } else { (i, j) };
                        frame.get_pixel(x, y, 0)
                    })
                    .collect();
                fast_median(&mut line)
            })
            .collect();

        let deltas: Vec<f32> = levels.windows(2).map(|w| w[1] - w[0]).collect();
        let mean = deltas.iter().sum::<f32>() / deltas.len() as f32;
        let variance =
            deltas.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / deltas.len() as f32;
        variance.sqrt() / std::f32::consts::SQRT_2
    }

    /// Excess spread of the per-line medians over what the sample noise alone
    /// predicts — the same quantity the fixture measurements in the plan report.
    fn line_median_spread(frame: &Frame, horizontal: bool) -> f32 {
        let (width, height) = (frame.width(), frame.height());
        let outer = if horizontal { height } else { width };
        let inner = if horizontal { width } else { height };
        let mut medians = Vec::with_capacity(outer);
        for i in 0..outer {
            let mut line: Vec<f32> = (0..inner)
                .map(|j| {
                    if horizontal {
                        frame.get_pixel(j, i, 0)
                    } else {
                        frame.get_pixel(i, j, 0)
                    }
                })
                .collect();
            medians.push(fast_median(&mut line));
        }
        let mean = medians.iter().sum::<f32>() / medians.len() as f32;
        (medians.iter().map(|m| (m - mean).powi(2)).sum::<f32>() / medians.len() as f32).sqrt()
    }

    /// A per-line offset drawn independently for each line — which is what
    /// readout FPN is. A deterministic low-period pattern would be a bad model
    /// *and* a bad test: the correction is a high-pass, so anything slow enough
    /// to pass for real structure is left alone on purpose.
    fn line_offsets(count: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * amplitude
            })
            .collect()
    }

    #[test]
    fn removes_an_injected_row_pattern() {
        let mut frame = noisy_sky(128, 128, 0.20, 0.01);
        let biases = line_offsets(128, 0.04, 0xBEEF_0001);
        for y in 0..128 {
            let bias = biases[y];
            for x in 0..128 {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + bias);
            }
        }
        let before = line_to_line_spread(&frame, true, (0, 0), 2);
        let before_total = line_median_spread(&frame, true);
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        let stats = remove_fpn(&mut cfa).unwrap();

        let after = line_to_line_spread(cfa.frame(), true, (0, 0), 2);
        assert!(stats.row_rms > 0.005, "row_rms was {}", stats.row_rms);
        assert!(
            after < before / 5.0,
            "row line-to-line spread {before} -> {after}, expected at least 5x"
        );
        assert!(
            line_median_spread(cfa.frame(), true) <= before_total,
            "the correction added total spread rather than removing it"
        );
    }

    #[test]
    fn removes_an_injected_column_pattern() {
        let mut frame = noisy_sky(128, 128, 0.20, 0.01);
        let biases = line_offsets(128, 0.04, 0xBEEF_0002);
        for x in 0..128 {
            let bias = biases[x];
            for y in 0..128 {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + bias);
            }
        }
        let before = line_to_line_spread(&frame, false, (0, 0), 2);
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        let stats = remove_fpn(&mut cfa).unwrap();

        let after = line_to_line_spread(cfa.frame(), false, (0, 0), 2);
        assert!(stats.column_rms > 0.005);
        assert!(
            after < before / 5.0,
            "column line-to-line spread {before} -> {after}, expected at least 5x"
        );
    }

    #[test]
    fn leaves_the_mosaic_itself_alone() {
        // Four sites at four distinct levels and no line pattern: the offsets
        // must all come out at zero, not "flatten" the CFA into grey.
        let mut frame = Frame::zeros(64, 64, 1).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                frame.set_pixel(x, y, 0, 0.1 + 0.1 * (y % 2) as f32 + 0.2 * (x % 2) as f32);
            }
        }
        let original = frame.clone();
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        let stats = remove_fpn(&mut cfa).unwrap();

        assert_eq!(stats, FpnStats::default());
        assert_eq!(cfa.frame().data(), original.data());
    }

    #[test]
    fn preserves_the_overall_level() {
        let frame = noisy_sky(96, 96, 0.30, 0.02);
        let before: f32 = frame.data().iter().sum::<f32>() / frame.sample_count() as f32;
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        remove_fpn(&mut cfa).unwrap();

        let after: f32 = cfa.frame().data().iter().sum::<f32>() / cfa.frame().sample_count() as f32;
        assert!(
            (after - before).abs() < 1e-3,
            "mean moved {before} -> {after}"
        );
    }

    #[test]
    fn a_mono_frame_is_corrected_as_a_single_site() {
        let mut frame = noisy_sky(96, 96, 0.20, 0.01);
        for y in (0..96).step_by(2) {
            for x in 0..96 {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + 0.02);
            }
        }
        let before = line_to_line_spread(&frame, true, (0, 0), 1);
        let mut cfa = CfaFrame::direct(frame);

        remove_fpn(&mut cfa).unwrap();

        let after = line_to_line_spread(cfa.frame(), true, (0, 0), 1);
        assert!(
            after < before / 5.0,
            "mono row line-to-line spread {before} -> {after}"
        );
    }

    #[test]
    fn an_rgb_frame_is_rejected() {
        let mut cfa = CfaFrame::direct(Frame::filled(8, 8, 3, 0.5).unwrap());
        assert!(remove_fpn(&mut cfa).is_err());
    }

    #[test]
    fn output_stays_inside_the_normalized_range() {
        let mut frame = noisy_sky(64, 64, 0.002, 0.004);
        for y in 0..64 {
            for x in 0..64 {
                if y % 2 == 0 {
                    frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + 0.01);
                }
            }
        }
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        remove_fpn(&mut cfa).unwrap();

        assert!(cfa.frame().data().iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}
