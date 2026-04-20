use super::rejection::RejectionMethod;
use crate::camera::DualSamplingMode;
use serde::{Deserialize, Serialize};

/// Available stacking types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StackingType {
    /// Deep Sky Object stacking (star-based registration, sigma clipping)
    #[default]
    DeepSky,
    /// Planetary stacking (correlation-based registration, quality selection)
    Planetary,
    /// Comet stacking (ROI centroid-based alignment, aggressive sigma clipping)
    Comet,
}

impl StackingType {
    /// Returns all available stacking types
    pub fn all() -> &'static [StackingType] {
        &[
            StackingType::DeepSky,
            StackingType::Planetary,
            StackingType::Comet,
        ]
    }

    /// Returns the display name for this stacking type
    pub fn display_name(&self) -> &'static str {
        match self {
            StackingType::DeepSky => "Deep Sky",
            StackingType::Planetary => "Planetary",
            StackingType::Comet => "Comet",
        }
    }

    /// Returns a description of this stacking type
    pub fn description(&self) -> &'static str {
        match self {
            StackingType::DeepSky => "Star-based registration with sigma clipping for nebulae, galaxies, and star clusters",
            StackingType::Planetary => "Correlation-based alignment with quality selection for Moon, planets, and Sun",
            StackingType::Comet => "ROI centroid alignment with aggressive rejection for comets (stars rejected as outliers)",
        }
    }

    /// Whether this stacking type uses star-based frame registration
    pub fn uses_star_registration(&self) -> bool {
        match self {
            StackingType::DeepSky => true,
            StackingType::Planetary => false,
            StackingType::Comet => false,
        }
    }

    /// Whether this stacking type supports live stacking mode
    pub fn supports_stacking(&self) -> bool {
        match self {
            StackingType::DeepSky => true,
            StackingType::Planetary => true,
            StackingType::Comet => true,
        }
    }

    /// Whether this stacking type supports quality-based frame weighting
    pub fn supports_quality_weighting(&self) -> bool {
        match self {
            StackingType::DeepSky => true,
            StackingType::Planetary => true,
            StackingType::Comet => true,
        }
    }

    /// Whether this stacking type needs aggressive stretch for preview
    pub fn uses_aggressive_stretch(&self) -> bool {
        match self {
            StackingType::DeepSky => true,
            StackingType::Planetary => false,
            StackingType::Comet => true,
        }
    }

    /// Preferred sensor mode for cameras that advertise dual sampling.
    /// Deep-sky and comet imaging benefit from lower read noise; planetary
    /// imaging prefers the higher frame rate of the normal mode.
    pub fn desired_sensor_mode(&self) -> DualSamplingMode {
        match self {
            StackingType::DeepSky | StackingType::Comet => DualSamplingMode::LowReadoutNoise,
            StackingType::Planetary => DualSamplingMode::Normal,
        }
    }

    /// Returns comprehensive info about this stacking type for API responses
    pub fn info(&self) -> StackingTypeInfo {
        StackingTypeInfo {
            id: *self,
            name: self.display_name().to_string(),
            description: self.description().to_string(),
            uses_star_registration: self.uses_star_registration(),
            supports_stacking: self.supports_stacking(),
            supports_quality_weighting: self.supports_quality_weighting(),
        }
    }
}

/// Comprehensive information about a stacking type
#[derive(Debug, Clone, Serialize)]
pub struct StackingTypeInfo {
    /// The stacking type identifier
    pub id: StackingType,
    /// Human-readable display name
    pub name: String,
    /// Description of when to use this type
    pub description: String,
    /// Whether this type uses star-based registration
    pub uses_star_registration: bool,
    /// Whether live stacking is supported
    pub supports_stacking: bool,
    /// Whether quality-based frame weighting is supported
    pub supports_quality_weighting: bool,
}

/// Weighting preset for quality-based frame weighting during stacking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WeightingPreset {
    /// Equal weight for all frames (disabled)
    Disabled,
    /// Balanced FWHM and SNR weighting
    #[default]
    Balanced,
    /// Prioritize FWHM (sharpness) for galaxies
    Galaxies,
    /// Prioritize SNR for nebulae
    Nebulae,
    /// Only use FWHM weighting
    FwhmOnly,
    /// Only use SNR weighting
    SnrOnly,
}

