use crate::render::StretchAggressiveness;
use serde::{Deserialize, Serialize};

/// Algorithm used for background extraction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundExtractionAlgorithm {
    /// Grid-based median with bilinear interpolation (fast, stable)
    GridBilinear,
    /// Radial Basis Function with Thin-Plate Splines (high quality, nebula-safe)
    Rbf,
}

impl Default for BackgroundExtractionAlgorithm {
    fn default() -> Self {
        Self::GridBilinear
    }
}

impl std::fmt::Display for BackgroundExtractionAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GridBilinear => write!(f, "Grid-based"),
            Self::Rbf => write!(f, "RBF"),
        }
    }
}

/// Configuration for background extraction
#[derive(Debug, Clone)]
pub struct BackgroundConfig {
    /// Algorithm to use for background extraction
    pub algorithm: BackgroundExtractionAlgorithm,
    /// Number of grid blocks horizontally
    pub grid_width: usize,
    /// Number of grid blocks vertically
    pub grid_height: usize,
    /// Sigma threshold for star rejection (pixels above median + sigma*MAD are rejected)
    pub star_rejection_sigma: f32,
    /// If true, subtract only the gradient (variation from reference level), preserving the base signal.
    /// If false, subtract the full background (traditional behavior).
    /// Default: true (gradient-only mode preserves signal in low-signal astronomical images).
    pub gradient_only: bool,
    /// Percentile to use as reference level in gradient-only mode (0.0 to 1.0).
    /// Lower values preserve more signal but may leave residual gradients.
    /// Higher values remove more gradient but may also remove extended object signal.
    /// Default: 0.1 (10th percentile) - balances gradient removal with signal preservation.
    pub reference_percentile: f32,
    /// Aggressiveness of background subtraction (0.0 to 1.0).
    /// Controls how much of the estimated background gradient to actually subtract.
    /// 0.0 = no subtraction, 1.0 = full subtraction, 0.5 = subtract 50% of estimated gradient.
    /// Default: 1.0 (full subtraction of the gradient).
    /// Use lower values (0.3-0.7) for images with extended objects like nebulae.
    pub aggressiveness: f32,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 16,
            grid_height: 16,
            star_rejection_sigma: 2.5,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 1.0,
        }
    }
}

impl BackgroundConfig {
    /// Preset for images with extended objects like nebulae.
    /// Uses conservative settings to preserve nebula signal.
    pub fn for_nebulae() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 2,
            grid_height: 2,
            star_rejection_sigma: 2.0,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 0.7,
        }
    }

    /// Preset for images with galaxies (medium stretch).
    pub fn for_galaxies() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 2,
            grid_height: 2,
            star_rejection_sigma: 2.0,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 0.7,
        }
    }

    /// Preset for star fields (low stretch, aggressive gradient removal).
    pub fn for_star_field() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 6,
            grid_height: 6,
            star_rejection_sigma: 2.5,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 0.8,
        }
    }

    /// Automatically select the best background extraction profile based on the selected autostretch profile.
    pub fn from_stretch_profile(aggressiveness: StretchAggressiveness) -> Self {
        match aggressiveness {
            StretchAggressiveness::High => Self::for_nebulae(),
            StretchAggressiveness::Medium => Self::for_galaxies(),
            StretchAggressiveness::Low => Self::for_star_field(),
        }
    }

    /// Preset for images with strong light pollution gradients but no extended objects.
    pub fn for_light_pollution() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 16,
            grid_height: 16,
            star_rejection_sigma: 2.5,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 1.0,
        }
    }

    /// Preset that automatically adapts based on background uniformity.
    /// Analyzes the background model and adjusts aggressiveness accordingly.
    pub fn adaptive() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 12,
            grid_height: 12,
            star_rejection_sigma: 2.0,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: -1.0,
        }
    }
}

impl BackgroundConfig {
    /// Set the extraction algorithm
    pub fn with_algorithm(mut self, algorithm: BackgroundExtractionAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Set the grid dimensions
    pub fn with_grid_size(mut self, width: usize, height: usize) -> Self {
        self.grid_width = width;
        self.grid_height = height;
        self
    }

    /// Set the star rejection sigma threshold
    pub fn with_star_rejection(mut self, sigma: f32) -> Self {
        self.star_rejection_sigma = sigma;
        self
    }

    /// Set gradient-only mode
    ///
    /// If true (default), only the gradient is subtracted, preserving the base signal level.
    /// If false, the full background is subtracted (traditional behavior).
    pub fn with_gradient_only(mut self, gradient_only: bool) -> Self {
        self.gradient_only = gradient_only;
        self
    }

    /// Set the reference percentile for gradient-only mode (0.0 to 1.0).
    /// Lower values preserve more signal (e.g., 0.05 for nebulae).
    /// Higher values remove more gradient (e.g., 0.2 for light pollution only).
    pub fn with_reference_percentile(mut self, percentile: f32) -> Self {
        self.reference_percentile = percentile.clamp(0.0, 1.0);
        self
    }

    /// Set the aggressiveness of background subtraction (0.0 to 1.0).
    /// Use lower values (0.3-0.7) for images with extended objects.
    /// Use -1.0 for automatic adaptation based on background uniformity.
    pub fn with_aggressiveness(mut self, aggressiveness: f32) -> Self {
        self.aggressiveness = if aggressiveness < 0.0 {
            -1.0 // Auto mode
        } else {
            aggressiveness.clamp(0.0, 1.0)
        };
        self
    }
}
