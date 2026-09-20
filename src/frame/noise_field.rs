//! A coarse, per-channel map of how noisy each part of an image is.
//!
//! Produced by `stacking::MasterStack` from the accumulator it already keeps, resampled
//! onto an output grid by `server::encoding`, and consumed by the denoise plugin. It
//! lives here rather than under any of the three because none of them owns it: it is an
//! image-shaped data type, and `frame` is where those live.
//!
//! **Variance, never sigma.** Every operation on this field — block reduction, resample,
//! folding channels into luminance — combines independent contributions in quadrature,
//! and a field of sigmas cannot be averaged or interpolated without squaring it first.
//! Resampling it like an image overstates the output noise by roughly `sqrt(k)` for a
//! `k`-fold reduction, and every threshold built on it then comes out that much too
//! aggressive. See [`NoiseField::resampled`].

use crate::error::{Result, StackError};

/// Source pixels per field cell, each axis.
///
/// The field describes a slowly varying quantity and every consumer of it works in
/// windows of 8 output pixels or more, so this trades resolution it does not need for
/// memory it would otherwise have to move per frame: at 3008x3008x3 the full-resolution
/// map would be 108 MB, and this is 1.7 MB. Coverage falls off over the session's drift
/// — tens of pixels — so the stack border, the feature the field mainly exists to
/// describe, is still resolved with room to spare.
pub const NOISE_REDUCTION: usize = 8;

/// Per-channel variance over a coarse grid, with the image geometry it describes.
///
/// Cells are plane-major like [`crate::frame::Frame`] (`idx = channel*w*h + y*w + x`).
/// A cell is [`f32::NAN`] where there was nothing to measure — a stack too shallow to
/// have a spread yet — rather than `0.0`, which would propagate silently through a
/// divide and read as "perfectly clean".
#[derive(Debug, Clone)]
pub struct NoiseField {
    variance: Vec<f32>,
    width: usize,
    height: usize,
    channels: usize,
    source_width: usize,
    source_height: usize,
}

