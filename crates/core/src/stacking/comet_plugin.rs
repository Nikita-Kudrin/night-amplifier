//! Traits for comet stacking plugins.
//!
//! This module defines the interface that must be implemented by
//! comet stacking plugins. The community version of Night Amplifier
//! provides the interface but not the implementation.

use super::StackingConfig;
use crate::plugins::Plugins;
use crate::error::Result;
use crate::frame::Frame;
use crate::planetary::AlignmentRoi;

/// What a comet context reads from the session's settings.
#[derive(Debug, Clone)]
pub struct CometSettings {
    /// The box the nucleus is searched in; `None` lets the plugin centre its own.
    pub roi: Option<AlignmentRoi>,
    /// The accumulator the observer asked for, rejection already reduced to what the
    /// live path runs.
    pub stacking: StackingConfig,
    /// What the context's own accumulator runs with.
    pub plugins: Plugins,
}

/// Comet centroid detection result
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CometCentroid {
    pub x: f32,
    pub y: f32,
    pub total_flux: f32,
    pub snr: f32,
}

/// Interface for a comet stacking implementation.
pub trait CometContext: Send + Sync {
    /// Update the ROI for detection.
    fn update_roi(&mut self, roi: AlignmentRoi);

    /// Initialize the stack with a reference frame.
    fn initialize_with_reference(&mut self, frame: &Frame) -> Result<()>;

    /// Add a frame to the comet stack.
    /// Returns true if the frame was successfully added.
    fn add_frame(&mut self, frame: &Frame) -> Result<bool>;

    /// Compute the current stacked result.
    fn compute(&self) -> Result<Frame>;

    /// Returns the number of frames in the stack.
    fn frame_count(&self) -> usize;

    /// Returns the width of the stack.
    fn width(&self) -> usize;

    /// Returns the height of the stack.
    fn height(&self) -> usize;

    /// Returns the number of channels in the stack.
    fn channels(&self) -> usize;

    /// Update stacking parameters from settings.
    fn update_from_settings(&mut self, settings: &CometSettings);

    /// Get current detector ROI (for UI/tracking)
    fn get_roi(&self) -> AlignmentRoi;
}

/// Interface for creating comet stacking contexts.
pub trait CometPlugin: Send + Sync {
    /// Create a new comet stacking context.
    fn create_context(
        &self,
        width: usize,
        height: usize,
        channels: usize,
        settings: &CometSettings,
    ) -> Box<dyn CometContext>;
}
