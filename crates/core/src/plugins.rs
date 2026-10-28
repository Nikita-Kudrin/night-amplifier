//! The Pro plugins a process runs with, as one value.
//!
//! Each slot is a Community trait Pro implements; an empty slot is Community behaviour.
//! The binary [`install`]s its set once at startup and `AppState` carries it to everything
//! the server drives, so a test hands its own set (or [`Plugins::none`]) to the code under
//! test instead of filling process-wide slots — one test binary can compare Community and
//! Pro behaviour. Domain constructors without a set of their own take [`Plugins::installed`].
//!
//! Every accessor is **licence-gated**: it answers `None` while the Pro licence is
//! inactive, so a licence that lapses mid-session degrades the very next frame.

use std::fmt;
use std::sync::{Arc, RwLock};

use crate::background::BackgroundAlgorithmPlugin;
use crate::planetary::PlanetaryStackerPlugin;
use crate::push_to::{FieldObjectsPlugin, PushToCatalogPlugin, PushToInstallerPlugin, PushToSolverPlugin};
use crate::render::denoise::ai::AiDenoisePlugin;
use crate::render::denoise::DenoisePlugin;
use crate::render::stretch::SaturationPlugin;
use crate::stacking::{CometPlugin, RejectionPlugin};

/// A filled slot: the plugin, and the type that implements it.
struct Filled<P: ?Sized> {
    plugin: Arc<P>,
    implementation: &'static str,
}

impl<P: ?Sized> Clone for Filled<P> {
    fn clone(&self) -> Self {
        Self { plugin: Arc::clone(&self.plugin), implementation: self.implementation }
    }
}

#[derive(Clone, Default)]
struct Slots {
    rejection: Option<Filled<dyn RejectionPlugin>>,
    background: Option<Filled<dyn BackgroundAlgorithmPlugin>>,
    planetary: Option<Filled<dyn PlanetaryStackerPlugin>>,
    saturation: Option<Filled<dyn SaturationPlugin>>,
    denoise: Option<Filled<dyn DenoisePlugin>>,
    ai_denoise: Option<Filled<dyn AiDenoisePlugin>>,
    comet: Option<Filled<dyn CometPlugin>>,
    push_to_solver: Option<Filled<dyn PushToSolverPlugin>>,
    push_to_catalog: Option<Filled<dyn PushToCatalogPlugin>>,
    push_to_installer: Option<Filled<dyn PushToInstallerPlugin>>,
    field_objects: Option<Filled<dyn FieldObjectsPlugin>>,
    /// Answer regardless of the process licence — for tests, which own their plugins.
    always_licensed: bool,
}

/// A set of plugins. Cloning shares it.
#[derive(Clone, Default)]
pub struct Plugins(Arc<Slots>);

static INSTALLED: RwLock<Option<Plugins>> = RwLock::new(None);

/// Adds `plugins` to the process's set. A slot already filled keeps its plugin, so a
/// second call only adds what the first left out.
pub fn install(plugins: Plugins) {
    let mut installed = INSTALLED.write().unwrap_or_else(|e| e.into_inner());
    let merged = match installed.take() {
        Some(existing) => existing.or(plugins),
        None => plugins,
    };
    *installed = Some(merged);
}

macro_rules! slots {
    ($($slot:ident: $with:ident, $plugin:ident;)*) => {
        impl Plugins {
            $(
                pub fn $with<P: $plugin + 'static>(mut self, plugin: Arc<P>) -> Self {
                    let implementation = std::any::type_name::<P>();
                    Arc::make_mut(&mut self.0).$slot = Some(Filled::<dyn $plugin> { plugin, implementation });
                    self
                }

                /// Licence-gated: `None` without the plugin or while the licence is inactive.
                pub fn $slot(&self) -> Option<&dyn $plugin> {
                    self.0.$slot.as_ref().map(|s| &*s.plugin).filter(|_| self.licensed())
                }
            )*

            /// Each filled slot with the type filling it, licence aside. [`Self::registered`]
            /// names only the slots, so a test's fake and Pro's plugin read alike there.
            pub fn implementations(&self) -> Vec<(&'static str, &'static str)> {
                [$((stringify!($slot), self.0.$slot.as_ref().map(|s| s.implementation)),)*]
                    .into_iter()
                    .filter_map(|(slot, implementation)| Some((slot, implementation?)))
                    .collect()
            }

            /// Every slot of `self` that is filled, the rest from `other`.
            fn or(self, other: Plugins) -> Plugins {
                let (mine, theirs) = (&self.0, &other.0);
                Plugins(Arc::new(Slots {
                    $($slot: mine.$slot.clone().or_else(|| theirs.$slot.clone()),)*
                    always_licensed: mine.always_licensed || theirs.always_licensed,
                }))
            }
        }
    };
}