impl NoiseField {
    /// `variance` is `width * height * channels` cells, plane-major, describing an image
    /// of `source_width x source_height`.
    pub fn new(
        variance: Vec<f32>,
        width: usize,
        height: usize,
        channels: usize,
        source_width: usize,
        source_height: usize,
    ) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }
        if variance.len() != width * height * channels {
            return Err(StackError::InvalidConfiguration(format!(
                "noise field has {} cells, expected {}x{}x{}",
                variance.len(),
                width,
                height,
                channels
            )));
        }
        Ok(Self {
            variance,
            width,
            height,
            channels,
            source_width,
            source_height,
        })
    }

    pub fn variance(&self) -> &[f32] {
        &self.variance
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Width of the image this field describes, in pixels.
    pub fn source_width(&self) -> usize {
        self.source_width
    }

    /// Height of the image this field describes, in pixels.
    pub fn source_height(&self) -> usize {
        self.source_height
    }

    /// Whether any cell carries a measurement. A field with none is the warm-up case and
    /// callers should pass `None` downstream rather than a field of NaN.
    pub fn is_usable(&self) -> bool {
        self.variance.iter().any(|v| v.is_finite() && *v > 0.0)
    }

    /// The field's own robust centre: the median of its finite cells, per channel,
    /// averaged. Zero when there is nothing to measure.
    ///
    /// Consumers that work in *relative* noise divide by this, which is what makes every
    /// spatially uniform gain between the accumulator and the denoiser — background
    /// neutralisation, preview binning, and the unmeasured correlation a bilinear warp
    /// leaves between neighbouring stack pixels — cancel instead of needing a model.
    pub fn robust_centre(&self) -> f32 {
        let mut finite: Vec<f32> = self
            .variance
            .iter()
            .copied()
            .filter(|v| v.is_finite() && *v > 0.0)
            .collect();
        if finite.is_empty() {
            return 0.0;
        }
        crate::statistics::select_median(&mut finite)
    }

    /// Bilinearly interpolated variance of `channel` at a pixel of the source image.
    ///
    /// Non-finite neighbours are dropped from the interpolation rather than allowed to
    /// poison it, so a warm-up hole shrinks from its edges instead of spreading. Returns
    /// [`f32::NAN`] only where every neighbour is unmeasured.
    pub fn sample(&self, channel: usize, x: usize, y: usize) -> f32 {
        let channel = channel.min(self.channels - 1);
        // Cell centres sit at (i + 0.5) * source/field along each axis, so a pixel's
        // position in cell coordinates is this, less the half-cell offset.
        let fx = self.cell_coord(x, self.source_width, self.width);
        let fy = self.cell_coord(y, self.source_height, self.height);
        let (x0, tx) = split(fx, self.width);
        let (y0, ty) = split(fy, self.height);
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);

        let plane = &self.variance[channel * self.width * self.height..][..self.width * self.height];
        let mut sum = 0.0;
        let mut weight = 0.0;
        for (cy, wy) in [(y0, 1.0 - ty), (y1, ty)] {
            for (cx, wx) in [(x0, 1.0 - tx), (x1, tx)] {
                let v = plane[cy * self.width + cx];
                if v.is_finite() && wy * wx > 0.0 {
                    sum += wy * wx * v;
                    weight += wy * wx;
                }
            }
        }
        if weight > 0.0 {
            sum / weight
        } else {
            f32::NAN
        }
    }

    fn cell_coord(&self, pixel: usize, source_len: usize, field_len: usize) -> f32 {
        if source_len == 0 || field_len == 0 {
            return 0.0;
        }
        let scale = field_len as f32 / source_len as f32;
        (pixel as f32 + 0.5) * scale - 0.5
    }

    /// This field with a per-channel multiplicative gain applied to the image it
    /// describes. Variance scales by the square of the gain.
    ///
    /// Background neutralisation is the one pipeline stage that reaches the denoiser as
    /// a multiply; everything else between the accumulator and the filters is additive,
    /// luminance-preserving or applied after them. See `PreviewRender::linear_gain`.
    pub fn scaled(&self, gain: &[f32]) -> Self {
        let plane = self.width * self.height;
        let mut variance = self.variance.clone();
        for (channel, cells) in variance.chunks_exact_mut(plane).enumerate() {
            let g = gain.get(channel).copied().unwrap_or(1.0);
            if g == 1.0 {
                continue;
            }
            let g2 = g * g;
            for cell in cells.iter_mut() {
                *cell *= g2;
            }
        }
        Self {
            variance,
            ..self.clone()
        }
    }

    /// This field, after the image it describes was binned by `factor`.
    ///
    /// A box mean of `factor^2` independent samples, which is a resample whose taps are
    /// `factor` equal weights of `1/factor` per axis — so it goes through the same
    /// quadrature path as any other, with `sum(w^2) = 1/factor`.
    pub fn binned(&self, factor: usize) -> Result<Self> {
        if factor <= 1 {
            return Ok(self.clone());
        }
        let scale = [1.0 / factor as f32];
        self.resampled(
            (self.source_width / factor).max(1),
            (self.source_height / factor).max(1),
            &scale,
            &scale,
        )
    }

    /// This field, resampled onto an output image of `target_width x target_height`.
    ///
    /// `column_scale` and `row_scale` are the per-output-index factors by which the
    /// resample kernel scales the variance of independent source samples — `sum(w^2)`
    /// for that index's taps, which `server::encoding::AxisTaps` builds beside the
    /// weights so the two cannot drift apart. An output pixel is `sum(w_i * x_i)` with
    /// `sum(w_i) = 1`, so its variance is `sum(w_i^2 * sigma_i^2)`; because this field is
    /// deliberately coarse, `sigma^2` is constant across a tap footprint of two or three
    /// source pixels and the separable double sum collapses to the product used here.
    ///
    /// Pass a single `1.0` for an axis that is not resampled.
    ///
    /// **This is the step that is easiest to get wrong and hardest to see.** Resampling
    /// the field like an image — averaging its sigmas — overstates output noise by
    /// roughly `sqrt(k)`, and nothing downstream reports a number that would show it;
    /// `encoding::tests` guards both directions.
    pub fn resampled(
        &self,
        target_width: usize,
        target_height: usize,
        column_scale: &[f32],
        row_scale: &[f32],
    ) -> Result<Self> {
        let width = target_width.div_ceil(NOISE_REDUCTION).max(1);
        let height = target_height.div_ceil(NOISE_REDUCTION).max(1);
        let mut variance = vec![f32::NAN; width * height * self.channels];

        for channel in 0..self.channels {
            let plane = &mut variance[channel * width * height..][..width * height];
            for cy in 0..height {
                // The output pixels this cell covers, and the source pixel its centre
                // corresponds to.
                let (oy0, oy1) = cell_span(cy, height, target_height);
                let src_y = self.source_pixel(oy0, oy1, target_height, self.source_height);
                let ry = axis_mean(row_scale, oy0, oy1);
                for cx in 0..width {
                    let (ox0, ox1) = cell_span(cx, width, target_width);
                    let src_x = self.source_pixel(ox0, ox1, target_width, self.source_width);
                    let rx = axis_mean(column_scale, ox0, ox1);
                    let v = self.sample(channel, src_x, src_y);
                    plane[cy * width + cx] = if v.is_finite() { v * rx * ry } else { f32::NAN };
                }
            }
        }

        Self::new(
            variance,
            width,
            height,
            self.channels,
            target_width,
            target_height,
        )
    }

    /// The source pixel an output span maps back to: its centre, clamped into range.
    fn source_pixel(&self, out0: usize, out1: usize, target_len: usize, source_len: usize) -> usize {
        if target_len == 0 || source_len == 0 {
            return 0;
        }
        let centre = (out0 + out1) as f32 * 0.5;
        let src = centre * source_len as f32 / target_len as f32;
        (src as usize).min(source_len - 1)
    }
}

