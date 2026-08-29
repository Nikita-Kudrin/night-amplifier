use crate::background::BackgroundConfig;
use crate::render::autostretch::AutoStretchConfig;
use crate::render::output::{ContrastConfig, DisplayOutput};
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

    /// Whether to apply SCNR (Subtractive Chromatic Noise Reduction)
    pub scnr: bool,
    /// Amount of SCNR to apply (0.0 to 1.0)
    pub scnr_amount: f32,

    /// Black floor and dithering applied where the frame becomes 8-bit.
    ///
    /// Unlike every other field this one is not a pipeline *stage* — the fused
    /// streaming encoders apply it in the same traversal that writes the output
    /// bytes, because that is the only place the output pixel coordinate the
    /// dither needs actually exists.
    pub display: DisplayOutput,
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
            scnr: false,
            scnr_amount: 1.0,
            display: DisplayOutput::PLAIN,
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

    /// Enable or disable SCNR
    pub fn with_scnr(mut self, enabled: bool) -> Self {
        self.scnr = enabled;
        self
    }

    /// Set the SCNR amount
    pub fn with_scnr_amount(mut self, amount: f32) -> Self {
        self.scnr_amount = amount;
        self.scnr = true;
        self
    }

    /// Set the black floor and dithering used at the 8-bit conversion.
    pub fn with_display(mut self, display: DisplayOutput) -> Self {
        self.display = display;
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
            scnr: true,
            scnr_amount: 1.0,
            display: DisplayOutput::PLAIN,
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
            scnr: false,
            scnr_amount: 1.0,
            display: DisplayOutput::PLAIN,
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
            scnr: false,
            scnr_amount: 1.0,
            display: DisplayOutput::PLAIN,
        }
    }
}