slots! {
    rejection: with_rejection, RejectionPlugin;
    background: with_background, BackgroundAlgorithmPlugin;
    planetary: with_planetary, PlanetaryStackerPlugin;
    saturation: with_saturation, SaturationPlugin;
    denoise: with_denoise, DenoisePlugin;
    ai_denoise: with_ai_denoise, AiDenoisePlugin;
    comet: with_comet, CometPlugin;
    push_to_solver: with_push_to_solver, PushToSolverPlugin;
    push_to_catalog: with_push_to_catalog, PushToCatalogPlugin;
    push_to_installer: with_push_to_installer, PushToInstallerPlugin;
    field_objects: with_field_objects, FieldObjectsPlugin;
}

impl Plugins {
    /// Community: no plugin in any slot.
    pub fn none() -> Self {
        Self::default()
    }

    /// The set the process [`install`]ed, or none.
    pub fn installed() -> Self {
        INSTALLED
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_default()
    }

    /// One Push-To implementation in all three of its slots.
    pub fn with_push_to<P>(self, plugin: Arc<P>) -> Self
    where
        P: PushToSolverPlugin + PushToCatalogPlugin + PushToInstallerPlugin + 'static,
    {
        self.with_push_to_solver(plugin.clone())
            .with_push_to_catalog(plugin.clone())
            .with_push_to_installer(plugin)
    }

    /// These plugins answer whatever the process licence says. For tests, whose plugins
    /// are their own; the binary's set stays gated.
    pub fn always_licensed(mut self) -> Self {
        Arc::make_mut(&mut self.0).always_licensed = true;
        self
    }

    /// Whether the accessors answer: the set is licence-free, or the process is licensed.
    pub fn licensed(&self) -> bool {
        self.0.always_licensed || crate::license::is_pro_active()
    }

    /// Whether the build ships sigma clipping, licence aside — what a session asks for
    /// by default ([`crate::stacking::RejectionMethod::best_available`]).
    pub fn ships_rejection(&self) -> bool {
        self.0.rejection.is_some()
    }

    /// Whether the build ships RBF background extraction, licence aside — what a session
    /// asks for by default ([`crate::background::BackgroundExtractionAlgorithm::best_available`]).
    pub fn ships_background(&self) -> bool {
        self.0.background.is_some()
    }

    /// Whether the build ships Shadow Saturation Boost, licence aside — whether a session
    /// starts with it on. Community's API refuses it, so its default cannot be on.
    pub fn ships_saturation(&self) -> bool {
        self.0.saturation.is_some()
    }

    /// The filled slots by name, licence aside, for the startup report.
    pub fn registered(&self) -> Vec<&'static str> {
        let s = &self.0;
        let push_to =
            s.push_to_solver.is_some() || s.push_to_catalog.is_some() || s.push_to_installer.is_some();
        [
            ("push_to", push_to),
            ("rejection", s.rejection.is_some()),
            ("comet", s.comet.is_some()),
            ("background", s.background.is_some()),
            ("planetary", s.planetary.is_some()),
            ("saturation", s.saturation.is_some()),
            ("denoise", s.denoise.is_some()),
            ("ai_denoise", s.ai_denoise.is_some()),
            ("field_objects", s.field_objects.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, filled)| filled.then_some(name))
        .collect()
    }
}

impl fmt::Debug for Plugins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Plugins")
            .field("registered", &self.registered())
            .field("always_licensed", &self.0.always_licensed)
            .finish()
    }
}

#[cfg(test)]
#[path = "plugins_tests.rs"]
mod tests;
