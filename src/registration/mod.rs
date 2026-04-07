//! Image Registration using Triangle Matching
//!
//! # Algorithm Overview
//!
//! Image registration aligns frames by finding corresponding stars between a reference
//! frame and each new frame. This module uses the **Triangle Matching** algorithm:
//!
//! 1. **Triangle Formation**: Create "asterisms" from triplets of nearby stars
//! 2. **Triangle Descriptor**: Compute scale-invariant ratios of side lengths
//! 3. **Matching**: Find triangles with similar descriptors between frames
//! 4. **Voting**: Use RANSAC-like voting to find consistent star correspondences
//! 5. **Transform Estimation**: Calculate the affine transformation matrix
//!
//! # Adaptive Registration
//!
//! The adaptive registration system handles various challenging conditions:
//! - **Field rotation**: Automatic detection and compensation
//! - **Cloud cover**: RANSAC-based outlier rejection
//! - **Satellite trails**: Robust statistics ignore outliers
//! - **Brightness variations**: Normalized matching independent of flux
//! - **Different FOV scales**: Adaptive triangle size selection
//!
//! # Triangle Descriptor
//!
//! For each triangle formed by 3 stars (A, B, C), we compute:
//! - Sort sides by length: `a ≤ b ≤ c`
//! - Descriptor: `(a/c, b/c)` - ratios are scale-invariant
//!
//! Two triangles match if their descriptors are within a tolerance.
//!
//! # Affine Transformation
//!
//! The 2D affine transform handles rotation, translation, and scale:
//!
//! ```text
//! | x' |   | cos(θ)·s  -sin(θ)·s  tx | | x |
//! | y' | = | sin(θ)·s   cos(θ)·s  ty | | y |
//! | 1  |   |    0          0       1 | | 1 |
//! ```
//!
//! For astronomical field rotation (no scaling), we solve for θ, tx, ty.
//!
//! # Module Structure
//!
//! - [`triangle`]: Triangle types and geometry
//! - [`transform`]: Affine transformation
//! - [`config`]: Registration configuration and presets
//! - [`matcher`]: Triangle-based star matching
//! - [`ransac`]: RANSAC transform estimation
//! - [`adaptive`]: Adaptive registration strategies
//! - [`engine`]: Core registration pipeline

mod adaptive;
mod config;
mod engine;
mod matcher;
mod ransac;
mod transform;
mod triangle;

pub use adaptive::{
    AdaptiveRegistration, AdaptiveRegistrationResult, BrightnessVariation, FovType,
    RegistrationHints,
};
pub use config::RegistrationConfig;
pub use engine::ImageRegistration;
pub use matcher::TriangleMatcher;
pub use transform::AffineTransform;
pub use triangle::Triangle;

use crate::detection::Star;
use crate::error::Result;

/// Convenience function to register frames with default settings.
pub fn register_frames(ref_stars: &[Star], tgt_stars: &[Star]) -> Result<AffineTransform> {
    ImageRegistration::with_defaults().register(ref_stars, tgt_stars)
}

/// Convenience function to register frames adaptively.
pub fn register_frames_adaptive(
    ref_stars: &[Star],
    tgt_stars: &[Star],
) -> Result<AdaptiveRegistrationResult> {
    AdaptiveRegistration::new().register(ref_stars, tgt_stars)
}
