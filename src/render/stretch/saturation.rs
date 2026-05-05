use crate::error::{Result, StackError};
use crate::frame::Frame;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::instrument;

/// Plugin trait for shadow saturation boost (Commercial feature)
pub trait SaturationPlugin: Send + Sync {
    /// Apply saturation boost to an RGB frame
    fn apply_boost(&self, frame: &mut Frame, config: &SaturationBoostConfig) -> Result<()>;
}

/// Global registry for the saturation plugin
pub static SATURATION_PLUGIN: OnceLock<Box<dyn SaturationPlugin>> = OnceLock::new();

/// Configuration for shadow saturation boost
///
/// This feature enhances color saturation in shadow regions where color is
/// perceptually lost during non-linear stretching. The boost is applied
/// selectively based on luminance, with a smooth rolloff to preserve
/// natural midtone colors and avoid amplifying noise in the darkest areas.
///
/// # How It Works
///
/// The saturation multiplier follows a bell-shaped curve:
/// - Pure black (L=0): No boost (avoids amplifying noise floor)
/// - Low shadows (L=peak): Maximum boost (faint nebula signal)
/// - Midtones and highlights: No boost (natural colors preserved)
///
/// The curve shape is: `M = strength × L/peak × (1 - L/upper)²`
/// This creates smooth transitions without harsh color banding.
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
