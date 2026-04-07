//! Planetary Stacking Module
//!
//! Planetary imaging differs significantly from Deep Sky Object (DSO) imaging:
//!
//! | Aspect | DSO | Planetary |
//! |--------|-----|-----------|
//! | Registration | Star triangle matching | Correlation-based (surface features) |
//! | Frame selection | Use all frames | Quality-based (best 10-30%) |
//! | Combination | Mean with sigma clipping | Percentile stacking |
//! | Frame rate | Slow (seconds) | Fast (30-300 fps) |
//! | Alignment | Rotation + translation | Translation only |
//!
//! # Algorithm Overview
//!
//! 1. **Quality Estimation**: Score each frame based on sharpness/contrast
//! 2. **Frame Selection**: Select best N% of frames
//! 3. **Alignment**: Cross-correlation to find translation offset
//! 4. **Stacking**: Combine aligned frames using percentile method
//!
//! # Usage
//!
//! ```ignore
//! let config = PlanetaryConfig::default();
//! let mut stacker = PlanetaryStacker::new(config)?;
//!
//! // Add frames (automatically scored and selected)
//! for frame in frames {
//!     stacker.add_frame(&frame)?;
//! }
//!
//! // Stack best frames
//! let result = stacker.stack()?;
//! ```

mod alignment;
mod config;
mod quality;
mod stacker;

#[cfg(test)]
mod tests;

// Re-export public types
pub use alignment::compute_alignment;
pub use config::{
    AlignmentRoi, PlanetaryConfig, PlanetaryStackMethod, PlanetaryStackStats, QualityMetric,
};
pub use quality::compute_quality;
pub use stacker::{
    stack_planetary, PlanetaryStacker, PlanetaryStackerPlugin, ScoredFrame, PLANETARY_PLUGIN,
};
