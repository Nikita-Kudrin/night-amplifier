//! Image Registration using Triangle Matching: aligns frames by matching stars between
//! a reference and each new frame. Forms "asterisms" from star triplets, computes
//! scale-invariant side-length ratios `(a/c, b/c)` (sorted `a ≤ b ≤ c`) as a descriptor,
//! matches triangles with similar descriptors, RANSAC-votes for correspondences, and
//! solves the affine transform (θ, tx/ty — no scaling). [`adaptive`] also handles field
//! rotation, cloud cover, satellite trails, brightness/FOV differences.
//!
//! [`adaptive`] runs the ladder on clutter-suppressed lists ([`clutter`]), falls back to
//! translation voting ([`translation`]), refits the winner over every detected star
//! ([`refine`]) and judges it against chance ([`support`]).
//!
//! Submodules: [`triangle`], [`transform`], [`config`], [`matcher`], [`ransac`], [`adaptive`], [`engine`],
//! [`refine`], [`clutter`], [`translation`], [`support`], [`neighbours`].

mod adaptive;
mod clutter;
mod config;
mod engine;
mod matcher;
mod neighbours;
mod ransac;
mod refine;
mod support;
mod transform;
mod translation;
mod triangle;

pub use adaptive::{
    AdaptiveRegistration, AdaptiveRegistrationResult, BrightnessVariation, FovType,
    RegistrationHints,
};
pub use config::RegistrationConfig;
pub use engine::ImageRegistration;
pub use matcher::TriangleMatcher;
pub use refine::refine_transform;
pub use support::Support;
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