/// Output pixels covered by field cell `i`, as a half-open span.
fn cell_span(i: usize, field_len: usize, target_len: usize) -> (usize, usize) {
    let lo = i * target_len / field_len;
    let hi = ((i + 1) * target_len / field_len).max(lo + 1).min(target_len);
    (lo, hi)
}

/// Mean of `scale` over `[lo, hi)`, or its only entry when the axis is not resampled.
fn axis_mean(scale: &[f32], lo: usize, hi: usize) -> f32 {
    if scale.len() == 1 {
        return scale[0];
    }
    let hi = hi.min(scale.len());
    if lo >= hi {
        return *scale.last().unwrap_or(&1.0);
    }
    let slice = &scale[lo..hi];
    slice.iter().sum::<f32>() / slice.len() as f32
}

/// Integer cell index and fraction for an interpolation coordinate.
fn split(coord: f32, len: usize) -> (usize, f32) {
    if coord <= 0.0 {
        return (0, 0.0);
    }
    let max = (len - 1) as f32;
    if coord >= max {
        return (len - 1, 0.0);
    }
    let base = coord.floor();
    (base as usize, coord - base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: f32, w: usize, h: usize, channels: usize, sw: usize, sh: usize) -> NoiseField {
        NoiseField::new(vec![value; w * h * channels], w, h, channels, sw, sh).unwrap()
    }

    #[test]
    fn an_unmeasured_field_reports_itself() {
        let field = flat(f32::NAN, 4, 4, 1, 32, 32);
        assert!(!field.is_usable());
        assert_eq!(field.robust_centre(), 0.0);
        assert!(field.sample(0, 5, 5).is_nan());
    }

    /// The whole point of the type: an output pixel built from `k` source samples has
    /// `1/k` of their variance, so the resample must scale by `sum(w^2)`, not by 1.
    #[test]
    fn resampling_scales_variance_by_the_tap_energy() {
        let field = flat(4.0, 4, 4, 1, 32, 32);
        // Four equal taps: sum(w) = 1, sum(w^2) = 4 * 0.25^2 = 0.25.
        let scale = vec![0.25f32; 8];
        let out = field.resampled(8, 8, &scale, &scale).unwrap();
        for &v in out.variance() {
            assert!(
                (v - 4.0 * 0.25 * 0.25).abs() < 1e-6,
                "variance {v} should be scaled by both axes' tap energy"
            );
        }
    }

    #[test]
    fn an_unresampled_axis_takes_a_single_unit_scale() {
        let field = flat(9.0, 2, 2, 1, 16, 16);
        let out = field.resampled(16, 16, &[1.0], &[1.0]).unwrap();
        assert!(out.variance().iter().all(|v| (v - 9.0).abs() < 1e-6));
        assert_eq!((out.source_width(), out.source_height()), (16, 16));
    }

    /// A warm-up hole stays the size of the cells that are actually unmeasured.
    ///
    /// The cell's own centre honestly reads NaN — there was nothing to measure there,
    /// and the consumer has a global estimate to fall back to. What must not happen is
    /// the hole *spreading*: as soon as a finite neighbour carries any interpolation
    /// weight at all it takes the whole answer, so the NaN region shrinks from its edges
    /// instead of bleeding a cell outward in every direction.
    #[test]
    fn a_nan_cell_does_not_spread_past_its_own_footprint() {
        let mut cells = vec![4.0f32; 16];
        cells[0] = f32::NAN;
        let field = NoiseField::new(cells, 4, 4, 1, 32, 32).unwrap();

        assert!(
            field.sample(0, 0, 0).is_nan(),
            "the unmeasured cell's own centre must stay unmeasured, not invent a value"
        );

        // Cell 0 spans source pixels 0..8 with its centre at 3.5, so anything from the
        // next cell's centre on reads purely measured cells.
        for y in 12..32 {
            for x in 12..32 {
                let v = field.sample(0, x, y);
                assert!(v.is_finite(), "({x},{y}) read {v}: the hole spread");
                assert!((v - 4.0).abs() < 1e-6);
            }
        }

        // And a pixel straddling the boundary takes its value from the finite side
        // rather than averaging a NaN in.
        let straddle = field.sample(0, 6, 3);
        assert!(
            straddle.is_finite() && (straddle - 4.0).abs() < 1e-6,
            "a straddling sample read {straddle} instead of its finite neighbour"
        );
    }

    #[test]
    fn the_robust_centre_ignores_unmeasured_cells() {
        let mut cells = vec![f32::NAN; 16];
        cells[3] = 2.0;
        cells[7] = 4.0;
        cells[11] = 6.0;
        let field = NoiseField::new(cells, 4, 4, 1, 32, 32).unwrap();
        assert_eq!(field.robust_centre(), 4.0);
    }

    /// The field describes the source grid, so a value placed at one corner has to come
    /// back from that corner and not the opposite one.
    #[test]
    fn sampling_keeps_the_fields_orientation() {
        let mut cells = vec![1.0f32; 16];
        cells[0] = 100.0; // top-left cell
        let field = NoiseField::new(cells, 4, 4, 1, 40, 40).unwrap();
        assert!(field.sample(0, 0, 0) > 50.0, "top-left lost its value");
        assert!(field.sample(0, 39, 39) < 2.0, "bottom-right picked up the corner");
    }

    #[test]
    fn a_mismatched_cell_count_is_refused() {
        assert!(NoiseField::new(vec![0.0; 5], 2, 2, 1, 16, 16).is_err());
        assert!(NoiseField::new(vec![], 0, 2, 1, 16, 16).is_err());
    }
}
