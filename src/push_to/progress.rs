//! Installation progress tracking types
//!
//! Types for tracking and reporting ASTAP installation progress.

use super::InstallStage;

/// Installation progress update
#[derive(Debug, Clone)]
pub enum InstallProgress {
    /// Starting installation
    Starting { component: String },
    /// Downloading component
    Downloading {
        component: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        stage: Option<InstallStage>,
    },
    /// Extracting archive
    Extracting {
        component: String,
        progress: f32,
        stage: Option<InstallStage>,
    },
    /// Component installed successfully
    Completed {
        component: String,
        stage: Option<InstallStage>,
    },
    /// Installation failed
    Failed { component: String, error: String },
}
