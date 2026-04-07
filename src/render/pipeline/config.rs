use crate::background::BackgroundConfig;
use crate::render::autostretch::AutoStretchConfig;
use crate::render::output::ContrastConfig;
use crate::render::stretch::SaturationBoostConfig;

/// Configuration for the unified render pipeline
#[derive(Debug, Clone)]
pub struct RenderPipelineConfig {
    /// Whether to apply background subtraction
    pub background_subtraction: bool,
    /// Configuration for background subtraction (if enabled)
    pub background_config: BackgroundConfig,

    /// Whether to apply auto-stretch
    pub auto_stretch: bool,
    /// Configuration for auto-stretch
    pub stretch_config: AutoStretchConfig,

    /// Whether to apply saturation boost
    pub saturation_boost: bool,
    /// Configuration for saturation boost
    pub saturation_config: SaturationBoostConfig,

    /// Whether to apply contrast adjustment
    pub contrast: bool,
    /// Configuration for contrast adjustment
    pub contrast_config: ContrastConfig,
}

impl Default for RenderPipelineConfig {
    fn default() -> Self {
        Self {
            background_subtraction: false,
            background_config: BackgroundConfig::default(),
            auto_stretch: true,
            stretch_config: AutoStretchConfig::default(),
            saturation_boost: false,
            saturation_config: SaturationBoostConfig::default(),
            contrast: true,
            contrast_config: ContrastConfig::default(),
        }
    }
}

impl RenderPipelineConfig {
    /// Create a new pipeline config with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable background subtraction
    pub fn with_background_subtraction(mut self, enabled: bool) -> Self {
        self.background_subtraction = enabled;
        self
    }

    /// Set the background subtraction config
    pub fn with_background_config(mut self, config: BackgroundConfig) -> Self {
        self.background_config = config;
        self.background_subtraction = true;
        self
    }

    /// Enable or disable auto-stretch
    pub fn with_auto_stretch(mut self, enabled: bool) -> Self {
        self.auto_stretch = enabled;
        self
    }

    /// Set the auto-stretch config
    pub fn with_stretch_config(mut self, config: AutoStretchConfig) -> Self {
        self.stretch_config = config;
        self.auto_stretch = true;
        self
    }

    /// Enable or disable saturation boost
    pub fn with_saturation_boost(mut self, enabled: bool) -> Self {
        self.saturation_boost = enabled;
        self
    }

    /// Set the saturation boost config (also enables saturation boost)
    pub fn with_saturation_config(mut self, config: SaturationBoostConfig) -> Self {
        self.saturation_config = config;
        self.saturation_boost = config.enabled;
        self
    }

    /// Enable or disable contrast adjustment
    pub fn with_contrast(mut self, enabled: bool) -> Self {
        self.contrast = enabled;
        self
    }

    /// Set the contrast config
    pub fn with_contrast_config(mut self, config: ContrastConfig) -> Self {
        self.contrast_config = config;
        self.contrast = true;
        self
    }

    /// Preset for deep sky imaging (nebulae, galaxies)
    pub fn deep_sky() -> Self {
        Self {
            background_subtraction: true,
            background_config: BackgroundConfig::default(),
            auto_stretch: true,
            stretch_config: AutoStretchConfig::default(),
            saturation_boost: true,
            saturation_config: SaturationBoostConfig {
                enabled: true,
                strength: 0.5,
                shadow_peak: 0.15,
                upper_limit: 0.4,
            },
            contrast: true,
            contrast_config: ContrastConfig::default(),
        }
    }

    /// Preset for planetary imaging
    pub fn planetary() -> Self {
        Self {
            background_subtraction: false,
            background_config: BackgroundConfig::default(),
            auto_stretch: true,
            stretch_config: AutoStretchConfig::from_profile(true, Default::default()),
            saturation_boost: false,
            saturation_config: SaturationBoostConfig::default(),
            contrast: true,
            contrast_config: ContrastConfig::subtle(),
        }
    }

    /// Preset for preview mode (fast, less aggressive)
    pub fn preview() -> Self {
        Self {
            background_subtraction: false,
            background_config: BackgroundConfig::default(),
            auto_stretch: true,
            stretch_config: AutoStretchConfig::default(),
            saturation_boost: false,
            saturation_config: SaturationBoostConfig::default(),
            contrast: true,
            contrast_config: ContrastConfig::default(),
        }
    }
}