impl WeightingPreset {
    /// Returns all available weighting presets
    pub fn all() -> &'static [WeightingPreset] {
        &[
            WeightingPreset::Disabled,
            WeightingPreset::Balanced,
            WeightingPreset::Galaxies,
            WeightingPreset::Nebulae,
            WeightingPreset::FwhmOnly,
            WeightingPreset::SnrOnly,
        ]
    }

    /// Returns the display name for this preset
    pub fn display_name(&self) -> &'static str {
        match self {
            WeightingPreset::Disabled => "Disabled",
            WeightingPreset::Balanced => "Balanced",
            WeightingPreset::Galaxies => "Galaxies",
            WeightingPreset::Nebulae => "Nebulae",
            WeightingPreset::FwhmOnly => "FWHM Only",
            WeightingPreset::SnrOnly => "SNR Only",
        }
    }

    /// Returns a description of this preset
    pub fn description(&self) -> &'static str {
        match self {
            WeightingPreset::Disabled => "Equal weight for all frames",
            WeightingPreset::Balanced => "Balanced FWHM and SNR weighting",
            WeightingPreset::Galaxies => "Prioritizes sharpness (FWHM) for fine details",
            WeightingPreset::Nebulae => "Prioritizes SNR for smoother backgrounds",
            WeightingPreset::FwhmOnly => "Only weights by sharpness (FWHM)",
            WeightingPreset::SnrOnly => "Only weights by signal-to-noise ratio",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameQuality {
    /// Full Width at Half Maximum of stars in pixels.
    /// Lower values indicate sharper stars (better seeing).
    /// Typical range: 1.5 - 8.0 pixels.
    pub fwhm: Option<f32>,

    /// Signal-to-Noise Ratio of the frame.
    /// Higher values indicate cleaner signal.
    /// Typically the median SNR of detected stars.
    pub snr: Option<f32>,
}

impl Default for FrameQuality {
    fn default() -> Self {
        Self {
            fwhm: None,
            snr: None,
        }
    }
}

impl FrameQuality {
    /// Creates quality metrics with both FWHM and SNR.
    pub fn new(fwhm: f32, snr: f32) -> Self {
        Self {
            fwhm: Some(fwhm),
            snr: Some(snr),
        }
    }

    /// Creates quality metrics with only FWHM (sharpness).
    pub fn from_fwhm(fwhm: f32) -> Self {
        Self {
            fwhm: Some(fwhm),
            snr: None,
        }
    }

    /// Creates quality metrics with only SNR.
    pub fn from_snr(snr: f32) -> Self {
        Self {
            fwhm: None,
            snr: Some(snr),
        }
    }

    /// Returns true if any quality metric is available.
    pub fn has_metrics(&self) -> bool {
        self.fwhm.is_some() || self.snr.is_some()
    }
}

/// Configuration for quality-based frame weighting during stacking.
///
/// Allows balancing between FWHM (sharpness) and SNR (noise) weighting.
/// The final weight is: `weight = fwhm_weight * fwhm_score + snr_weight * snr_score`
/// where scores are normalized to [0, 1] across all frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightingConfig {
    /// Weight given to FWHM-based scoring (0.0 to 1.0).
    /// Higher values prioritize sharper frames.
    pub fwhm_weight: f32,

    /// Weight given to SNR-based scoring (0.0 to 1.0).
    /// Higher values prioritize lower-noise frames.
    pub snr_weight: f32,

    /// Minimum weight applied to any frame (prevents complete rejection).
    /// Default: 0.1 (worst frame still contributes 10% of best frame).
    pub min_weight: f32,

    /// Power exponent for weight scaling.
    /// Higher values increase contrast between good and bad frames.
    /// Default: 1.0 (linear), 2.0 = quadratic emphasis on quality.
    pub power: f32,
}

impl Default for WeightingConfig {
    fn default() -> Self {
        Self {
            fwhm_weight: 0.5,
            snr_weight: 0.5,
            min_weight: 0.1,
            power: 1.0,
        }
    }
}

impl WeightingConfig {
    /// Creates equal weighting between FWHM and SNR (balanced).
    pub fn balanced() -> Self {
        Self::default()
    }

    /// Creates weighting optimized for galaxies (prioritizes sharpness/FWHM).
    pub fn for_galaxies() -> Self {
        Self {
            fwhm_weight: 0.8,
            snr_weight: 0.2,
            min_weight: 0.1,
            power: 1.5,
        }
    }

    /// Creates weighting optimized for nebulae (prioritizes SNR).
    pub fn for_nebulae() -> Self {
        Self {
            fwhm_weight: 0.2,
            snr_weight: 0.8,
            min_weight: 0.1,
            power: 1.0,
        }
    }

    /// Creates weighting using only FWHM (maximum sharpness).
    pub fn fwhm_only() -> Self {
        Self {
            fwhm_weight: 1.0,
            snr_weight: 0.0,
            min_weight: 0.1,
            power: 1.0,
        }
    }

    /// Creates weighting using only SNR (maximum smoothness).
    pub fn snr_only() -> Self {
        Self {
            fwhm_weight: 0.0,
            snr_weight: 1.0,
            min_weight: 0.1,
            power: 1.0,
        }
    }

    /// Disables weighting (all frames equal weight).
    pub fn disabled() -> Self {
        Self {
            fwhm_weight: 0.0,
            snr_weight: 0.0,
            min_weight: 1.0,
            power: 1.0,
        }
    }

    /// Sets the FWHM weight.
    pub fn with_fwhm_weight(mut self, weight: f32) -> Self {
        self.fwhm_weight = weight.clamp(0.0, 1.0);
        self
    }

