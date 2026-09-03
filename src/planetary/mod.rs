//! Planetary Stacking Module. Differs from DSO imaging throughout: correlation-based
//! registration (surface features, not star triangles), quality-based frame selection
//! (best 10-30%, not all frames), percentile stacking (not sigma-clipped mean),
//! translation-only alignment, and much higher frame rates (30-300fps vs seconds).
//!
//! Pipeline: score each frame by sharpness/contrast -> select best N% -> align via
//! cross-correlation -> combine via percentile stacking.
//!
//! ```ignore
//! let mut stacker = PlanetaryStacker::new(PlanetaryConfig::default())?;
//! for frame in frames { stacker.add_frame(&frame)?; }
//! let result = stacker.stack()?;
//! ```

mod alignment;
mod config;
mod quality;
mod stacker;

#[cfg(test)]
mod tests;

// Re-export public types
pub use alignment::{compute_alignment, compute_centroid};
pub use config::{
    AlignmentRoi, PlanetaryConfig, PlanetaryStackMethod, PlanetaryStackStats, QualityMetric,
};
pub use quality::{compute_quality, frame_to_luminance};
pub use stacker::{
    stack_planetary, BilinearTap, PlanetaryStacker, PlanetaryStackerPlugin, ScoredFrame,
    PLANETARY_PLUGIN,
};
