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

/// The stacking state a capture leaves behind when it ends unexpectedly — a dropout
/// mid-session must not cost the whole two hours. Normally owned by the stacking task
/// for the life of one capture and dropped on exit; when a reconnect will resume the
/// session, parked in `AppState.stacking_carryover` and handed to the next stacking
/// task instead. Only valid for a resume at the same frame geometry — discarded by
/// the stacking task's existing dimension-mismatch check otherwise, same as a
/// mid-session binning change.
pub struct StackingCarryover {
    pub stacking: Option<StackingContext>,
    pub comet: Option<Box<dyn CometContext>>,
    pub planetary: Option<PlanetaryStackingContext>,
}