    /// Sets the SNR weight.
    pub fn with_snr_weight(mut self, weight: f32) -> Self {
        self.snr_weight = weight.clamp(0.0, 1.0);
        self
    }

    /// Sets the minimum weight floor.
    pub fn with_min_weight(mut self, min: f32) -> Self {
        self.min_weight = min.clamp(0.0, 1.0);
        self
    }

    /// Sets the power exponent for weight scaling.
    pub fn with_power(mut self, power: f32) -> Self {
        self.power = power.max(0.1);
        self
    }

    /// Returns true if weighting is effectively disabled.
    pub fn is_disabled(&self) -> bool {
        (self.fwhm_weight == 0.0 && self.snr_weight == 0.0) || self.min_weight >= 1.0
    }
}

/// Configuration for the stacking algorithm.
#[derive(Debug, Clone)]
pub struct StackingConfig {
    /// Rejection method to use
    pub rejection: RejectionMethod,
    /// Number of sigma for clipping (default: 2.5)
    pub sigma_low: f32,
    /// Number of sigma for clipping high values (default: 2.5)
    pub sigma_high: f32,
    /// Maximum iterations for sigma clipping (default: 3)
    pub max_iterations: usize,
    /// Minimum number of frames required for rejection to activate
    pub min_frames_for_rejection: usize,
    /// Quality-based frame weighting configuration
    pub weighting: WeightingConfig,
}

impl Default for StackingConfig {
    fn default() -> Self {
        Self {
            rejection: RejectionMethod::None,
            sigma_low: 2.5,
            sigma_high: 2.5,
            max_iterations: 3,
            min_frames_for_rejection: 3,
            weighting: WeightingConfig::balanced(),
        }
    }
}

impl StackingConfig {
    /// Creates a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the rejection method.
    pub fn with_rejection(mut self, method: RejectionMethod) -> Self {
        self.rejection = method;
        self
    }

    /// Sets symmetric sigma thresholds.
    pub fn with_sigma(mut self, sigma: f32) -> Self {
        self.sigma_low = sigma;
        self.sigma_high = sigma;
        self
    }

    /// Sets asymmetric sigma thresholds.
    pub fn with_sigma_asymmetric(mut self, low: f32, high: f32) -> Self {
        self.sigma_low = low;
        self.sigma_high = high;
        self
    }

    /// Sets maximum clipping iterations.
    pub fn with_max_iterations(mut self, iterations: usize) -> Self {
        self.max_iterations = iterations;
        self
    }

    /// Sets the quality-based weighting configuration.
    pub fn with_weighting(mut self, weighting: WeightingConfig) -> Self {
        self.weighting = weighting;
        self
    }

    /// Enables quality-based weighting with balanced FWHM/SNR weights.
    pub fn with_quality_weighting(mut self) -> Self {
        self.weighting = WeightingConfig::balanced();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_quality_creation() {
        let q = FrameQuality::new(2.5, 15.0);
        assert_eq!(q.fwhm, Some(2.5));
        assert_eq!(q.snr, Some(15.0));
        assert!(q.has_metrics());

        let q_fwhm = FrameQuality::from_fwhm(3.0);
        assert_eq!(q_fwhm.fwhm, Some(3.0));
        assert_eq!(q_fwhm.snr, None);
        assert!(q_fwhm.has_metrics());

        let q_snr = FrameQuality::from_snr(20.0);
        assert_eq!(q_snr.fwhm, None);
        assert_eq!(q_snr.snr, Some(20.0));
        assert!(q_snr.has_metrics());

        let q_empty = FrameQuality::default();
        assert!(!q_empty.has_metrics());
    }

    #[test]
    fn test_weighting_config_presets() {
        let balanced = WeightingConfig::balanced();
        assert_eq!(balanced.fwhm_weight, 0.5);
        assert_eq!(balanced.snr_weight, 0.5);
        assert!(!balanced.is_disabled());

        let galaxies = WeightingConfig::for_galaxies();
        assert!(galaxies.fwhm_weight > galaxies.snr_weight);

        let nebulae = WeightingConfig::for_nebulae();
        assert!(nebulae.snr_weight > nebulae.fwhm_weight);

        let disabled = WeightingConfig::disabled();
        assert!(disabled.is_disabled());
    }

    #[test]
    fn test_stacking_config_with_weighting() {
        let config = StackingConfig::default()
            .with_weighting(WeightingConfig::for_galaxies())
            .with_sigma(2.0);

        assert_eq!(config.weighting.fwhm_weight, 0.8);
        assert_eq!(config.sigma_low, 2.0);
    }

    #[test]
    fn desired_sensor_mode_matches_stacking_type() {
        assert_eq!(
            StackingType::DeepSky.desired_sensor_mode(),
            DualSamplingMode::LowReadoutNoise
        );
        assert_eq!(
            StackingType::Comet.desired_sensor_mode(),
            DualSamplingMode::LowReadoutNoise
        );
        assert_eq!(
            StackingType::Planetary.desired_sensor_mode(),
            DualSamplingMode::Normal
        );
    }
}
