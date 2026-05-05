//! Unified Render Pipeline orchestration
//!
//! This module provides the central RenderPipeline which coordinates the sequence
//! of processing stages: background subtraction, autostretching, saturation boosting,
//! and contrast adjustment.

use crate::background::subtract_background_with_config;
use crate::error::{Result, StackError};
use crate::frame::Frame;
use tracing::{debug, field, instrument, warn, Span};

mod config;
pub use config::RenderPipelineConfig;

use super::autostretch::{auto_stretch_frame, AutoStretchResult};
use super::output::apply_contrast_frame;
use super::stretch::apply_shadow_saturation_boost;

/// Result from processing a frame through the pipeline
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Auto-stretch result (if auto-stretch was applied)
    pub stretch_result: Option<AutoStretchResult>,
    /// Whether background subtraction was applied
    pub background_subtracted: bool,
    /// Whether saturation boost was applied
    pub saturation_boosted: bool,
    /// Whether contrast was applied
    pub contrast_applied: bool,
}

/// Unified render pipeline for processing astronomical frames
pub struct RenderPipeline {
    config: RenderPipelineConfig,
}

impl RenderPipeline {
    /// Create a new render pipeline with the given configuration
    pub fn new(config: RenderPipelineConfig) -> Self {
        Self { config }
    }

    /// Create a pipeline with default configuration
    pub fn with_defaults() -> Self {
        Self::new(RenderPipelineConfig::default())
    }

    /// Get a reference to the current configuration
    pub fn config(&self) -> &RenderPipelineConfig {
        &self.config
    }

    /// Get a mutable reference to the configuration
    pub fn config_mut(&mut self) -> &mut RenderPipelineConfig {
        &mut self.config
    }

    /// Process a frame through the complete pipeline in-place
    #[instrument(skip(self, frame), fields(
        width = frame.width(),
        height = frame.height(),
        channels = frame.channels(),
        background_subtracted = field::Empty,
        auto_stretched = field::Empty,
        saturation_boosted = field::Empty,
        contrast_applied = field::Empty,
    ))]
    pub fn process(&self, frame: &mut Frame) -> Result<PipelineResult> {
        let channels = frame.channels();
        if channels != 1 && channels != 3 {
            return Err(StackError::InvalidConfiguration(format!(
                "RenderPipeline requires 1 or 3 channels, got {}",
                channels
            )));
        }

        let mut result = PipelineResult {
            stretch_result: None,
            background_subtracted: false,
            saturation_boosted: false,
            contrast_applied: false,
        };

        // Stage 1: Background subtraction
        if self.config.background_subtraction {
            let _span = tracing::info_span!("background_subtraction").entered();
            if let Err(e) =
                subtract_background_with_config(frame, self.config.background_config.clone())
            {
                warn!(error = %e, "Background subtraction failed");
            } else {
                result.background_subtracted = true;
                debug!(algorithm = %self.config.background_config.algorithm, "Background subtraction applied");
            }
        }

        // Stage 2: Auto-stretch
        if self.config.auto_stretch {
            let _span = tracing::info_span!("auto_stretch").entered();
            match auto_stretch_frame(frame, self.config.stretch_config) {
                Ok(stretch_result) => {
                    result.stretch_result = Some(stretch_result);
                    debug!(
                        stretch_factor = stretch_result.stretch_factor,
                        black_point = stretch_result.black_point,
                        "Auto-stretch applied"
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Auto-stretch failed");
                }
            }
        }

        // Stage 3: Saturation boost (RGB only)
        if self.config.saturation_boost && channels == 3 {
            let _span = tracing::info_span!("saturation_boost").entered();
            let mut sat_config = self.config.saturation_config;
            sat_config.enabled = true;

            if let Err(e) = apply_shadow_saturation_boost(frame, &sat_config) {
                warn!(error = %e, "Saturation boost failed");
            } else {
                result.saturation_boosted = true;
                debug!("Saturation boost applied");
            }
        }

        // Stage 4: Contrast adjustment (RGB only for now)
        if self.config.contrast && channels == 3 && !self.config.contrast_config.is_disabled() {
            let _span = tracing::info_span!("contrast_adjustment").entered();
            if let Err(e) = apply_contrast_frame(frame, &self.config.contrast_config) {
                warn!(error = %e, "Contrast adjustment failed");
            } else {
                result.contrast_applied = true;
                debug!("Contrast adjustment applied");
            }
        }

        let span = Span::current();
        span.record("background_subtracted", result.background_subtracted);
        span.record("auto_stretched", result.stretch_result.is_some());
        span.record("saturation_boosted", result.saturation_boosted);
        span.record("contrast_applied", result.contrast_applied);

        Ok(result)
    }

    /// Process a frame and return a new frame (non-mutating)
    pub fn process_copy(&self, frame: &Frame) -> Result<(Frame, PipelineResult)> {
        let mut output = frame.clone();
        let result = self.process(&mut output)?;
        Ok((output, result))
    }
}

/// Convenience function: process frame with default pipeline settings
pub fn process_frame_default(frame: &mut Frame) -> Result<PipelineResult> {
    RenderPipeline::with_defaults().process(frame)
}

/// Convenience function: process frame for deep sky imaging
pub fn process_frame_deep_sky(frame: &mut Frame) -> Result<PipelineResult> {
    RenderPipeline::new(RenderPipelineConfig::deep_sky()).process(frame)
}

/// Convenience function: process frame for planetary imaging
pub fn process_frame_planetary(frame: &mut Frame) -> Result<PipelineResult> {
    RenderPipeline::new(RenderPipelineConfig::planetary()).process(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::autostretch::AutoStretchConfig;
    use crate::render::output::ContrastConfig;
    use crate::render::stretch::SaturationBoostConfig;

    #[test]
    fn test_pipeline_config_presets() {
        let config = RenderPipelineConfig::deep_sky();
        assert!(config.background_subtraction);
        assert!(config.auto_stretch);

        let config_p = RenderPipelineConfig::planetary();
        assert!(!config_p.background_subtraction);
        assert!(config_p.auto_stretch);
    }

    #[test]
    fn test_pipeline_process_rgb() {
        let data = vec![0.05f32; 64 * 64 * 3];
        let mut frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let pipeline = RenderPipeline::with_defaults();
        let result = pipeline.process(&mut frame).unwrap();
        assert!(result.stretch_result.is_some());
    }

    #[test]
    fn test_pipeline_all_stages() {
        let data = vec![0.05f32; 32 * 32 * 3];
        let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();
        let config = RenderPipelineConfig::deep_sky();
        let pipeline = RenderPipeline::new(config);
        let result = pipeline.process(&mut frame).unwrap();
        assert!(result.background_subtracted);
        assert!(result.stretch_result.is_some());
        assert_eq!(
            result.saturation_boosted,
            crate::license::pro_plugin(&crate::render::SATURATION_PLUGIN).is_some()
        );
    }
}
