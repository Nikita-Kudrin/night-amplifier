//! Planetary stacker engine for collecting, scoring, aligning, and stacking frames.

use rayon::prelude::*;
use tracing::{field, instrument, Span};

use crate::error::{Result, StackError};
use crate::frame::Frame;

use super::alignment::compute_alignment;
use super::config::{
    AlignmentRoi, PlanetaryConfig, PlanetaryStackMethod, PlanetaryStackStats,
};
use super::quality::compute_quality;
use std::sync::OnceLock;

/// PlanetaryStacker trait for the Pro plugin
pub trait PlanetaryStackerPlugin: Send + Sync {
    /// Aligns and warps the frame against the reference using multi-point surface alignment
    fn warp_frame(
        &self,
        frame: &Frame,
        reference: &Frame,
        roi: &AlignmentRoi,
        search_radius: usize,
    ) -> Result<Frame>;
    /// Clears any cached alignment points when the reference frame changes
    fn clear_cache(&self);
}

/// Global registry for the planetary stacking plugin
pub static PLANETARY_PLUGIN: OnceLock<Box<dyn PlanetaryStackerPlugin>> = OnceLock::new();

/// A scored frame ready for stacking
#[derive(Debug)]
pub struct ScoredFrame {
    /// The frame data
    pub frame: Frame,
    /// Quality score (higher = better)
    pub quality: f32,
    /// Alignment offset from reference (dx, dy)
    pub offset: (f32, f32),
}

/// Planetary stacker engine.
///
/// Collects frames, scores them by quality, aligns to a reference,
/// and combines the best frames into a final stack.
pub struct PlanetaryStacker {
    pub config: PlanetaryConfig,
    pub reference: Option<Frame>,
    pub frames: Vec<ScoredFrame>,
    pub width: usize,
    pub height: usize,
    pub channels: usize,
}

impl PlanetaryStacker {
    /// Creates a new planetary stacker with the given configuration
    pub fn new(config: PlanetaryConfig) -> Self {
        Self {
            config,
            reference: None,
            frames: Vec::new(),
            width: 0,
            height: 0,
            channels: 0,
        }
    }

    /// Creates a stacker with default configuration
    pub fn with_defaults() -> Self {
        Self::new(PlanetaryConfig::default())
    }

    /// Sets the reference frame for alignment.
    ///
    /// If not set, the first frame will be used as reference.
    pub fn set_reference(&mut self, frame: Frame) {
        self.width = frame.width();
        self.height = frame.height();
        self.channels = frame.channels();
        self.reference = Some(frame);
    }

    pub fn add_frame(&mut self, frame: &Frame) -> Result<f32> {
        self.add_frame_builtin(frame)
    }

