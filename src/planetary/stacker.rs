//! Planetary stacker engine for collecting, scoring, aligning, and stacking frames.

use rayon::prelude::*;

use crate::error::{Result, StackError};
use crate::frame::Frame;

use super::alignment::compute_alignment;
use super::config::{
    AlignmentRoi, PlanetaryConfig, PlanetaryStackMethod, PlanetaryStackStats, QualityMetric,
};
use super::quality::compute_quality;
use std::sync::OnceLock;

/// PlanetaryStacker trait for the Pro plugin
pub trait PlanetaryStackerPlugin: Send + Sync {
    fn add_frame(&self, frame: &Frame, stacker: &mut PlanetaryStacker) -> Result<f32>;
    fn stack(&self, stacker: &PlanetaryStacker) -> Result<Frame>;
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
                    return Ok(quality);
                }
            }
        }

        self.frames.push(ScoredFrame {
            frame: frame.clone(),
            quality,
            offset,
        });

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
        let roi = self.config.alignment_roi.unwrap_or_else(|| {
            let size = (self.width.min(self.height) / 2).max(64);
            AlignmentRoi::centered(self.width, self.height, size)
        });

        compute_alignment(
            reference,
            frame,
            &roi,
            self.config.search_radius,
            self.config.subpixel_factor,
        )
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

    fn apply_offset(&self, frame: &Frame, offset: (f32, f32)) -> Result<Frame> {
        if offset.0.abs() < 0.001 && offset.1.abs() < 0.001 {
            return Ok(frame.clone());
        }

        let width = frame.width();
        let height = frame.height();
        let channels = frame.channels();
        let src_data = frame.data();

        let mut result = vec![0.0f32; width * height * channels];
        let (dx, dy) = offset;

        for y in 0..height {
            for x in 0..width {
                let src_x = x as f32 - dx;
                let src_y = y as f32 - dy;

                if let Some(value) =
                    bilinear_sample(src_data, width, height, channels, src_x, src_y)
                {
                    let dst_idx = (y * width + x) * channels;
                    for c in 0..channels {
                        result[dst_idx + c] = value[c];
                    }
                }
            }
        }

        Frame::from_f32_vec(result, width, height, channels)
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
fn bilinear_sample(
    src_data: &[f32],
    width: usize,
    height: usize,
    channels: usize,
    src_x: f32,
    src_y: f32,
) -> Option<Vec<f32>> {
    if src_x < 0.0 || src_x >= (width - 1) as f32 || src_y < 0.0 || src_y >= (height - 1) as f32 {
        return None;
    }

    let x0 = src_x.floor() as usize;
    let y0 = src_y.floor() as usize;
    let x1 = x0 + 1;
    let y1 = y0 + 1;

    let fx = src_x - x0 as f32;
    let fy = src_y - y0 as f32;

    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;

    let mut result = Vec::with_capacity(channels);
    for c in 0..channels {
        let v00 = src_data[(y0 * width + x0) * channels + c];
        let v10 = src_data[(y0 * width + x1) * channels + c];
        let v01 = src_data[(y1 * width + x0) * channels + c];
        let v11 = src_data[(y1 * width + x1) * channels + c];

        result.push(w00 * v00 + w10 * v10 + w01 * v01 + w11 * v11);
    }

    Some(result)
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
