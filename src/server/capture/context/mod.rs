//! Stacking contexts for the capture loop.
//!
//! One context per stacking mode, each owning the state that mode accumulates
//! across a session: `StackingContext` for star-registered deep-sky work,
//! `PlanetaryStackingContext` for correlation-aligned planetary work, and
//! `CometStacker` over the Pro `CometPlugin`. The stacking task drives all three
//! through [`LiveStacker`], so a new mode is one impl plus a [`create_live_stacker`] arm.

mod comet;
mod deep_sky;
mod planetary;

pub use comet::CometStacker;
#[cfg(test)]
pub(crate) use comet::StubComet;
pub use deep_sky::StackingContext;
pub use planetary::PlanetaryStackingContext;

use crate::frame::{Frame, NoiseField};
use crate::server::state::CaptureSettings;
use crate::stacking::StackingType;

use super::frame_gate::FrameAdmission;

/// One stacking mode's accumulator, as the stacking task drives it frame by frame.
pub trait LiveStacker: Send {
    /// The mode this accumulator serves. A carried stack only resumes under the same one.
    fn kind(&self) -> StackingType;

    /// `(width, height, channels)` the stack was built for.
    fn geometry(&self) -> (usize, usize, usize);

    /// Frames in the stack, the reference included.
    fn depth(&self) -> usize;

    fn has_reference(&self) -> bool;

    /// Re-reads the mode's parameters; called before every frame so an edit lands mid-stack.
    fn apply_settings(&mut self, settings: &CaptureSettings);

    fn set_reference(&mut self, frame: &Frame) -> Result<(), String>;

    /// Offers a frame. `Err` means the stack could not take it at all, and the caller
    /// shows the raw sub; a frame merely not admitted is `Ok` with `added: false`.
    fn offer(&mut self, frame: &Frame, settings: &CaptureSettings) -> Result<FrameAdmission, String>;

    /// The stack for display, with its coverage map where the mode keeps one and it has
    /// something to say (see [`NoiseField::is_usable`]).
    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), String>;
}

/// Builds the accumulator for `kind`, sized to `frame`.
pub fn create_live_stacker(
    kind: StackingType,
    frame: &Frame,
    settings: &CaptureSettings,
) -> Result<Box<dyn LiveStacker>, String> {
    let (width, height, channels) = (frame.width(), frame.height(), frame.channels());
    match kind {
        StackingType::DeepSky => StackingContext::new(width, height, channels, settings)
            .map(|ctx| Box::new(ctx) as Box<dyn LiveStacker>)
            .ok_or_else(|| "Failed to create stacking context".to_string()),
        StackingType::Planetary => PlanetaryStackingContext::new(width, height, channels, settings)
            .map(|ctx| Box::new(ctx) as Box<dyn LiveStacker>)
            .ok_or_else(|| "Failed to create planetary stacking context".to_string()),
        StackingType::Comet => CometStacker::new(width, height, channels, settings)
            .map(|ctx| Box::new(ctx) as Box<dyn LiveStacker>),
    }
}

/// The stacking state a capture leaves behind when it ends unexpectedly — a dropout
/// mid-session must not cost the whole two hours. Parked in `AppState.stacking_carryover`
/// when a reconnect will resume the session, and handed to the next stacking task.
/// Only valid for a resume at the same frame geometry and mode — discarded by the
/// stacking task's reset checks otherwise, same as a mid-session binning change.
pub struct StackingCarryover {
    pub stacker: Box<dyn LiveStacker>,
}
