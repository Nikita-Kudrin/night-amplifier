//! Push-To Navigation System (Pro Feature)
//!
//! Push-To navigation is a professional feature available only in Night Amplifier Pro.
//! This module provides the plugin interfaces to allow safe compilation of the Community version
//! while gating the functionality.
//!
//! The plugin is split into three focused sub-traits following the Interface Segregation Principle:
//! - [`PushToSolverPlugin`] — plate solving, position tracking, and direction calculation
//! - [`PushToCatalogPlugin`] — catalog search, target selection, and database loading
//! - [`PushToInstallerPlugin`] — ASTAP and catalog installation management

pub mod error;
pub use error::{PushToError, PushToResult};

use crate::detection::StarDetector;
use crate::frame::Frame;
use crate::server::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, CoordinateResponse,
    DatabaseTypeResponse, PushToDirectionResponse, PushToPositionResponse, PushToStatusResponse,
    TelescopeSettings,
};
use async_trait::async_trait;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStage {
    /// Downloading ASTAP CLI binary
    DownloadingCli,
    /// Extracting ASTAP CLI binary
    ExtractingCli,
    /// ASTAP CLI completed
    CliCompleted,
    /// Downloading star database
    DownloadingDatabase,
    /// Extracting star database
    ExtractingDatabase,
    /// Database completed (all done)
    DatabaseCompleted,
    /// Catalog files (used for OpenNGC catalog)
    CatalogFiles,
}

impl InstallStage {
    /// Get a human-readable name for this stage
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::DownloadingCli => "Downloading ASTAP CLI",
            Self::ExtractingCli => "Extracting ASTAP CLI",
            Self::CliCompleted => "ASTAP CLI Installed",
            Self::DownloadingDatabase => "Downloading Database",
            Self::ExtractingDatabase => "Extracting Database",
            Self::DatabaseCompleted => "Database Installed",
            Self::CatalogFiles => "Target Catalog",
        }
    }

    /// Get overall progress percentage for this stage (0-100)
    pub fn base_progress(&self) -> f32 {
        match self {
            Self::DownloadingCli => 0.0,
            Self::ExtractingCli => 15.0,
            Self::CliCompleted => 20.0,
            Self::DownloadingDatabase => 20.0,
            Self::ExtractingDatabase => 90.0,
            Self::DatabaseCompleted => 100.0,
            Self::CatalogFiles => 0.0,
        }
    }

    /// Get the weight of this stage in overall progress
    pub fn weight(&self) -> f32 {
        match self {
            Self::DownloadingCli => 15.0,      // 0-15%
            Self::ExtractingCli => 5.0,        // 15-20%
            Self::CliCompleted => 0.0,         // checkpoint
            Self::DownloadingDatabase => 70.0, // 20-90%
            Self::ExtractingDatabase => 10.0,  // 90-100%
            Self::DatabaseCompleted => 0.0,    // checkpoint
            Self::CatalogFiles => 100.0,       // independent process
        }
    }
}

/// What one call to [`PushToSolverPlugin::process_new_frame`] actually did.
///
/// The caller needs this to know whether the position it was handed is news. Without
/// it every frame that merely reuses the previous solve is announced as a fresh one:
/// the field log for 2026-08-22 carries ~1500 identical `Plate solve succeeded`
/// lines at one per second, and each of those also overwrites a genuine failure in
/// the UI on the very next frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SolveOutcome {
    /// A plate solve ran on this frame and succeeded.
    Solved,
    /// No solve ran; any position returned is the previous solve's.
    #[default]
    Cached,
    /// No solve ran and there is nothing cached to report.
    Idle,
}

/// The result of offering one frame to the Push-To system.
#[derive(Debug, Clone, Default)]
pub struct FrameOutcome {
    /// Whether `position` came from a solve on this frame or from the cache.
    pub outcome: SolveOutcome,
    /// Current pointing, freshly solved or cached.
    pub position: Option<PushToPositionResponse>,
    /// Direction to the current target, if both are known.
    pub direction: Option<PushToDirectionResponse>,
}

impl FrameOutcome {
    /// Nothing known and nothing done.
    pub fn idle() -> Self {
        Self {
            outcome: SolveOutcome::Idle,
            position: None,
            direction: None,
        }
    }

    /// A position and direction reused from an earlier solve.
    pub fn cached(
        position: Option<PushToPositionResponse>,
        direction: Option<PushToDirectionResponse>,
    ) -> Self {
        Self {
            outcome: if position.is_none() {
                SolveOutcome::Idle
            } else {
                SolveOutcome::Cached
            },
            position,
            direction,
        }
    }

    /// A position solved on this frame.
    pub fn solved(
        position: PushToPositionResponse,
        direction: Option<PushToDirectionResponse>,
    ) -> Self {
        Self {
            outcome: SolveOutcome::Solved,
            position: Some(position),
            direction,
        }
    }
}

/// Why Push-To is not attempting to solve right now.
///
/// Reported so the UI can say *which* precondition is missing. Before this, every
/// one of these cases logged at `debug!` and returned, which is what "I installed
/// ASTAP and nothing happens" looks like from the outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushToBlocker {
    /// No target has been selected.
    NoTarget,
    /// ASTAP binary or star database is missing.
    SolverNotReady,
    /// A solve failed recently; the next attempt is being held off.
    BackingOff,
}

