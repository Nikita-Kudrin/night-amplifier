//! Adaptive registration strategies.
//!
//! Provides automatic configuration selection based on imaging conditions and
//! hint-based optimization for challenging scenarios.

use tracing::{field, instrument, Span};

use crate::detection::Star;
use crate::error::{Result, StackError};

use super::config::RegistrationConfig;
use super::engine::ImageRegistration;
use super::transform::AffineTransform;

/// Result of adaptive registration including diagnostics.
#[derive(Debug, Clone)]
pub struct AdaptiveRegistrationResult {
    /// The computed transform.
    pub transform: AffineTransform,
    /// Number of matched stars.
    pub matched_stars: usize,
    /// Mean residual error.
    pub mean_residual: f32,
    /// Configuration preset that succeeded.
    pub config_used: String,
    /// Number of attempts before success.
    pub attempts: usize,
}

/// Adaptive registration that tries multiple strategies.
pub struct AdaptiveRegistration {
    configs: Vec<(String, RegistrationConfig)>,
}

impl Default for AdaptiveRegistration {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveRegistration {
    /// Creates a new adaptive registration with fast configuration progression.
    /// Only tries 2 configs for speed - default and robust fallback.
    pub fn new() -> Self {
        Self {
            configs: vec![
                ("fast".to_string(), RegistrationConfig::fast()),
                ("robust".to_string(), RegistrationConfig::robust()),
            ],
        }
    }

    /// Creates adaptive registration with full config set (slower but more thorough).
    pub fn thorough() -> Self {
        Self {
            configs: vec![
                ("default".to_string(), RegistrationConfig::default()),
                ("wide_field".to_string(), RegistrationConfig::wide_field()),
                (
                    "narrow_field".to_string(),
                    RegistrationConfig::narrow_field(),
                ),
                ("robust".to_string(), RegistrationConfig::robust()),
                ("permissive".to_string(), RegistrationConfig::permissive()),
            ],
        }
    }

    /// Adds a custom configuration to try.
    pub fn with_config(mut self, name: &str, config: RegistrationConfig) -> Self {
        self.configs.push((name.to_string(), config));
        self
    }

    /// Registers using adaptive strategy - tries multiple configurations until one works.
    #[instrument(skip(self, ref_stars, tgt_stars), fields(
        ref_count = ref_stars.len(),
        target_count = tgt_stars.len(),
        config_used = field::Empty,
        matched_stars = field::Empty,
        mean_residual = field::Empty,
        attempts = field::Empty,
    ))]
    pub fn register(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
    ) -> Result<AdaptiveRegistrationResult> {
        let mut last_error = String::new();

        for (attempt, (name, config)) in self.configs.iter().enumerate() {
            let registration = ImageRegistration::new(config.clone());

            match registration.register(ref_stars, tgt_stars) {
                Ok(transform) => {
                    let (matched_stars, mean_residual) = self.compute_diagnostics(
                        ref_stars,
                        tgt_stars,
                        &transform,
                        config.max_residual * 2.0,
                    );

                    let span = Span::current();
                    span.record("config_used", name.as_str());
                    span.record("matched_stars", matched_stars);
                    span.record("mean_residual", mean_residual);
                    span.record("attempts", attempt + 1);

                    return Ok(AdaptiveRegistrationResult {
                        transform,
                        matched_stars,
                        mean_residual,
                        config_used: name.clone(),
                        attempts: attempt + 1,
                    });
                }
                Err(e) => {
                    last_error = format!("{}: {}", name, e);
                }
            }
        }

        Err(StackError::Registration(format!(
            "All registration strategies failed. Last error: {}",
            last_error
        )))
    }

    /// Registers with hints about the expected image characteristics.
    pub fn register_with_hints(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        hints: &RegistrationHints,
    ) -> Result<AdaptiveRegistrationResult> {
        let mut prioritized_configs = Vec::new();

        let hint_config = hints.to_config();
        prioritized_configs.push(("hint_based".to_string(), hint_config));
        prioritized_configs.extend(self.configs.clone());

        let adaptive = AdaptiveRegistration {
            configs: prioritized_configs,
        };
        adaptive.register(ref_stars, tgt_stars)
    }

    fn compute_diagnostics(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        transform: &AffineTransform,
        threshold: f32,
    ) -> (usize, f32) {
        let correspondences =
            get_correspondences_for_transform(ref_stars, tgt_stars, transform, threshold);

        if correspondences.is_empty() {
            return (0, 0.0);
        }

        let mean_residual = correspondences
            .iter()
            .map(|&(ri, ti)| transform.residual(&tgt_stars[ti], &ref_stars[ri]))
            .sum::<f32>()
            / correspondences.len() as f32;

        (correspondences.len(), mean_residual)
    }
}

