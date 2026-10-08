//! What the Push-To plugin reports while it works, and the port it reports through. The
//! server maps each event onto its `/ws/events` message; the plugin never sees the wire.

use super::InstallStage;

/// One thing the plugin has to tell observers.
#[derive(Debug, Clone, PartialEq)]
pub enum PushToEvent {
    SolveStarted {
        target_name: Option<String>,
    },
    /// A cold solve works down an ordered list of attempts, the last of which can run for
    /// a minute; this says which one, so a slow solve is not mistaken for a hung one.
    SolveProgress {
        stage: String,
        attempt: usize,
        total: usize,
    },
    AstapInstallStarting {
        component: String,
    },
    AstapInstallProgress {
        component: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        /// Of the current download, 0-100.
        percent: Option<f32>,
        stage: Option<String>,
        /// Of the whole installation, 0-100.
        overall_percent: Option<f32>,
    },
    AstapInstallExtracting {
        component: String,
        /// Of the extraction, 0-100.
        progress: f32,
        stage: Option<String>,
        overall_percent: Option<f32>,
    },
    AstapInstallCompleted {
        component: String,
        stage: Option<String>,
        overall_percent: Option<f32>,
    },
    AstapInstallFailed {
        component: String,
        error: String,
    },
    CatalogInstallStarting,
    CatalogInstallProgress {
        file_name: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        percent: Option<f32>,
    },
    CatalogFileCompleted {
        file_name: String,
    },
    CatalogInstallCompleted {
        object_count: usize,
    },
    CatalogInstallFailed {
        error: String,
    },
}

/// Where the plugin's events go. The server's implementation forwards them to every
/// `/ws/events` client; a test can record them.
pub trait PushToEvents: Send + Sync {
    fn emit(&self, event: PushToEvent);
}

fn percent_of(done: u64, total: Option<u64>) -> Option<f32> {
    total.map(|total| (done as f32 / total as f32) * 100.0)
}

/// The stage's name, and where `fraction` (0-1) of it puts the whole installation.
fn stage_progress(stage: Option<&InstallStage>, fraction: f32) -> (Option<String>, Option<f32>) {
    match stage {
        Some(s) => (
            Some(s.display_name().to_string()),
            Some(s.base_progress() + s.weight() * fraction),
        ),
        None => (None, None),
    }
}

impl PushToEvent {
    pub fn astap_install_progress(
        component: impl Into<String>,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        stage: Option<&InstallStage>,
    ) -> Self {
        let percent = percent_of(bytes_downloaded, total_bytes);
        let (stage, overall_percent) = stage_progress(stage, percent.unwrap_or(0.0) / 100.0);
        Self::AstapInstallProgress {
            component: component.into(),
            bytes_downloaded,
            total_bytes,
            percent,
            stage,
            overall_percent,
        }
    }

    pub fn astap_install_extracting(
        component: impl Into<String>,
        progress: f32,
        stage: Option<&InstallStage>,
    ) -> Self {
        let (stage, overall_percent) = stage_progress(stage, progress / 100.0);
        Self::AstapInstallExtracting {
            component: component.into(),
            progress,
            stage,
            overall_percent,
        }
    }

    pub fn astap_install_completed(component: impl Into<String>, stage: Option<&InstallStage>) -> Self {
        let (stage, overall_percent) = stage_progress(stage, 0.0);
        Self::AstapInstallCompleted {
            component: component.into(),
            stage,
            overall_percent,
        }
    }

    pub fn catalog_install_progress(
        file_name: impl Into<String>,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
    ) -> Self {
        Self::CatalogInstallProgress {
            file_name: file_name.into(),
            bytes_downloaded,
            total_bytes,
            percent: percent_of(bytes_downloaded, total_bytes),
        }
    }
}