    /// Built-in add_frame implementation.
    #[instrument(skip(self, frame), fields(
        width = frame.width(),
        height = frame.height(),
        frame_count = field::Empty,
        quality = field::Empty,
    ))]
    pub fn add_frame_builtin(&mut self, frame: &Frame) -> Result<f32> {
        if self.reference.is_none() {
            return self.add_first_frame(frame);
        }
        self.validate_dimensions(frame)?;

        let quality = compute_quality(frame, self.config.quality_metric);
        let offset = self.compute_frame_alignment(frame);

        if self.config.max_frames > 0 && self.frames.len() >= self.config.max_frames {
            if let Some(worst_idx) = self
                .frames
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.quality.partial_cmp(&b.quality).unwrap())
                .map(|(i, _)| i)
            {
                if quality > self.frames[worst_idx].quality {
                    self.frames.swap_remove(worst_idx);
                } else {
                    let span = Span::current();
                    span.record("frame_count", self.frames.len());
                    span.record("quality", quality);
                    return Ok(quality);
                }
            }
        }

        self.frames.push(ScoredFrame {
            frame: frame.clone(),
            quality,
            offset,
        });

        let span = Span::current();
        span.record("frame_count", self.frames.len());
        span.record("quality", quality);
        Ok(quality)
    }

    fn add_first_frame(&mut self, frame: &Frame) -> Result<f32> {
        self.width = frame.width();
        self.height = frame.height();
        self.channels = frame.channels();
        self.reference = Some(frame.clone());

        let quality = compute_quality(frame, self.config.quality_metric);
        self.frames.push(ScoredFrame {
            frame: frame.clone(),
            quality,
            offset: (0.0, 0.0),
        });

        Ok(quality)
    }

    fn validate_dimensions(&self, frame: &Frame) -> Result<()> {
        if frame.width() != self.width || frame.height() != self.height {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: self.width,
                cal_height: self.height,
            });
        }
        Ok(())
    }

    fn compute_frame_alignment(&self, frame: &Frame) -> (f32, f32) {
        let reference = self.reference.as_ref().unwrap();

        let roi = if self.config.auto_tracking {
            let lum = super::quality::frame_to_luminance(frame);
            let (cx, cy) = super::alignment::compute_centroid(&lum, self.width, self.height);
            let (base_w, base_h) = match self.config.alignment_roi {
                Some(r) => (r.width, r.height),
                None => {
                    let size = (self.width.min(self.height) / 2).max(64);
                    (size, size)
                }
            };
            AlignmentRoi::centered_at(cx, cy, base_w, base_h, self.width, self.height)
        } else {
            self.config.alignment_roi.unwrap_or_else(|| {
                let size = (self.width.min(self.height) / 2).max(64);
                AlignmentRoi::centered(self.width, self.height, size)
            })
        };

        let (dx, dy, ncc) = compute_alignment(
            reference,
            frame,
            &roi,
            self.config.search_radius,
            self.config.subpixel_factor,
        );

        tracing::debug!(dx, dy, ncc, "Planetary stacker alignment results");
        (dx, dy)
    }

    /// Returns the number of frames collected
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Returns the quality scores of all frames
    pub fn quality_scores(&self) -> Vec<f32> {
        self.frames.iter().map(|f| f.quality).collect()
    }

    pub fn stack(&self) -> Result<Frame> {
        self.stack_builtin()
    }

    /// Built-in stack implementation.
    pub fn stack_builtin(&self) -> Result<Frame> {
        if self.frames.is_empty() {
            return Err(StackError::InvalidConfiguration(
                "No frames to stack".to_string(),
            ));
        }

        let indices = self.select_best_frames();

        match self.config.stacking_method {
            PlanetaryStackMethod::Mean => self.stack_mean(&indices),
            PlanetaryStackMethod::Median => self.stack_percentile(&indices, 0.5),
            PlanetaryStackMethod::Percentile => {
                self.stack_percentile(&indices, self.config.percentile)
            }
            PlanetaryStackMethod::WeightedMean => self.stack_weighted_mean(&indices),
        }
    }

    fn select_best_frames(&self) -> Vec<usize> {
        let total_frames = self.frames.len();
        let selected_count = self.config.compute_selected_count(total_frames);

        let mut indices: Vec<usize> = (0..total_frames).collect();
        indices.sort_by(|&a, &b| {
            self.frames[b]
                .quality
                .partial_cmp(&self.frames[a].quality)
                .unwrap()
        });

        indices.into_iter().take(selected_count).collect()
    }

    fn stack_mean(&self, indices: &[usize]) -> Result<Frame> {
        let pixel_count = self.width * self.height * self.channels;
        let mut sum = vec![0.0f64; pixel_count];
        let mut count = vec![0u32; pixel_count];

        for &idx in indices {
            let scored = &self.frames[idx];
            let aligned = self.apply_offset(&scored.frame, scored.offset)?;
            let data = aligned.data();

            for (i, &v) in data.iter().enumerate() {
                if v > 0.0 {
                    sum[i] += v as f64;
                    count[i] += 1;
                }
            }
        }

        let result: Vec<f32> = sum
            .iter()
            .zip(count.iter())
            .map(|(&s, &c)| if c > 0 { (s / c as f64) as f32 } else { 0.0 })
            .collect();

        Frame::from_f32_vec(result, self.width, self.height, self.channels)
    }

    fn stack_percentile(&self, indices: &[usize], percentile: f32) -> Result<Frame> {
        let pixel_count = self.width * self.height * self.channels;

        let aligned_frames: Vec<Frame> = indices
            .iter()
            .map(|&idx| {
                let scored = &self.frames[idx];
                self.apply_offset(&scored.frame, scored.offset)
            })
            .collect::<Result<Vec<_>>>()?;

        let result: Vec<f32> = (0..pixel_count)
            .into_par_iter()
            .map(|pixel_idx| {
                let mut values: Vec<f32> = aligned_frames
                    .iter()
                    .map(|f| f.data()[pixel_idx])
                    .filter(|&v| v > 0.0)
                    .collect();

                if values.is_empty() {
                    return 0.0;
                }

                values.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let idx = ((values.len() - 1) as f32 * percentile).round() as usize;
                values[idx.min(values.len() - 1)]
            })
            .collect();

        Frame::from_f32_vec(result, self.width, self.height, self.channels)
    }

    fn stack_weighted_mean(&self, indices: &[usize]) -> Result<Frame> {
        let pixel_count = self.width * self.height * self.channels;
        let mut weighted_sum = vec![0.0f64; pixel_count];
        let mut weight_sum = vec![0.0f64; pixel_count];

        let weights = self.compute_normalized_weights(indices);

        for (i, &idx) in indices.iter().enumerate() {
            let scored = &self.frames[idx];
            let weight = weights[i];
            let aligned = self.apply_offset(&scored.frame, scored.offset)?;
            let data = aligned.data();

            for (j, &v) in data.iter().enumerate() {
                if v > 0.0 {
                    weighted_sum[j] += v as f64 * weight;
                    weight_sum[j] += weight;
                }
            }
        }

        let result: Vec<f32> = weighted_sum
            .iter()
            .zip(weight_sum.iter())
            .map(|(&ws, &w)| if w > 0.0 { (ws / w) as f32 } else { 0.0 })
            .collect();

        Frame::from_f32_vec(result, self.width, self.height, self.channels)
    }

    fn compute_normalized_weights(&self, indices: &[usize]) -> Vec<f64> {
        let qualities: Vec<f32> = indices.iter().map(|&i| self.frames[i].quality).collect();
        let min_q = qualities.iter().cloned().fold(f32::MAX, f32::min);
        let max_q = qualities.iter().cloned().fold(f32::MIN, f32::max);
        let range = (max_q - min_q).max(1e-6);

        qualities
            .iter()
            .map(|&q| ((q - min_q) / range + 0.1) as f64)
            .collect()
    }

    /// Resamples `frame` by a whole-frame sub-pixel `offset`.
    ///
    /// Written plane by plane and in parallel. The previous version called a
    /// `bilinear_sample` that returned `Option<Vec<f32>>`, i.e. one heap allocation per
    /// output pixel — 4.17 million of them per aligned frame at IMX464 resolution, on a
    /// single thread. `BilinearTap` resolves the source position once per pixel and the
    /// three planes then read through it, so the O(1) work per pixel is shared rather
    /// than repeated per channel.
    fn apply_offset(&self, frame: &Frame, offset: (f32, f32)) -> Result<Frame> {
        if offset.0.abs() < 0.001 && offset.1.abs() < 0.001 {
            return Ok(frame.clone());
        }

        let width = frame.width();
        let height = frame.height();
        let channels = frame.channels();
        let (dx, dy) = offset;

        let mut out = Frame::zeros(width, height, channels)?;
        let chunk_rows = crate::parallel::balanced_chunk_len(width * height).div_ceil(width).max(1);

        if channels == 3 {
            let (src_r, src_g, src_b) = frame.planes();
            let (dst_r, dst_g, dst_b) = out.planes_mut();
            dst_r
                .par_chunks_mut(width * chunk_rows)
                .zip(dst_g.par_chunks_mut(width * chunk_rows))
                .zip(dst_b.par_chunks_mut(width * chunk_rows))
                .enumerate()
                .for_each(|(block, ((r_block, g_block), b_block))| {
                    let y_start = block * chunk_rows;
                    for (row, ((r_row, g_row), b_row)) in r_block
                        .chunks_mut(width)
                        .zip(g_block.chunks_mut(width))
                        .zip(b_block.chunks_mut(width))
                        .enumerate()
                    {
                        let y = y_start + row;
                        for x in 0..width {
                            let Some(tap) = BilinearTap::at(
                                x as f32 - dx,
                                y as f32 - dy,
                                width,
                                height,
                            ) else {
                                continue;
                            };
                            r_row[x] = tap.sample(src_r);
                            g_row[x] = tap.sample(src_g);
                            b_row[x] = tap.sample(src_b);
                        }
                    }
                });
            return Ok(out);
        }

        for c in 0..channels {
            let src = frame.channel_data(c);
            out.channel_data_mut(c)
                .par_chunks_mut(width * chunk_rows)
                .enumerate()
                .for_each(|(block, rows)| {
                    let y_start = block * chunk_rows;
                    for (row, out_row) in rows.chunks_mut(width).enumerate() {
                        let y = y_start + row;
                        for x in 0..width {
                            if let Some(tap) =
                                BilinearTap::at(x as f32 - dx, y as f32 - dy, width, height)
                            {
                                out_row[x] = tap.sample(src);
                            }
                        }
                    }
                });
        }

        Ok(out)
    }

    /// Clears all frames and resets the stacker
    pub fn clear(&mut self) {
        self.frames.clear();
        self.reference = None;
    }

    /// Returns stacking statistics
    pub fn statistics(&self) -> PlanetaryStackStats {
        if self.frames.is_empty() {
            return PlanetaryStackStats::default();
        }

        let qualities: Vec<f32> = self.frames.iter().map(|f| f.quality).collect();
        let offsets: Vec<(f32, f32)> = self.frames.iter().map(|f| f.offset).collect();

        let min_quality = qualities.iter().cloned().fold(f32::MAX, f32::min);
        let max_quality = qualities.iter().cloned().fold(f32::MIN, f32::max);
        let mean_quality = qualities.iter().sum::<f32>() / qualities.len() as f32;

        let max_offset = offsets
            .iter()
            .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
            .fold(0.0f32, f32::max);

        let selected_frames = self.config.compute_selected_count(self.frames.len());

        PlanetaryStackStats {
            total_frames: self.frames.len(),
            selected_frames,
            min_quality,
            max_quality,
            mean_quality,
            max_offset,
        }
    }
}

