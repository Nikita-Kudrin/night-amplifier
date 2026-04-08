//! Push-To Navigation System (Pro Feature)
//!
//! Push-To navigation is a professional feature available only in Night Amplifier Pro.
//! This module provides the plugin interfaces to allow safe compilation of the Community version
//! while gating the functionality.

pub mod error;

pub use error::{PushToError, PushToResult};

use crate::detection::StarDetector;
use crate::frame::Frame;
use crate::server::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, CoordinateResponse,
    DatabaseTypeResponse, PushToDirectionResponse, PushToPositionResponse, PushToStatusResponse,
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

/// Plugin trait for advanced Push-To navigation
#[async_trait]
pub trait PushToSystemPlugin: Send + Sync {
    /// Process a new frame for plate solving.
    /// Needs to return both the position result (if successful) and the direction result (if target is set)
    async fn process_new_frame(
        &self,
        frame: &Frame,
        detector: &StarDetector,
    ) -> PushToResult<(
        Option<PushToPositionResponse>,
        Option<PushToDirectionResponse>,
    )>;

    // Real-time API
    async fn get_status(&self) -> PushToStatusResponse;
    async fn cancel_solve(&self) -> PushToResult<()>;
    async fn search_catalog(&self, query: &str, limit: usize) -> Vec<CatalogEntryResponse>;
    async fn get_catalog_by_type(&self, catalog_type: &str) -> Vec<CatalogEntryResponse>;
    async fn set_target_by_name(&self, name: &str) -> Result<CatalogEntryResponse, String>;
    async fn set_target_by_coords(&self, ra: f64, dec: f64) -> Result<CoordinateResponse, String>;
    async fn clear_target(&self) -> Result<(), String>;
    async fn get_direction(&self) -> Option<PushToDirectionResponse>;
    async fn set_fov(&self, fov: f32) -> Result<(), String>;
    async fn load_database(&self, path: &str) -> Result<(), String>;

    // Installers
    async fn get_astap_status(&self) -> AstapStatusResponse;
    async fn get_astap_databases(&self) -> Vec<DatabaseTypeResponse>;
    async fn install_astap(
        &self,
        database_type: &str,
        events: tokio::sync::broadcast::Sender<crate::server::ServerEvent>,
    ) -> Result<(), String>;
    async fn get_catalog_status(&self) -> CatalogStatusResponse;
    async fn install_catalog(
        &self,
        events: tokio::sync::broadcast::Sender<crate::server::ServerEvent>,
    ) -> Result<(), String>;
}

/// Global registry for the Push-To plugin
pub static PUSH_TO_PLUGIN: OnceLock<Box<dyn PushToSystemPlugin>> = OnceLock::new();
