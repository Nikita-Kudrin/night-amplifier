use crate::render::stretch::ToneMappingAlgorithm;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StretchAggressiveness {
    Low,
    Medium,
    High,
}

impl Default for StretchAggressiveness {
    fn default() -> Self {
        Self::Medium
    }
}

/// Configuration for the automatic stretch factor solver
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AutoStretchConfig {
    pub target_background: f32,
    pub black_point_sigma: f32,
    pub min_stretch: f32,
    pub max_stretch: f32,
    pub tolerance: f32,
    pub max_iterations: u32,
    pub per_channel_black_point: bool,
    pub tone_mapping: ToneMappingAlgorithm,
}

impl Default for AutoStretchConfig {
    fn default() -> Self {
        Self {
            target_background: 0.10,
            black_point_sigma: 2.8,
            min_stretch: 0.1,
            max_stretch: 100.0,
            tolerance: 0.001,
            max_iterations: 50,
            per_channel_black_point: false,
            tone_mapping: ToneMappingAlgorithm::default(),
        }
    }
}

impl AutoStretchConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_target_background(mut self, target: f32) -> Self {
        self.target_background = target.clamp(0.01, 0.5);
        self
    }

    pub fn with_black_point_sigma(mut self, sigma: f32) -> Self {
        self.black_point_sigma = sigma.clamp(0.5, 5.0);
        self
    }

    pub fn with_min_stretch(mut self, min: f32) -> Self {
        self.min_stretch = min.max(0.01);
        self
    }

    pub fn with_max_stretch(mut self, max: f32) -> Self {
        self.max_stretch = max.max(self.min_stretch);
        self
    }

    pub fn with_per_channel_black_point(mut self, enabled: bool) -> Self {
        self.per_channel_black_point = enabled;
        self
    }

    pub fn with_tone_mapping(mut self, algorithm: ToneMappingAlgorithm) -> Self {
        self.tone_mapping = algorithm;
        self
    }

    pub fn dark_sky() -> Self {
        Self::default()
            .with_target_background(0.10)
            .with_black_point_sigma(2.5)
    }

    pub fn preserve_faint() -> Self {
        Self::default()
            .with_target_background(0.20)
            .with_black_point_sigma(1.5)
    }

    pub fn light_polluted() -> Self {
        Self::default()
            .with_target_background(0.12)
            .with_black_point_sigma(3.0)
    }

    pub fn openlivestacker_style() -> Self {
        Self {
            target_background: 0.08,
            black_point_sigma: 3.0,
            min_stretch: 0.1,
            max_stretch: 100.0,
            tolerance: 0.001,
            max_iterations: 50,
            per_channel_black_point: false,
            tone_mapping: ToneMappingAlgorithm::Asinh,
        }
    }

    pub fn from_profile(is_planetary: bool, aggressiveness: StretchAggressiveness) -> Self {
        if is_planetary {
            Self {
                target_background: 0.05,
                black_point_sigma: 3.0,
                tone_mapping: ToneMappingAlgorithm::Asinh,
                min_stretch: 0.1,
                max_stretch: 2.0,
                ..Default::default()
            }
        } else {
            match aggressiveness {
                StretchAggressiveness::Low => Self {
                    target_background: 0.10,
                    black_point_sigma: 1.5,
                    tone_mapping: ToneMappingAlgorithm::Asinh,
                    min_stretch: 1.0,
                    max_stretch: 20.0,
                    ..Default::default()
                },
                StretchAggressiveness::Medium => Self {
                    target_background: 0.08,
                    black_point_sigma: 1.5,
                    tone_mapping: ToneMappingAlgorithm::Mtf,
                    min_stretch: 0.01,
                    max_stretch: 0.5,
                    ..Default::default()
                },
                StretchAggressiveness::High => Self {
                    target_background: 0.11,
                    black_point_sigma: 2.2,
                    tone_mapping: ToneMappingAlgorithm::Mtf,
                    min_stretch: 0.001,
                    max_stretch: 0.5,
                    ..Default::default()
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autostretch_config_defaults() {
        let config = AutoStretchConfig::default();
        assert!((config.target_background - 0.10).abs() < 1e-6);
        assert!((config.black_point_sigma - 2.8).abs() < 1e-6);
        assert!((config.min_stretch - 0.1).abs() < 1e-6);
        assert!((config.max_stretch - 100.0).abs() < 1e-6);
        assert!((config.tolerance - 0.001).abs() < 1e-6);
        assert_eq!(config.max_iterations, 50);
        assert!(!config.per_channel_black_point);
    }

    #[test]
    fn test_autostretch_config_presets() {
        let dark = AutoStretchConfig::dark_sky();
        assert!((dark.target_background - 0.10).abs() < 1e-6);
        assert!((dark.black_point_sigma - 2.5).abs() < 1e-6);

        let faint = AutoStretchConfig::preserve_faint();
        assert!((faint.target_background - 0.20).abs() < 1e-6);
        assert!((faint.black_point_sigma - 1.5).abs() < 1e-6);

        let lp = AutoStretchConfig::light_polluted();
        assert!((lp.target_background - 0.12).abs() < 1e-6);
        assert!((lp.black_point_sigma - 3.0).abs() < 1e-6);

        let ols = AutoStretchConfig::openlivestacker_style();
        assert!((ols.target_background - 0.08).abs() < 1e-6);
        assert!((ols.black_point_sigma - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_autostretch_config_builder() {
        let config = AutoStretchConfig::new()
            .with_target_background(0.20)
            .with_black_point_sigma(1.5)
            .with_min_stretch(0.5)
            .with_max_stretch(50.0)
            .with_per_channel_black_point(true);

        assert!((config.target_background - 0.20).abs() < 1e-6);
        assert!((config.black_point_sigma - 1.5).abs() < 1e-6);
        assert!((config.min_stretch - 0.5).abs() < 1e-6);
        assert!((config.max_stretch - 50.0).abs() < 1e-6);
        assert!(config.per_channel_black_point);
    }

    #[test]
    fn test_autostretch_config_clamping() {
        let config = AutoStretchConfig::new().with_target_background(0.0);
        assert!((config.target_background - 0.01).abs() < 1e-6);

        let config = AutoStretchConfig::new().with_target_background(1.0);
        assert!((config.target_background - 0.5).abs() < 1e-6);

        let config = AutoStretchConfig::new().with_black_point_sigma(0.5);
        assert!((config.black_point_sigma - 0.5).abs() < 1e-6);

        let config = AutoStretchConfig::new().with_black_point_sigma(10.0);
        assert!((config.black_point_sigma - 5.0).abs() < 1e-6);
    }
}
