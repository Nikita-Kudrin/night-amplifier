//! Install-related ServerEvent constructors (ASTAP and catalog installation)

use super::ServerEvent;

impl ServerEvent {
    pub fn astap_install_starting(component: impl Into<String>) -> Self {
        ServerEvent::AstapInstallStarting {
            component: component.into(),
        }
    }

    pub fn astap_install_progress(
        component: impl Into<String>,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        stage: Option<&crate::push_to::InstallStage>,
    ) -> Self {
        let percent = total_bytes.map(|total| (bytes_downloaded as f32 / total as f32) * 100.0);

        // Calculate overall progress based on stage
        let (stage_name, overall_percent) = if let Some(s) = stage {
            let base = s.base_progress();
            let weight = s.weight();
            let stage_progress = percent.unwrap_or(0.0) / 100.0;
            let overall = base + (weight * stage_progress);
            (Some(s.display_name().to_string()), Some(overall))
        } else {
            (None, None)
        };

        ServerEvent::AstapInstallProgress {
            component: component.into(),
            bytes_downloaded,
            total_bytes,
            percent,
            stage: stage_name,
            overall_percent,
        }
    }

    pub fn astap_install_extracting(
        component: impl Into<String>,
        progress: f32,
        stage: Option<&crate::push_to::InstallStage>,
    ) -> Self {
        // Calculate overall progress based on stage
        let (stage_name, overall_percent) = if let Some(s) = stage {
            let base = s.base_progress();
            let weight = s.weight();
            let stage_progress = progress / 100.0;
            let overall = base + (weight * stage_progress);
            (Some(s.display_name().to_string()), Some(overall))
        } else {
            (None, None)
        };

        ServerEvent::AstapInstallExtracting {
            component: component.into(),
            progress,
            stage: stage_name,
            overall_percent,
        }
    }

    pub fn astap_install_completed(
        component: impl Into<String>,
        stage: Option<&crate::push_to::InstallStage>,
    ) -> Self {
        let (stage_name, overall_percent) = if let Some(s) = stage {
            (Some(s.display_name().to_string()), Some(s.base_progress()))
        } else {
            (None, None)
        };

        ServerEvent::AstapInstallCompleted {
            component: component.into(),
            stage: stage_name,
            overall_percent,
        }
    }

    pub fn astap_install_failed(component: impl Into<String>, error: impl Into<String>) -> Self {
        ServerEvent::AstapInstallFailed {
            component: component.into(),
            error: error.into(),
        }
    }

    pub fn catalog_install_starting() -> Self {
        ServerEvent::CatalogInstallStarting
    }

    pub fn catalog_install_progress(
        file_name: impl Into<String>,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
    ) -> Self {
        let percent = total_bytes.map(|total| (bytes_downloaded as f32 / total as f32) * 100.0);
        ServerEvent::CatalogInstallProgress {
            file_name: file_name.into(),
            bytes_downloaded,
            total_bytes,
            percent,
        }
    }

    pub fn catalog_file_completed(file_name: impl Into<String>) -> Self {
        ServerEvent::CatalogFileCompleted {
            file_name: file_name.into(),
        }
    }

    pub fn catalog_install_completed(object_count: usize) -> Self {
        ServerEvent::CatalogInstallCompleted { object_count }
    }

    pub fn catalog_install_failed(error: impl Into<String>) -> Self {
        ServerEvent::CatalogInstallFailed {
            error: error.into(),
        }
    }
}
