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

use thiserror::Error;
use tracing::warn;

use night_amplifier_core::error::StackError;
use night_amplifier_core::frame::{Frame, NoiseField};
use night_amplifier_core::planetary::AlignmentRoi;
use crate::state::CaptureSettings;
use night_amplifier_core::plugins::Plugins;
use night_amplifier_core::stacking::{CometSettings, RejectionMethod, StackingConfig, StackingType};

use super::frame_gate::FrameAdmission;

/// Reference stars star registration needs: one triangle.
pub const MIN_REFERENCE_STARS: usize = 3;

/// Why a live stack could not be built, started or read. A frame the stack merely did not
/// admit is not one of these: that is a [`FrameAdmission`].
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LiveStackError {
    /// The mode runs on a Pro plugin this build or licence does not have.
    #[error("{} stacking needs Night Amplifier Pro", .0.display_name())]
    PluginMissing(StackingType),

    #[error("Too few stars detected ({found}) for registration, need at least {MIN_REFERENCE_STARS}")]
    TooFewStars { found: usize },

    /// A frame was offered before the reference that starts the stack.
    #[error("The stack has no reference frame yet")]
    NoReference,

    /// The accumulator, star detection or the mode's plugin failed.
    #[error(transparent)]
    Stack(#[from] StackError),
}

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
    fn apply_settings(&mut self, settings: &StackSettings);

    fn set_reference(&mut self, frame: &Frame) -> Result<(), LiveStackError>;

    /// Offers a frame. `Err` means the stack could not take it at all, and the caller
    /// shows the raw sub; a frame merely not admitted is `Ok` with `added: false`.
    fn offer(
        &mut self,
        frame: &Frame,
        settings: &StackSettings,
    ) -> Result<FrameAdmission, LiveStackError>;

    /// The stack for display, with its coverage map where the mode keeps one and it has
    /// something to say (see [`NoiseField::is_usable`]).
    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), LiveStackError>;
}

/// What the live stackers read from the session's settings, taken once per frame.
#[derive(Debug, Clone)]
pub struct StackSettings {
    /// The accumulator configuration, rejection already reduced to what the live path runs.
    pub config: StackingConfig,
    pub comet_roi: Option<AlignmentRoi>,
    pub planetary_roi: Option<AlignmentRoi>,
    pub planetary_auto_tracking: bool,
    pub planetary_multi_point_alignment: bool,
    /// What the stack runs: rejection, multi-point alignment, the comet nucleus.
    pub plugins: Plugins,
}

impl StackSettings {
    pub fn of(settings: &CaptureSettings, plugins: &Plugins) -> Self {
        Self {
            config: stacking_config(settings, plugins),
            comet_roi: settings.comet_roi,
            planetary_roi: settings.planetary_roi,
            planetary_auto_tracking: settings.planetary_auto_tracking,
            planetary_multi_point_alignment: settings.planetary_multi_point_alignment,
            plugins: plugins.clone(),
        }
    }

    /// The comet plugin's view of these settings.
    pub fn comet(&self) -> CometSettings {
        CometSettings {
            roi: self.comet_roi,
            stacking: self.config.clone(),
            plugins: self.plugins.clone(),
        }
    }
}

/// Builds the accumulator for `kind`, sized to `frame`.
pub fn create_live_stacker(
    kind: StackingType,
    frame: &Frame,
    settings: &StackSettings,
) -> Result<Box<dyn LiveStacker>, LiveStackError> {
    let (width, height, channels) = (frame.width(), frame.height(), frame.channels());
    Ok(match kind {
        StackingType::DeepSky => Box::new(StackingContext::new(width, height, channels, settings)?),
        StackingType::Planetary => {
            Box::new(PlanetaryStackingContext::new(width, height, channels, settings)?)
        }
        StackingType::Comet => Box::new(CometStacker::new(width, height, channels, settings)?),
    })
}

/// The stacking state a capture leaves behind when it ends unexpectedly — a dropout
/// mid-session must not cost the whole two hours. Parked in `AppState::resume`
/// when a reconnect will resume the session, and handed to the next stacking task.
/// Only valid for a resume at the same frame geometry and mode — discarded by the
/// stacking task's reset checks otherwise, same as a mid-session binning change.
pub struct StackingCarryover {
    pub stacker: Box<dyn LiveStacker>,
}

/// The accumulator configuration every mode runs for `settings`, at session start and on
/// every edit alike — planetary once started on licence state and then passed Min-Max
/// straight through on the first edit.
fn stacking_config(settings: &CaptureSettings, plugins: &Plugins) -> StackingConfig {
    StackingConfig::default()
        .with_rejection(resolve_rejection(settings, plugins))
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
fn resolve_rejection(settings: &CaptureSettings, plugins: &Plugins) -> RejectionMethod {
    if plugins.rejection().is_none() {
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
    use night_amplifier_core::stacking::{IncrementalPixel, RejectionPlugin};
    use std::sync::Arc;

    /// Stands in for Pro's rejection plugin where only its presence matters.
    struct FakeRejection;

    impl RejectionPlugin for FakeRejection {
        fn compute_rejection(
            &self,
            _: &[f32],
            _: RejectionMethod,
            _: &StackingConfig,
        ) -> night_amplifier_core::error::Result<(f32, u32)> {
            unreachable!("configuration only")
        }

        fn compute_weighted_rejection(
            &self,
            _: &[f32],
            _: &[f32],
            _: RejectionMethod,
            _: &StackingConfig,
        ) -> night_amplifier_core::error::Result<(f32, f32)> {
            unreachable!("configuration only")
        }

        fn blend_incremental(
            &self,
            _: &mut [IncrementalPixel],
            _: &[f32],
            _: f32,
            _: f32,
            _: f32,
            _: &StackingConfig,
        ) -> night_amplifier_core::error::Result<()> {
            unreachable!("configuration only")
        }
    }

    fn community() -> Plugins {
        Plugins::none().always_licensed()
    }

    fn pro() -> Plugins {
        Plugins::none().with_rejection(Arc::new(FakeRejection)).always_licensed()
    }

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
                resolve_rejection(&settings, &community()),
                RejectionMethod::None,
                "{method:?} resolved to something Community cannot run"
            );
        }
    }

    /// With the plugin, a session runs what was asked, reduced to what the live path runs.
    #[test]
    fn with_the_plugin_the_method_asked_for_runs() {
        let mut settings = CaptureSettings::default();
        for method in ALL_METHODS {
            settings.rejection_method = method;
            assert_eq!(resolve_rejection(&settings, &pro()), live_equivalent(method));
        }
    }

    /// The licence gates the plugin: unlicensed, it resolves like Community. Nothing in
    /// this test binary activates the process licence.
    #[test]
    fn an_unlicensed_plugin_resolves_like_community() {
        let gated = Plugins::none().with_rejection(Arc::new(FakeRejection));
        let settings = CaptureSettings {
            rejection_method: RejectionMethod::SigmaClip,
            ..CaptureSettings::default()
        };
        assert_eq!(resolve_rejection(&settings, &gated), RejectionMethod::None);
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
        for (method, plugins) in ALL_METHODS.into_iter().flat_map(|m| [(m, community()), (m, pro())]) {
            let mut settings = CaptureSettings::default();
            settings.rejection_method = method;
            settings.rejection_sigma = 3.1;

            let settings = StackSettings::of(&settings, &plugins);
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
