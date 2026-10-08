//! Configuration for image registration.
//!
//! Provides configuration presets for different imaging conditions (wide-field,
//! narrow-field, challenging conditions, etc.).

/// Configuration for image registration.
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// Maximum number of stars to use for matching (default: 50).
    pub max_stars: usize,
    /// Minimum number of triangles for matching (default: 3).
    pub min_triangles: usize,
    /// Descriptor matching tolerance (default: 0.02).
    pub descriptor_tolerance: f32,
    /// Maximum residual error for a valid match in pixels (default: 3.0).
    pub max_residual: f32,
    /// Minimum number of matched star pairs for valid registration (default: 4).
    pub min_matches: usize,
    /// Maximum triangle side length to consider (default: 500 pixels).
    pub max_triangle_side: f32,
    /// Minimum triangle side length to consider (default: 5 pixels).
    pub min_triangle_side: f32,
    /// Enable RANSAC for robust transform estimation (default: true).
    pub use_ransac: bool,
    /// RANSAC iterations (default: 100).
    pub ransac_iterations: usize,
    /// RANSAC inlier threshold in pixels (default: 3.0).
    pub ransac_threshold: f32,
}

impl Default for RegistrationConfig {
    fn default() -> Self {
        Self {
            max_stars: 50,
            min_triangles: 3,
            descriptor_tolerance: 0.05,
            max_residual: 8.0,
            min_matches: 4,
            max_triangle_side: 600.0,
            min_triangle_side: 10.0,
            use_ransac: true,
            ransac_iterations: 150,
            ransac_threshold: 6.0,
        }
    }
}

impl RegistrationConfig {
    /// Creates a new configuration with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum number of stars to use.
    pub fn with_max_stars(mut self, max: usize) -> Self {
        self.max_stars = max;
        self
    }

    /// Sets the descriptor matching tolerance.
    pub fn with_tolerance(mut self, tol: f32) -> Self {
        self.descriptor_tolerance = tol;
        self
    }

    /// Sets the maximum residual error.
    pub fn with_max_residual(mut self, max: f32) -> Self {
        self.max_residual = max;
        self
    }

    /// Enables or disables RANSAC.
    pub fn with_ransac(mut self, enabled: bool) -> Self {
        self.use_ransac = enabled;
        self
    }

    /// Preset for wide-field images (large FOV, spread-out stars).
    pub fn wide_field() -> Self {
        Self {
            max_stars: 30,
            min_triangles: 3,
            descriptor_tolerance: 0.04,
            max_residual: 8.0,
            min_matches: 3,
            max_triangle_side: 800.0,
            min_triangle_side: 20.0,
            use_ransac: true,
            ransac_iterations: 50,
            ransac_threshold: 6.0,
        }
    }

    /// Preset for narrow-field images (small FOV, dense stars).
    pub fn narrow_field() -> Self {
        Self {
            max_stars: 40,
            min_triangles: 3,
            descriptor_tolerance: 0.02,
            max_residual: 3.0,
            min_matches: 3,
            max_triangle_side: 300.0,
            min_triangle_side: 3.0,
            use_ransac: true,
            ransac_iterations: 50,
            ransac_threshold: 3.0,
        }
    }

    /// Preset for challenging conditions (clouds, satellites, etc.).
    pub fn robust() -> Self {
        Self {
            max_stars: 25,
            min_triangles: 2,
            descriptor_tolerance: 0.05,
            max_residual: 10.0,
            min_matches: 3,
            max_triangle_side: 600.0,
            min_triangle_side: 5.0,
            use_ransac: true,
            ransac_iterations: 80,
            ransac_threshold: 8.0,
        }
    }

    /// Preset for very relaxed matching (last resort).
    pub fn permissive() -> Self {
        Self {
            max_stars: 20,
            min_triangles: 2,
            descriptor_tolerance: 0.08,
            max_residual: 15.0,
            min_matches: 3,
            max_triangle_side: 1000.0,
            min_triangle_side: 3.0,
            use_ransac: false,
            ransac_iterations: 30,
            ransac_threshold: 10.0,
        }
    }

    /// Fast preset - prioritizes speed over robustness.
    pub fn fast() -> Self {
        Self {
            max_stars: 15,
            min_triangles: 2,
            descriptor_tolerance: 0.05,
            max_residual: 10.0,
            min_matches: 3,
            max_triangle_side: 600.0,
            min_triangle_side: 5.0,
            use_ransac: false,
            ransac_iterations: 0,
            ransac_threshold: 8.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_presets() {
        let configs = [
            RegistrationConfig::default(),
            RegistrationConfig::wide_field(),
            RegistrationConfig::narrow_field(),
            RegistrationConfig::robust(),
            RegistrationConfig::permissive(),
            RegistrationConfig::fast(),
        ];

        for config in configs {
            assert!(config.max_stars > 0);
            assert!(config.min_triangle_side < config.max_triangle_side);
            assert!(config.descriptor_tolerance > 0.0);
        }
    }

    #[test]
    fn test_builder_pattern() {
        let config = RegistrationConfig::new()
            .with_max_stars(100)
            .with_tolerance(0.1)
            .with_max_residual(5.0)
            .with_ransac(false);

        assert_eq!(config.max_stars, 100);
        assert!((config.descriptor_tolerance - 0.1).abs() < 1e-6);
        assert!((config.max_residual - 5.0).abs() < 1e-6);
        assert!(!config.use_ransac);
    }
}
