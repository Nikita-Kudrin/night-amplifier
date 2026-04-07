//! Outlier rejection algorithms for image stacking (Community Stub)
//!
//! Advanced rejection methods (Sigma Clipping, MinMax) are executed and optimized
//! in the Night Amplifier Pro version.

use crate::error::{Result, StackError};
use crate::stacking::config::StackingConfig;
use crate::stacking::incremental_pixel::IncrementalPixel;
use std::sync::OnceLock;

/// Rejection method for stacking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RejectionMethod {
    /// No rejection - simple average of all frames
    None,
    /// Sigma clipping: reject values > N sigma from mean (Pro only)
    SigmaClip,
    /// Winsorized sigma clipping: clip outliers to threshold instead of rejecting (Pro only)
    WinsorizedSigmaClip,
    /// Min-max rejection: discard min and max, average the rest (Pro only)
    MinMax,
}

impl Default for RejectionMethod {
    fn default() -> Self {
        Self::None
    }
}

/// Plugin trait for advanced outlier rejection methods
pub trait RejectionPlugin: Send + Sync {
    fn is_enabled(&self) -> bool {
        true
    }

    fn compute_rejection(
        &self,
        pixel_data: &[f32],
        method: RejectionMethod,
        config: &StackingConfig,
    ) -> Result<(f32, u32)>;

    fn compute_weighted_rejection(
        &self,
        pixel_data: &[f32],
        weights: &[f32],
        method: RejectionMethod,
        config: &StackingConfig,
    ) -> Result<(f32, f32)>;

    fn blend_incremental(
        &self,
        pixels: &mut [IncrementalPixel],
        frame_data: &[f32],
        border_value: f32,
        border_tolerance: f32,
        weight: f32,
        config: &StackingConfig,
    ) -> Result<()>;
}

/// Global registry for the rejection plugin
pub static REJECTION_PLUGIN: OnceLock<Box<dyn RejectionPlugin>> = OnceLock::new();
