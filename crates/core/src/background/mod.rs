//! Background extraction and light pollution subtraction
//!
//! This module estimates and removes the sky background gradient (light pollution, moon,
//! horizon glow, vignetting):
//! 1. Lays sample nodes over the frame, both edges and an inset line included
//! 2. Takes a star-rejected median per node via iterative sigma clipping
//! 3. Classifies nodes as sky or target ([`classify`]): robust polynomial ladder, glow
//!    readmitted by where it peaks, catalog objects ([`FieldProtection`]) kept out
//! 4. Interpolates the completed grid (monotone cubic here, thin-plate spline in Pro)
//! 5. Adds what the grid could not follow along the frame's edges ([`EdgeProfiles`])
//! 6. Subtracts it from the image

pub mod classify;
mod config;
mod edges;
mod extractor;
pub mod grid;
mod model;
mod pchip;
mod poly;
pub mod protection;

pub use classify::{classify, Classified, NodeLayout};
pub use config::{BackgroundConfig, BackgroundExtractionAlgorithm};
pub use edges::EdgeProfiles;
pub use extractor::BackgroundExtractor;
pub use grid::{
    compute_box_size, extract_node_value, layout_nodes, mad, mad_with_scratch, median, node_axis,
    sample_nodes, GridNode,
};
pub use model::BackgroundModel;
pub use protection::{FieldProtection, ProtectedRegion};

use crate::error::Result;
use crate::frame::Frame;

/// Plugin trait for advanced background algorithms (implemented in Pro version)
pub trait BackgroundAlgorithmPlugin: Send + Sync {
    fn estimate_rbf(&self, frame: &Frame, config: &BackgroundConfig) -> Result<BackgroundModel>;
}

/// Convenience function for background subtraction
pub fn subtract_background(frame: &mut Frame) -> Result<()> {
    BackgroundExtractor::with_defaults().subtract(frame)
}

/// Convenience function for background subtraction with custom config
pub fn subtract_background_with_config(frame: &mut Frame, config: BackgroundConfig) -> Result<()> {
    BackgroundExtractor::new(config).subtract(frame)
}

#[cfg(test)]
mod tests;