/// Samples a pixel with bilinear interpolation, returning None if out of bounds.
/// The four plane-relative offsets and weights one output pixel interpolates from.
///
/// Plane-relative on purpose: the same tap serves every channel, so a colour frame
/// resolves the source position once per pixel rather than once per pixel per channel.
/// That is also why this is `pub`: Pro's planetary stacker resamples through the same
/// four taps (with an IDW displacement instead of a whole-frame offset), and a second
/// copy over there is one more place for the two to disagree about layout.
pub struct BilinearTap {
    base00: usize,
    base10: usize,
    base01: usize,
    base11: usize,
    w00: f32,
    w10: f32,
    w01: f32,
    w11: f32,
}

impl BilinearTap {
    /// `None` when the source position falls outside the frame, in which case the
    /// caller leaves that output pixel at its initial value.
    #[inline]
    pub fn at(src_x: f32, src_y: f32, width: usize, height: usize) -> Option<Self> {
        if src_x < 0.0 || src_x >= (width - 1) as f32 || src_y < 0.0 || src_y >= (height - 1) as f32
        {
            return None;
        }

        let x0 = src_x.floor() as usize;
        let y0 = src_y.floor() as usize;
        let fx = src_x - x0 as f32;
        let fy = src_y - y0 as f32;

        let base00 = y0 * width + x0;
        Some(Self {
            base00,
            base10: base00 + 1,
            base01: base00 + width,
            base11: base00 + width + 1,
            w00: (1.0 - fx) * (1.0 - fy),
            w10: fx * (1.0 - fy),
            w01: (1.0 - fx) * fy,
            w11: fx * fy,
        })
    }

    /// Interpolates one plane. The plane must be `width * height` long and come from
    /// the frame the tap was built against.
    #[inline]
    pub fn sample(&self, plane: &[f32]) -> f32 {
        self.w00 * plane[self.base00]
            + self.w10 * plane[self.base10]
            + self.w01 * plane[self.base01]
            + self.w11 * plane[self.base11]
    }
}

/// Convenience function to stack a slice of frames
pub fn stack_planetary(frames: &[Frame], config: PlanetaryConfig) -> Result<Frame> {
    if frames.is_empty() {
        return Err(StackError::InvalidConfiguration(
            "No frames to stack".to_string(),
        ));
    }

    let mut stacker = PlanetaryStacker::new(config);

    for frame in frames {
        stacker.add_frame(frame)?;
    }

    stacker.stack()
}
