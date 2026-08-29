//! Row and column fixed-pattern noise removal on the raw mosaic
//!
//! Sensor readout gives every row and every column a small offset of its own.
//! It is measurable on both fixtures — 5.9 ADU of excess per row and 6.7 per
//! column on the IMX533, 39 and 31 on the IMX464 — and unlike photon noise it
//! does **not** average down with frame count: after 35 subs the random noise is
//! at 14 ADU while the pattern still sits at 6, so it becomes roughly 40 % of
//! what is left. Drift on an undriven mount smears it, which is why it reads as
//! soft banding rather than as sharp lines.
//!
//! Nothing downstream removes it. Background extraction models a smooth
//! gradient, and a wavelet denoiser *smooths* a one-pixel-wide line into a soft
//! streak — worse than leaving it alone.
//!
//! # The correction
//!
//! On each colour site, subtract `row_median - site_median`, then
//! `column_median - site_median` recomputed on the row-corrected data. Two O(N)
//! passes per axis, medians throughout so a star field or a bright nebula
//! crossing a row cannot drag its offset.
//!
//! The reference level is the median *of the row medians* rather than a median
//! over the whole site: the offsets it produces sum to zero by construction, so
//! the correction cannot shift the frame's overall level and change what the
//! autostretch solves for.
//!
//! Sites are corrected independently — the four Bayer sites sit at different
//! levels, and a row median taken across them would measure the mosaic.

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

/// What [`remove_fpn`] took out, in normalized units.
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

/// Turn per-line medians into offsets against each site's own reference level.
///
/// `medians[i][p]` is the median of line `i` on the site whose *other* parity is
/// `p`; the site a line belongs to is `(p, i % step)`, so the reference is taken
/// over the lines of matching parity only.
fn axis_offsets(medians: &[[f32; MAX_STEP]], step: usize) -> Vec<[f32; MAX_STEP]> {
    let mut reference = [[0.0f32; MAX_STEP]; MAX_STEP];
    for (p, row) in reference.iter_mut().enumerate().take(step) {
        for (parity, slot) in row.iter_mut().enumerate().take(step) {
            let mut values: Vec<f32> = medians
                .iter()
                .skip(parity)
                .step_by(step)
                .map(|m| m[p])
                .collect();
            *slot = fast_median(&mut values);
        }
    }

    medians
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut out = [0.0f32; MAX_STEP];
            for (p, slot) in out.iter_mut().enumerate().take(step) {
                *slot = m[p] - reference[p][i % step];
            }
            out
        })
        .collect()
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

    #[test]
    fn removes_an_injected_row_pattern() {
        let mut frame = noisy_sky(128, 128, 0.20, 0.01);
        for y in 0..128 {
            let bias = if y % 3 == 0 { 0.02 } else { -0.01 };
            for x in 0..128 {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + bias);
            }
        }
        let before = line_median_spread(&frame, true);
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        let stats = remove_fpn(&mut cfa).unwrap();

        let after = line_median_spread(cfa.frame(), true);
        assert!(stats.row_rms > 0.005, "row_rms was {}", stats.row_rms);
        assert!(
            after < before / 10.0,
            "row spread {before} -> {after}, expected an order of magnitude"
        );
    }

    #[test]
    fn removes_an_injected_column_pattern() {
        let mut frame = noisy_sky(128, 128, 0.20, 0.01);
        for x in 0..128 {
            let bias = if x % 5 == 0 { 0.03 } else { -0.005 };
            for y in 0..128 {
                frame.set_pixel(x, y, 0, frame.get_pixel(x, y, 0) + bias);
            }
        }
        let before = line_median_spread(&frame, false);
        let mut cfa = CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap();

        let stats = remove_fpn(&mut cfa).unwrap();

        let after = line_median_spread(cfa.frame(), false);
        assert!(stats.column_rms > 0.005);
        assert!(
            after < before / 10.0,
            "column spread {before} -> {after}, expected an order of magnitude"
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
        let before = line_median_spread(&frame, true);
        let mut cfa = CfaFrame::direct(frame);

        remove_fpn(&mut cfa).unwrap();

        let after = line_median_spread(cfa.frame(), true);
        assert!(after < before / 10.0, "row spread {before} -> {after}");
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