/// Hints about image characteristics to guide registration.
#[derive(Debug, Clone, Default)]
pub struct RegistrationHints {
    /// Expected field of view type.
    pub fov_type: FovType,
    /// Whether clouds or obstructions might be present.
    pub has_obstructions: bool,
    /// Whether significant field rotation is expected.
    pub has_rotation: bool,
    /// Whether satellite trails might be present.
    pub has_satellites: bool,
    /// Expected brightness variation.
    pub brightness_variation: BrightnessVariation,
}

impl RegistrationHints {
    /// Creates default hints.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets FOV type hint.
    pub fn with_fov(mut self, fov: FovType) -> Self {
        self.fov_type = fov;
        self
    }

    /// Sets obstruction hint.
    pub fn with_obstructions(mut self, has: bool) -> Self {
        self.has_obstructions = has;
        self
    }

    /// Sets rotation hint.
    pub fn with_rotation(mut self, has: bool) -> Self {
        self.has_rotation = has;
        self
    }

    /// Converts hints to a registration configuration.
    pub(crate) fn to_config(&self) -> RegistrationConfig {
        let mut config = match self.fov_type {
            FovType::Wide => RegistrationConfig::wide_field(),
            FovType::Standard => RegistrationConfig::default(),
            FovType::Narrow => RegistrationConfig::narrow_field(),
        };

        if self.has_obstructions || self.has_satellites {
            config.use_ransac = true;
            config.ransac_iterations = 200;
            config.ransac_threshold *= 1.5;
            config.descriptor_tolerance *= 1.5;
        }

        match self.brightness_variation {
            BrightnessVariation::Stable => {}
            BrightnessVariation::Moderate => {
                config.max_residual *= 1.5;
            }
            BrightnessVariation::High => {
                config.max_residual *= 2.0;
                config.min_matches = 3.max(config.min_matches - 1);
            }
        }

        config
    }
}

/// Field of view type hint.
#[derive(Debug, Clone, Copy, Default)]
pub enum FovType {
    /// Wide field (> 2 degrees).
    Wide,
    /// Standard field.
    #[default]
    Standard,
    /// Narrow field (< 0.5 degrees).
    Narrow,
}

/// Brightness variation hint.
#[derive(Debug, Clone, Copy, Default)]
pub enum BrightnessVariation {
    /// Stable brightness.
    #[default]
    Stable,
    /// Some variation (thin clouds, etc.).
    Moderate,
    /// High variation (clouds rolling, etc.).
    High,
}

/// Gets correspondences from a transform (for diagnostics).
fn get_correspondences_for_transform(
    ref_stars: &[Star],
    tgt_stars: &[Star],
    transform: &AffineTransform,
    threshold: f32,
) -> Vec<(usize, usize)> {
    let mut correspondences = Vec::new();
    let mut tgt_used = vec![false; tgt_stars.len()];

    for (ri, ref_star) in ref_stars.iter().enumerate() {
        let mut best_dist = f32::MAX;
        let mut best_ti = 0;

        for (ti, tgt_star) in tgt_stars.iter().enumerate() {
            if tgt_used[ti] {
                continue;
            }
            let dist = transform.residual(tgt_star, ref_star);
            if dist < best_dist {
                best_dist = dist;
                best_ti = ti;
            }
        }

        if best_dist < threshold {
            correspondences.push((ri, best_ti));
            tgt_used[best_ti] = true;
        }
    }

    correspondences
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_stars() -> Vec<Star> {
        vec![
            Star::new(100.0, 100.0, 1000.0, 0.9, 50.0),
            Star::new(200.0, 100.0, 900.0, 0.85, 45.0),
            Star::new(150.0, 200.0, 800.0, 0.8, 40.0),
            Star::new(250.0, 180.0, 700.0, 0.75, 35.0),
            Star::new(120.0, 280.0, 600.0, 0.7, 30.0),
        ]
    }

    #[test]
    fn test_adaptive_registration() {
        let ref_stars = create_test_stars();
        let tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| Star::new(s.x + 5.0, s.y - 3.0, s.flux, s.peak, s.snr))
            .collect();

        let adaptive = AdaptiveRegistration::new();
        let result = adaptive.register(&ref_stars, &tgt_stars).unwrap();

        assert!(result.matched_stars >= 3);
        assert!(result.mean_residual < 5.0);
    }

    #[test]
    fn test_hints_config_generation() {
        let hints = RegistrationHints::new()
            .with_fov(FovType::Wide)
            .with_obstructions(true);

        let config = hints.to_config();

        assert!(config.use_ransac);
        assert_eq!(config.ransac_iterations, 200);
    }
}
