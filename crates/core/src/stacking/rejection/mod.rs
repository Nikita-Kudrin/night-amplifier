//! Outlier rejection algorithms for image stacking (Community Stub)
//!
//! Advanced rejection methods (Sigma Clipping, MinMax) are executed and optimized
//! in the Night Amplifier Pro version.

use crate::error::{Result, StackError};
use crate::stacking::config::StackingConfig;
use crate::stacking::incremental_pixel::IncrementalPixel;

/// Rejection method for stacking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum RejectionMethod {
    /// No rejection - simple average of all frames
    #[default]
    None,
    /// Sigma clipping: reject values > N sigma from mean (Pro only)
    SigmaClip,
    /// Winsorized sigma clipping: clip outliers to threshold instead of rejecting (Pro only)
    WinsorizedSigmaClip,
    /// Min-max rejection: discard min and max, average the rest (Pro only)
    MinMax,
}

impl RejectionMethod {
    /// What a session asks for when the observer never chose: sigma clipping in a build
    /// that ships the rejection plugin, `None` in Community, which has nothing else to
    /// run. Keyed on the build, not the licence, so a lapsed licence never rewrites a
    /// saved choice — the live path already resolves an unlicensed one to `None`.
    pub fn best_available() -> Self {
        if crate::plugins::Plugins::installed().ships_rejection() {
            Self::SigmaClip
        } else {
            Self::None
        }
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
