use std::sync::Arc;

use super::protection::FieldProtection;
use serde::{Deserialize, Serialize};

/// Algorithm used for background extraction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub enum BackgroundExtractionAlgorithm {
    /// Grid nodes joined by monotone cubic interpolation (fast, stable; the wire name keeps
    /// the bilinear it replaced)
    #[default]
    GridBilinear,
    /// Radial Basis Function with Thin-Plate Splines (high quality, nebula-safe)
    Rbf,
}

impl BackgroundExtractionAlgorithm {
    /// What a session asks for by default: RBF in a build that ships the background plugin,
    /// Grid in Community. Keyed on the build, not the licence, as
    /// [`crate::stacking::RejectionMethod::best_available`] is: a frame without a licensed
    /// plugin runs Grid (the session's `resolve_background_algorithm`), the choice kept.
    pub fn best_available() -> Self {
        if crate::plugins::Plugins::installed().ships_background() {
            Self::Rbf
        } else {
            Self::GridBilinear
        }
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

/// Configuration for background extraction.
///
/// One config for every stretch mode: the extractor's own target masking protects the
/// target, so the strength is a full 1.0. The 0.7 the stretch profiles used to set left
/// 30 % of every gradient on screen (M57: 4.7 sigma of sky range against 0.9 at 1.0).
#[derive(Debug, Clone)]
pub struct BackgroundConfig {
    /// Algorithm to use for background extraction
    pub algorithm: BackgroundExtractionAlgorithm,
    /// Node lines across, both edges included (see `grid::node_axis`)
    pub grid_width: usize,
    /// Node lines down, both edges included
    pub grid_height: usize,
    /// Extra node line this far in from each edge, as a frame fraction (0: none)
    pub edge_inset: f32,
    /// Sigma threshold for star rejection (pixels above median + sigma*MAD are rejected)
    pub star_rejection_sigma: f32,
    /// If true, subtract only the gradient (variation from reference level), preserving the base signal.
    /// If false, subtract the full background (traditional behavior).
    pub gradient_only: bool,
    /// Percentile of the model taken as the reference level in gradient-only mode (0.0 to 1.0).
    pub reference_percentile: f32,
    /// Share of the estimated gradient to subtract (0.0 to 1.0).
    pub aggressiveness: f32,
    /// Catalog objects in this frame, from the session's plate solve, never fitted as sky.
    pub protection: Option<Arc<FieldProtection>>,
    /// Measure what the grid leaves along the frame's edges ([`super::EdgeProfiles`]). Off
    /// for a single sub: its noise buries a band a stack shows, and it re-estimates every frame.
    pub edge_profiles: bool,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            algorithm: BackgroundExtractionAlgorithm::default(),
            grid_width: 12,
            grid_height: 12,
            edge_inset: 0.035,
            star_rejection_sigma: 2.5,
            gradient_only: true,
            reference_percentile: 0.1,
            aggressiveness: 1.0,
            protection: None,
            edge_profiles: true,
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

    /// Set the edge inset line (frame fraction; 0 for none)
    pub fn with_edge_inset(mut self, inset: f32) -> Self {
        self.edge_inset = inset.clamp(0.0, 0.25);
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
    pub fn with_reference_percentile(mut self, percentile: f32) -> Self {
        self.reference_percentile = percentile.clamp(0.0, 1.0);
        self
    }

    /// Set the share of the gradient to subtract (0.0 to 1.0).
    pub fn with_aggressiveness(mut self, aggressiveness: f32) -> Self {
        self.aggressiveness = aggressiveness.clamp(0.0, 1.0);
        self
    }

    /// Catalog objects to keep out of the model.
    pub fn with_protection(mut self, protection: Option<Arc<FieldProtection>>) -> Self {
        self.protection = protection;
        self
    }

    /// Whether to measure the edges' profiles on top of the grid.
    pub fn with_edge_profiles(mut self, edge_profiles: bool) -> Self {
        self.edge_profiles = edge_profiles;
        self
    }
}
