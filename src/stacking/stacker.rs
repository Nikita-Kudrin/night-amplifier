//! Stacking engine that combines warping and stacking steps.
//!
//! Provides a higher-level interface for the complete stacking workflow:
//! 1. Accept a new frame
//! 2. Warp it using the provided transform
//! 3. Add it to the master stack
//!
//! # Weighted Stacking
//!
//! Supports quality-based frame weighting via `add_frame_with_quality` and
//! `add_reference_with_quality`. When `WeightingConfig` is enabled in the
//! stacking configuration, frames with better FWHM (sharpness) or SNR
//! will contribute more to the final result.

use crate::error::Result;
use crate::frame::Frame;
use crate::registration::AffineTransform;
use tracing::{info_span, instrument, Span};

use super::config::{FrameQuality, StackingConfig};
use super::stack::MasterStack;
use super::warp::warp_frame_into;

/// Stacking engine that combines all steps.
///
/// Provides a higher-level interface for the complete stacking workflow:
/// 1. Accept a new frame
/// 2. Warp it using the provided transform
/// 3. Add it to the master stack
pub struct Stacker {
    stack: MasterStack,
    warp_buffer: Frame,
    border_value: f32,
}

impl Stacker {
    /// Creates a new stacker with the given dimensions.
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        config: StackingConfig,
    ) -> Result<Self> {
        let stack = MasterStack::new(width, height, channels, config)?;
        let warp_buffer = Frame::zeros(width, height, channels)?;

        Ok(Self {
            stack,
            warp_buffer,
            border_value: 0.0,
        })
    }

    /// Creates a stacker with default configuration.
    pub fn with_defaults(width: usize, height: usize, channels: usize) -> Result<Self> {
        Self::new(width, height, channels, StackingConfig::default())
    }

    /// Sets the border value used for warping.
    pub fn set_border_value(&mut self, value: f32) {
        self.border_value = value;
    }

    /// Adds the reference frame (no warping needed).
    ///
    /// Uses default quality (equal weight). For weighted stacking,
    /// use `add_reference_with_quality` instead.
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        channels = frame.channels(),
        frame_count = tracing::field::Empty
    ))]
    pub fn add_reference(&mut self, frame: &Frame) -> Result<()> {
        let result = self.stack.add_frame(frame);
        Span::current().record("frame_count", self.stack.frame_count());
        result
    }

    /// Adds the reference frame with quality metrics for weighted stacking.
    ///
    /// # Arguments
    /// * `frame` - Reference frame (not warped)
    /// * `quality` - Quality metrics for this frame
    pub fn add_reference_with_quality(
        &mut self,
        frame: &Frame,
        quality: FrameQuality,
    ) -> Result<()> {
        self.stack.add_frame_with_quality(frame, quality)
    }

    /// Adds a frame with the specified transform.
    ///
    /// The frame is warped using the transform, then added to the stack.
    /// Uses default quality (equal weight). For weighted stacking,
    /// use `add_frame_with_quality` instead.
    #[instrument(skip(self, frame, transform), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        rotation_deg = transform.rotation.to_degrees(),
        translation = %format!("({:.1}, {:.1})", transform.tx, transform.ty),
        frame_count = tracing::field::Empty
    ))]
    pub fn add_frame(&mut self, frame: &Frame, transform: &AffineTransform) -> Result<()> {
        {
            let _warp_span =
                info_span!("warp_frame", width = frame.width(), height = frame.height(),).entered();
            warp_frame_into(frame, transform, &mut self.warp_buffer, self.border_value)?;
        }
        let result = self
            .stack
            .add_frame_with_border(&self.warp_buffer, self.border_value, 1e-6);
        Span::current().record("frame_count", self.stack.frame_count());
        result
    }

    /// Adds a frame with the specified transform and quality metrics.
    ///
    /// The frame is warped using the transform, then added to the stack
    /// with the provided quality metrics for weighted stacking.
    ///
    /// # Arguments
    /// * `frame` - Frame to add
    /// * `transform` - Affine transform to apply (computed from registration)
    /// * `quality` - Quality metrics for this frame (FWHM, SNR)
    pub fn add_frame_with_quality(
        &mut self,
        frame: &Frame,
        transform: &AffineTransform,
        quality: FrameQuality,
    ) -> Result<()> {
        {
            let _warp_span =
                info_span!("warp_frame", width = frame.width(), height = frame.height(),).entered();
            warp_frame_into(frame, transform, &mut self.warp_buffer, self.border_value)?;
        }
        self.stack.add_frame_with_border_and_quality(
            &self.warp_buffer,
            self.border_value,
            1e-6,
            quality,
        )
    }

    /// Returns the frame quality metrics for all accumulated frames.
    pub fn frame_qualities(&self) -> &[FrameQuality] {
        self.stack.frame_qualities()
    }

    /// Returns the number of frames in the stack.
    pub fn frame_count(&self) -> usize {
        self.stack.frame_count()
    }

    pub fn width(&self) -> usize {
        self.stack.width()
    }

    pub fn height(&self) -> usize {
        self.stack.height()
    }

    pub fn channels(&self) -> usize {
        self.stack.channels()
    }

    /// Computes the current stacked result.
    #[instrument(skip(self), fields(
        frame_count = self.stack.frame_count()
    ))]
    pub fn compute(&self) -> Result<Frame> {
        self.stack.compute()
    }

    /// Clears the stack for reuse.
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Update the stacking configuration dynamically.
    pub fn update_config(&mut self, config: StackingConfig) {
        self.stack.update_config(config);
    }

    /// Returns the coverage map.
    pub fn coverage_map(&self) -> Frame {
        self.stack.coverage_map()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_spot_frame(width: usize, height: usize, spot_x: usize, spot_y: usize) -> Frame {
        let mut frame = Frame::filled(width, height, 3, 0.1).unwrap();
        let data = frame.data_mut();

        for dy in 0..height {
            for dx in 0..width {
                let dist_sq =
                    (dx as f32 - spot_x as f32).powi(2) + (dy as f32 - spot_y as f32).powi(2);
                let intensity = (-dist_sq / 50.0).exp();

                let idx = (dy * width + dx) * 3;
                data[idx] += intensity;
                data[idx + 1] += intensity;
                data[idx + 2] += intensity;
            }
        }

        frame
    }

    #[test]
    fn test_stacker() {
        let mut stacker = Stacker::with_defaults(32, 32, 3).unwrap();

        let ref_frame = create_spot_frame(32, 32, 16, 16);
        stacker.add_reference(&ref_frame).unwrap();

        let shifted_frame = create_spot_frame(32, 32, 18, 17);
        let transform = AffineTransform::new(0.0, 1.0, -2.0, -1.0);
        stacker.add_frame(&shifted_frame, &transform).unwrap();

        assert_eq!(stacker.frame_count(), 2);

        let result = stacker.compute().unwrap();
        assert_eq!(result.width(), 32);
        assert_eq!(result.height(), 32);
    }
}
