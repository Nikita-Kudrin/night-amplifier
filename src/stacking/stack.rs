//! Master stack accumulator for live stacking.
//!
//! Accumulates frames using true O(1) incremental stacking.
//! Frame history is completely discarded in favor of running statistics,
//! providing instantaneous compute times and a flat memory footprint.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::telemetry::metrics as telemetry_metrics;
use rayon::prelude::*;
use tracing::{info_span, warn};

use super::config::{FrameQuality, StackingConfig};
use super::incremental_pixel::IncrementalPixel;
use super::quality_limits::QualityLimits;
use super::rejection::{RejectionMethod, REJECTION_PLUGIN};

pub struct MasterStack {
    width: usize,
    height: usize,
    channels: usize,
    config: StackingConfig,
    frame_count: usize,
    pixels: Vec<IncrementalPixel>,
    quality_limits: QualityLimits,
    frame_qualities: Vec<FrameQuality>,
}

impl MasterStack {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        config: StackingConfig,
    ) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        if matches!(
            config.rejection,
            RejectionMethod::SigmaClip
                | RejectionMethod::WinsorizedSigmaClip
                | RejectionMethod::MinMax
        ) {
            if REJECTION_PLUGIN.get().is_none() {
                return Err(StackError::InvalidConfiguration(
                    "Advanced outlier rejection (Sigma Clipping, MinMax) is only available in Night Amplifier Pro.\n\
                     Please consider upgrading to unlock this feature.".into(),
                ));
            }
        }

        let pixel_count = width * height * channels;
        Ok(Self {
            width,
            height,
            channels,
            config,
            frame_count: 0,
            pixels: vec![IncrementalPixel::new(); pixel_count],
            quality_limits: QualityLimits::default(),
            frame_qualities: Vec::new(),
        })
    }

    pub fn with_defaults(width: usize, height: usize, channels: usize) -> Result<Self> {
        Self::new(width, height, channels, StackingConfig::default())
    }

    pub fn add_frame(&mut self, frame: &Frame) -> Result<()> {
        self.add_frame_with_quality(frame, FrameQuality::default())
    }

    pub fn add_frame_with_quality(&mut self, frame: &Frame, quality: FrameQuality) -> Result<()> {
        self.add_frame_with_border_and_quality(frame, 0.0, 1e-6, quality)
    }

    pub fn add_frame_with_border(
        &mut self,
        frame: &Frame,
        border_value: f32,
        border_tolerance: f32,
    ) -> Result<()> {
        self.add_frame_with_border_and_quality(
            frame,
            border_value,
            border_tolerance,
            FrameQuality::default(),
        )
    }

    pub fn add_frame_with_border_and_quality(
        &mut self,
        frame: &Frame,
        border_value: f32,
        border_tolerance: f32,
        quality: FrameQuality,
    ) -> Result<()> {
        if frame.width() != self.width
            || frame.height() != self.height
            || frame.channels() != self.channels
        {
            return Err(StackError::CalibrationDimensionMismatch {
                frame_width: frame.width(),
                frame_height: frame.height(),
                cal_width: self.width,
                cal_height: self.height,
            });
        }

        let data = frame.data();

        // 1. Update running quality limits and calculate this frame's dynamic weight
        self.quality_limits.update(&quality);
        let weight = self
            .quality_limits
            .calculate_weight(&quality, &self.config.weighting);

        let sigma_low = self.config.sigma_low;
        let sigma_high = self.config.sigma_high;
        let min_frames = self.config.min_frames_for_rejection as u16;

        let needs_rejection = matches!(
            self.config.rejection,
            RejectionMethod::SigmaClip | RejectionMethod::WinsorizedSigmaClip
        );

        if needs_rejection {
            if let Some(plugin) = REJECTION_PLUGIN.get() {
                plugin.blend_incremental(
                    &mut self.pixels,
                    data,
                    border_value,
                    border_tolerance,
                    weight,
                    &self.config,
                )?;
            } else {
                return Err(StackError::InvalidConfiguration(
                    "Advanced outlier rejection is a Pro feature.".into(),
                ));
            }
        } else {
            let _store_span =
                info_span!("blend_pixels", frame_count = self.frame_count + 1).entered();

            // 2. Blend the frame directly into the Master result in O(1) memory (No rejection)
            self.pixels
                .par_iter_mut()
                .zip(data.par_iter())
                .for_each(|(pixel, &val)| {
                    // Ignore borders
                    if (val - border_value).abs() < border_tolerance {
                        return;
                    }

                    // Blend into running average
                    pixel.blend(val, weight);
                });

            drop(_store_span);
        }

        self.frame_count += 1;
        self.frame_qualities.push(quality);
        Ok(())
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
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

    pub fn frame_qualities(&self) -> &[FrameQuality] {
        &self.frame_qualities
    }

    pub fn compute(&self) -> Result<Frame> {
        let pixel_count = self.width * self.height * self.channels;
        let mut result = vec![0.0f32; pixel_count];

        let _span = info_span!("compute_pixels").entered();

        // Just extract the running mean. Zero math required!
        result
            .par_iter_mut()
            .zip(self.pixels.par_iter())
            .for_each(|(res, p)| {
                *res = p.mean;
            });

        Frame::from_f32_vec(result, self.width, self.height, self.channels)
    }

    pub fn config(&self) -> &StackingConfig {
        &self.config
    }

    pub fn coverage_map(&self) -> Frame {
        let max_count = self.frame_count as f32;
        let data: Vec<f32> = self
            .pixels
            .iter()
            .map(|p| p.count as f32 / max_count.max(1.0))
            .collect();

        Frame::from_f32_vec(data, self.width, self.height, self.channels)
            .expect("Coverage map creation should not fail")
    }

    pub fn clear(&mut self) {
        self.frame_count = 0;
        self.pixels.par_iter_mut().for_each(|p| p.reset());
        self.quality_limits = QualityLimits::default();
        self.frame_qualities.clear();
    }

    /// Update the stacking configuration dynamically.
    ///
    /// This allows changing rejection methods or sigma thresholds mid-stack.
    /// Subsequent frames will use the new configuration.
    pub fn update_config(&mut self, mut config: StackingConfig) {
        // Enforce Pro gating during dynamic updates
        if matches!(
            config.rejection,
            RejectionMethod::SigmaClip
                | RejectionMethod::WinsorizedSigmaClip
                | RejectionMethod::MinMax
        ) {
            if REJECTION_PLUGIN.get().is_none() {
                warn!("Ignoring request for advanced rejection method - Night Amplifier Pro required.");
                config.rejection = RejectionMethod::None;
            }
        }
        self.config = config;
    }

    pub fn memory_usage(&self) -> usize {
        self.pixels.len() * std::mem::size_of::<IncrementalPixel>()
    }

    pub fn record_metrics(&self, stack_id: &str) {
        let pixel_count = (self.width * self.height * self.channels) as u64;

        telemetry_metrics::record_master_stack_memory(self.memory_usage() as u64, stack_id);
        telemetry_metrics::record_master_stack_frame_count(self.frame_count as u64, stack_id);
        telemetry_metrics::record_master_stack_qualities_count(
            self.frame_qualities.len() as u64,
            stack_id,
        );
        telemetry_metrics::record_master_stack_pixel_count(pixel_count, stack_id);
    }
}
