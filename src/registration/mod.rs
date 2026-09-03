//! Image Registration using Triangle Matching: aligns frames by finding corresponding
//! stars between a reference and each new frame. Pipeline: form "asterisms" from star
//! triplets -> compute scale-invariant side-length ratios `(a/c, b/c)` as a descriptor
//! (sorted `a ≤ b ≤ c`) -> match triangles with similar descriptors across frames ->
//! RANSAC-like voting for consistent correspondences -> solve the affine transform.
//!
//! [`adaptive`] handles field rotation, cloud cover, satellite trails, brightness
//! variation, and FOV-scale differences. The 2D affine transform solves for θ
//! (rotation), tx/ty (translation) — no scaling for astronomical field rotation.
//!
//! Submodules: [`triangle`], [`transform`], [`config`], [`matcher`], [`ransac`],
//! [`adaptive`], [`engine`].

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
