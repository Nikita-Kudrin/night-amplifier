use crate::render::stretch::ToneMappingAlgorithm;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum StretchAggressiveness {
    Low,
    #[default]
    Medium,
    High,
}

/// A frame that is not a stack, which is what every non-live caller renders.
fn default_stack_depth() -> u32 {
    1
}

/// The middle of the Background Grain dial, which is what a caller that has no dial
/// (an export, a test, a one-shot render) should render like.
fn default_grain_split() -> f32 {
    super::logic::DEFAULT_GRAIN_SPLIT
}

/// Configuration for the automatic stretch factor solver
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AutoStretchConfig {
    pub target_background: f32,
    /// Sigmas below the sky to put the black point, *before* the solve scales it.
    ///
    /// `compute_auto_stretch_with_algorithm` adapts it down on a signal-heavy frame
    /// and up with `logic::depth_grain_gain`, so the factor actually applied is not
    /// this number: `with_black_point_sigma` bounds this at 5.0, and the product is
    /// bounded separately. `AutoStretchResult::adaptive_sigma` is what was used.
    pub black_point_sigma: f32,
    pub min_stretch: f32,
    pub max_stretch: f32,
    pub tolerance: f32,
    pub max_iterations: u32,
    pub per_channel_black_point: bool,
    /// Frames in the stack this solve is for, or 1 for a single frame.
    ///
    /// Sets how much of the stack's noise reduction is spent on a calmer sky rather
    /// than on brighter faint signal; see `logic::depth_grain_gain`.
    #[serde(default = "default_stack_depth")]
    pub stack_depth: u32,
    /// Share of the stack's `sqrt(N)` the curve spends on a calmer sky instead of a
    /// brighter target, `0..=0.25`.
    ///
    /// The expensive half of the Background Grain dial: unlike the wavelet's
    /// `star_protection`, which buys sky for almost nothing, this costs rendered target
    /// contrast **1:1**. See `logic::depth_grain_gain`, and `logic::MIN_GRAIN_SPLIT` for
    /// why the dial does not reach zero.
    #[serde(default = "default_grain_split")]
    pub grain_split: f32,
    pub tone_mapping: ToneMappingAlgorithm,
    pub color_intensity: f32,
}

impl Default for AutoStretchConfig {
    fn default() -> Self {
        Self {
            target_background: 0.10,
            black_point_sigma: 2.8,
            min_stretch: 0.1,
            max_stretch: 10000.0,
            tolerance: 0.001,
            max_iterations: 50,
            per_channel_black_point: false,
            stack_depth: default_stack_depth(),
            grain_split: default_grain_split(),
            tone_mapping: ToneMappingAlgorithm::default(),
            color_intensity: 1.0,
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

    /// The stack depth this solve is for; see [`Self::stack_depth`].
    pub fn with_stack_depth(mut self, frames: u32) -> Self {
        self.stack_depth = frames.max(1);
        self
    }

    /// The share of the stack's depth this solve spends on the sky; see
    /// [`Self::grain_split`]. Clamped to the dial's range, since it arrives over JSON.
    pub fn with_grain_split(mut self, split: f32) -> Self {
        self.grain_split = if split.is_finite() {
            split.clamp(0.0, super::logic::MAX_GRAIN_SPLIT)
        } else {
            default_grain_split()
        };
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

    pub fn with_color_intensity(mut self, intensity: f32) -> Self {
        self.color_intensity = intensity;
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
            max_stretch: 10000.0,
            tolerance: 0.001,
            max_iterations: 50,
            per_channel_black_point: false,
            stack_depth: default_stack_depth(),
            grain_split: default_grain_split(),
            tone_mapping: ToneMappingAlgorithm::Asinh,
            color_intensity: 1.0,
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
                // Star Fields, and **asinh on purpose** — a product decision, not a
                // tuning one. This mode exists to show a field of stars and nothing
                // else: nebulosity and galaxy structure are explicitly not its job, so
                // the gentler curve and the brighter sky that come with asinh are the
                // character that is wanted.
                //
                // What it costs, measured on a 3028-frame M27 so the next person does not
                // have to re-derive it: asinh pins the rendered star peak at ~160 output
                // levels however it is tuned (MTF reaches 220), and it shows *fewer* stars
                // than either MTF profile — 2398 per megapixel above sky+20 against Deep
                // Sky's 2777, and 976 above sky+60 against 1721. Raising
                // `target_background` recovers the count but only by lifting the sky with
                // it (0.16 gives 3096 stars and a sky of 40 output levels); bounding
                // `max_stretch` to keep highlights linear makes both worse at once. So
                // these are the best asinh numbers available at a sky of 24, not a local
                // optimum waiting to be improved.
                StretchAggressiveness::Low => Self {
                    target_background: 0.10,
                    black_point_sigma: 1.5,
                    tone_mapping: ToneMappingAlgorithm::Asinh,
                    min_stretch: 1.0,
                    max_stretch: 10000.0,
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
        assert!((config.max_stretch - 10000.0).abs() < 1e-6);
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
