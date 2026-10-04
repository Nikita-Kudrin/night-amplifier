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

use tracing::warn;

use crate::frame::{Frame, NoiseField};
use crate::server::state::CaptureSettings;
use crate::stacking::{RejectionMethod, StackingConfig, StackingType, REJECTION_PLUGIN};

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

/// The accumulator configuration a deep-sky or planetary session runs for `settings`,
/// at session start and on every edit alike — planetary once started on licence state
/// and then passed Min-Max straight through on the first edit.
pub(super) fn stacking_config(settings: &CaptureSettings) -> StackingConfig {
    StackingConfig::default()
        .with_rejection(resolve_rejection(settings))
        .with_sigma(settings.rejection_sigma)
        .with_weighting(settings.weighting_preset.into())
}

/// The nearest method the *live* accumulator can actually execute.
///
/// `MinMax` needs the min/max of a sample set nobody keeps — `MasterStack` holds 16
/// bytes a pixel and no history, and two more floats would put a 3008x3008x3 stack at
/// 650 MB. Only the batch `compute_rejection` implements it; the live path routes only
/// the two clipping methods to the plugin and averages everything else, so passing
/// `MinMax` through would leave the session with *no* rejection. Substituting the
/// nearest method that does run is the honest reading of what was asked for.
fn live_equivalent(method: RejectionMethod) -> RejectionMethod {
    match method {
        RejectionMethod::MinMax => RejectionMethod::SigmaClip,
        other => other,
    }
}

/// The rejection method a session should run: what the observer asked for, reduced to
/// what the live path can execute, or `None` when the Pro plugin is not loaded.
fn resolve_rejection(settings: &CaptureSettings) -> RejectionMethod {
    if crate::license::pro_plugin(&REJECTION_PLUGIN).is_none() {
        return RejectionMethod::None;
    }
    let resolved = live_equivalent(settings.rejection_method);
    if resolved != settings.rejection_method {
        warn!(
            requested = ?settings.rejection_method,
            using = ?resolved,
            "Requested rejection method needs frame history the live stack does not keep"
        );
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive: a new variant must be considered by every test here.
    const ALL_METHODS: [RejectionMethod; 4] = [
        RejectionMethod::None,
        RejectionMethod::SigmaClip,
        RejectionMethod::WinsorizedSigmaClip,
        RejectionMethod::MinMax,
    ];

    /// Without the Pro plugin every method resolves to `None`, whatever the observer
    /// asked for — Community has no implementation to run.
    #[test]
    fn rejection_needs_the_plugin() {
        let mut settings = CaptureSettings::default();
        for method in ALL_METHODS {
            settings.rejection_method = method;
            assert_eq!(
                resolve_rejection(&settings),
                RejectionMethod::None,
                "{method:?} resolved to something Community cannot run"
            );
        }
    }

    /// Every method must reduce to one the live accumulator actually routes to the
    /// plugin. `MasterStack` sends only the two clipping methods there and averages
    /// everything else, so a method that survives this unchanged and is not in that pair
    /// disables rejection silently — which is what choosing Min-Max used to do.
    #[test]
    fn every_method_reduces_to_one_the_live_stack_runs() {
        for method in ALL_METHODS {
            let resolved = live_equivalent(method);
            assert!(
                matches!(
                    resolved,
                    RejectionMethod::None
                        | RejectionMethod::SigmaClip
                        | RejectionMethod::WinsorizedSigmaClip
                ),
                "{method:?} reduces to {resolved:?}, which the live stack does not route \
                 to the rejection plugin — it would silently average instead"
            );
        }
    }

    /// The substitution only applies where it has to.
    #[test]
    fn methods_the_live_stack_runs_are_left_alone() {
        for method in [
            RejectionMethod::None,
            RejectionMethod::SigmaClip,
            RejectionMethod::WinsorizedSigmaClip,
        ] {
            assert_eq!(live_equivalent(method), method);
        }
    }

    /// The shipped default asks for the best method the build runs, or reading the field
    /// turns rejection off for every Pro observer without a persisted setting. Community
    /// registers no plugin; Pro's `rejection_defaults` checks sigma clipping.
    #[test]
    fn default_settings_ask_for_the_builds_best_method() {
        assert_eq!(
            CaptureSettings::default().rejection_method,
            RejectionMethod::best_available()
        );
    }

    /// Deep-sky and planetary run the same rejection for the same settings, at start and
    /// after an edit — planetary used to differ on both.
    #[test]
    fn planetary_and_deep_sky_configure_the_stack_alike() {
        for method in ALL_METHODS {
            let mut settings = CaptureSettings::default();
            settings.rejection_method = method;
            settings.rejection_sigma = 3.1;

            let mut deep_sky = StackingContext::new(32, 32, 1, &settings).expect("builds");
            let mut planetary =
                PlanetaryStackingContext::new(32, 32, 1, &settings).expect("builds");
            assert_eq!(deep_sky.stacker.config(), planetary.stacker.config(), "{method:?}");

            deep_sky.update_from_settings(&settings);
            planetary.update_from_settings(&settings);
            assert_eq!(deep_sky.stacker.config(), planetary.stacker.config(), "{method:?}");
        }
    }
}
