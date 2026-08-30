//! Stacking contexts for the capture loop.
//!
//! One context per stacking mode, each owning the state that mode accumulates
//! across a session: `StackingContext` for star-registered deep-sky work and
//! `PlanetaryStackingContext` for correlation-aligned planetary work. Comet
//! stacking lives behind `CometPlugin` and is implemented in Pro.

mod deep_sky;
mod planetary;

pub use deep_sky::StackingContext;
pub use planetary::PlanetaryStackingContext;

use crate::stacking::CometContext;

/// The stacking state a capture leaves behind when it ends unexpectedly.
///
/// A dropout in the middle of a two-hour session must not cost the two hours.
/// The stacking task normally owns these contexts for the life of one capture
/// and drops them on exit; when a reconnect is going to resume the session,
/// they are parked in `AppState.stacking_carryover` instead and handed to the
/// next stacking task.
///
/// Only valid for a resume at the same frame geometry — the stacking task's
/// existing dimension-mismatch check discards them otherwise, exactly as it
/// does for a binning change mid-session.
pub struct StackingCarryover {
    pub stacking: Option<StackingContext>,
    pub comet: Option<Box<dyn CometContext>>,
    pub planetary: Option<PlanetaryStackingContext>,
}