impl PushToBlocker {
    /// Human-readable explanation for the UI.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::NoTarget => "No target selected",
            Self::SolverNotReady => "ASTAP or its star database is not installed",
            Self::BackingOff => "Waiting before the next solve attempt",
        }
    }
}

/// Plate solving, position tracking, and direction calculation.
#[async_trait]
pub trait PushToSolverPlugin: Send + Sync {
    /// Initialize the plugin with an event sender
    fn init(&self, _events: tokio::sync::broadcast::Sender<crate::server::ServerEvent>) {}

    /// Process a new frame for plate solving.
    ///
    /// The returned [`FrameOutcome`] says whether a solve actually ran, so callers can
    /// tell a fresh position from a cached one.
    async fn process_new_frame(
        &self,
        frame: &Frame,
        detector: &StarDetector,
        wanderer_mode: bool,
    ) -> PushToResult<FrameOutcome>;

    /// Get the current Push-To navigation status
    async fn get_status(&self) -> PushToStatusResponse;

    /// Cancel the current plate solving process.
    ///
    /// Returns `true` if a solve was actually in flight. Callers use this to avoid
    /// announcing a cancellation that never happened — an unconditional
    /// `PositionSolveFailed` on every settings save reads as "Failed to find M31" in
    /// the status bar.
    async fn cancel_solve(&self) -> PushToResult<bool>;

    /// Cancel any solve in flight and arm the system to solve again on the next
    /// settled frame.
    ///
    /// This is what an equipment change needs: the in-flight solve was computed
    /// against the old focal length or sensor and is worthless, but the user still
    /// wants a position. A bare cancel leaves the movement detector reporting `Idle`
    /// for an unchanged star field, so nothing would ever restart it.
    async fn restart_solve(&self) -> PushToResult<()>;

    /// Get the current push direction to target
    async fn get_direction(&self) -> Option<PushToDirectionResponse>;

    /// Update the field-of-view hint for the solver
    async fn set_fov(&self, fov: f32) -> Result<(), String>;

    /// Update telescope settings for FOV calculation.
    /// The solver will compute the precise image-height FOV from these parameters.
    async fn set_telescope_settings(&self, settings: TelescopeSettings) -> Result<(), String>;
}

/// Catalog search, target selection, and database operations.
#[async_trait]
pub trait PushToCatalogPlugin: Send + Sync {
    /// Search the catalog for targets matching a query
    async fn search_catalog(&self, query: &str, limit: usize) -> Vec<CatalogEntryResponse>;

    /// Get all catalog entries of a specific type (e.g. "Messier", "NGC", "IC")
    async fn get_catalog_by_type(&self, catalog_type: &str) -> Vec<CatalogEntryResponse>;

    /// Set the current target by catalog name (e.g. "M31", "NGC 7000")
    async fn set_target_by_name(&self, name: &str) -> Result<CatalogEntryResponse, String>;

    /// Set the current target by RA/Dec coordinates
    async fn set_target_by_coords(&self, ra: f64, dec: f64) -> Result<CoordinateResponse, String>;

    /// Clear the current target
    async fn clear_target(&self) -> Result<(), String>;

    /// Load a solver database from the given path
    async fn load_database(&self, path: &str) -> Result<(), String>;
}

/// ASTAP binary and catalog installation management.
#[async_trait]
pub trait PushToInstallerPlugin: Send + Sync {
    /// Get ASTAP installation status
    async fn get_astap_status(&self) -> AstapStatusResponse;

    /// Get available database types for installation
    async fn get_astap_databases(&self) -> Vec<DatabaseTypeResponse>;

    /// Start ASTAP installation (binary and selected databases)
    async fn install_astap(
        &self,
        database_types: &[String],
        events: tokio::sync::broadcast::Sender<crate::server::ServerEvent>,
    ) -> Result<(), String>;

    /// Get OpenNGC catalog installation status
    async fn get_catalog_status(&self) -> CatalogStatusResponse;

    /// Start OpenNGC catalog installation
    async fn install_catalog(
        &self,
        include_stars: bool,
        events: tokio::sync::broadcast::Sender<crate::server::ServerEvent>,
    ) -> Result<(), String>;
}

/// Combined Push-To plugin trait for registration in the global OnceLock.
///
/// Implementors must provide all three sub-traits. The single OnceLock keeps
/// the registration pattern simple while sub-traits let consumers depend only
/// on the interface they need.
pub trait PushToSystemPlugin:
    PushToSolverPlugin + PushToCatalogPlugin + PushToInstallerPlugin
{
}

/// Blanket implementation: any type implementing all three sub-traits is a PushToSystemPlugin.
impl<T: PushToSolverPlugin + PushToCatalogPlugin + PushToInstallerPlugin> PushToSystemPlugin for T {}

/// Global registry for the Push-To plugin
pub static PUSH_TO_PLUGIN: OnceLock<Box<dyn PushToSystemPlugin>> = OnceLock::new();
