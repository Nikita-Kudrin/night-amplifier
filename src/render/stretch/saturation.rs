use crate::error::{Result, StackError};
use crate::frame::Frame;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::instrument;

/// Plugin trait for shadow saturation boost (Commercial feature)
pub trait SaturationPlugin: Send + Sync {
    /// Apply saturation boost to an RGB frame
    fn apply_boost(&self, frame: &mut Frame, config: &SaturationBoostConfig) -> Result<()>;

    /// Apply saturation boost to a flat row of interleaved RGB f32 samples.
    ///
    /// Called per-row inside the fused encode kernels (`expand_to_rgb8_fused`,
    /// `box_downsample_to_rgb8_fused`) where the full-frame `apply_boost` would
    /// require a second full-resolution pass. `row.len()` is always a multiple of 3.
    fn apply_boost_slice(&self, row: &mut [f32], config: &SaturationBoostConfig);
}

/// Global registry for the saturation plugin
pub static SATURATION_PLUGIN: OnceLock<Box<dyn SaturationPlugin>> = OnceLock::new();

/// Configuration for shadow saturation boost: enhances colour saturation in shadow
/// regions where colour is perceptually lost during non-linear stretching, applied
/// selectively by luminance with a smooth rolloff. Bell-shaped multiplier: no boost
/// at pure black (L=0, avoids amplifying noise floor), maximum at low shadows
/// (L=peak, faint nebula signal), no boost at midtones/highlights (natural colours
/// preserved). Curve: `M = strength × L/peak × (1 - L/upper)²`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SaturationBoostConfig {
    /// Whether saturation boost is enabled
    pub enabled: bool,
    /// Boost strength (0.0 = no boost, 1.0 = maximum boost)
    /// Typical range: 0.2 to 0.8
    pub strength: f32,
    /// Luminance value where boost peaks (0.0-0.5)
    /// Lower values target darker shadows, higher values extend into lower midtones
    pub shadow_peak: f32,
    /// Upper luminance limit where boost fades to zero (0.1-0.6)
    /// Should be greater than shadow_peak
    pub upper_limit: f32,
}

impl Default for SaturationBoostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strength: 0.5,
            shadow_peak: 0.15,
            upper_limit: 0.4,
        }
    }
}

/// Note: Configuration logic and presets are implemented in the Pro version.
/// In Community version, this is used as a data structure only.
impl SaturationBoostConfig {}

#[instrument(skip(frame, config), fields(
    resolution = %format!("{}x{}", frame.width(), frame.height()),
    strength = config.strength,
    shadow_peak = config.shadow_peak
))]
pub fn apply_shadow_saturation_boost(
    frame: &mut Frame,
    config: &SaturationBoostConfig,
) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }

    if let Some(plugin) = crate::license::pro_plugin(&SATURATION_PLUGIN) {
        plugin.apply_boost(frame, config)
    } else {
        Err(StackError::InvalidConfiguration(
            "Shadow Saturation Boost is a Pro feature. Please upgrade to enable this functionality.".into(),
        ))
    }
}
