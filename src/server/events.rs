//! Server events for WebSocket communication
//!
//! Events are serialized to JSON using serde with automatic snake_case naming.

use serde::Serialize;

use super::state::CaptureState;

/// Event types sent to WebSocket clients
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Capture state changed
    StateChanged { state: CaptureStateDto },

    /// New frame captured
    FrameCaptured {
        frame_number: u64,
        stacked_count: u64,
    },

    /// Frame was rejected
    FrameRejected {
        frame_number: u64,
        stacked_count: u64,
        reason: String,
    },

    /// Settings were updated
    SettingsUpdated,

    /// Camera connected
    CameraConnected { name: String },

    /// Camera disconnected
    CameraDisconnected { name: String },

    /// Error occurred
    Error { message: String },

    /// Disk writer queue warning (queue depth exceeds threshold)
    DiskWriterWarning { queue_depth: usize },

    /// Disk writer queue warning cleared
    DiskWriterWarningCleared,

    /// Warning message (e.g., client too slow)
    Warning { message: String },

    // ========================================================================
    // Push-To Navigation Events
    // ========================================================================
    /// Plate solving started
    PlateSolvingStarted { target_name: Option<String> },

    /// Position solved successfully
    PositionSolved {
        ra_degrees: f64,
        dec_degrees: f64,
        ra_string: String,
        dec_string: String,
        stars_matched: usize,
        confidence: f64,
        rotation_deg: f64,
    },

    /// Position solve failed
    PositionSolveFailed { reason: String },

    /// Push direction updated
    PushDirectionUpdated {
        angle_deg: f64,
        distance_deg: f64,
        direction_hint: String,
        is_close: bool,
        fov_deg: Option<f64>,
    },

    /// Target changed
    TargetChanged {
        designation: Option<String>,
        ra_degrees: f64,
        dec_degrees: f64,
    },

    /// Target cleared
    TargetCleared,

    // ========================================================================
    // ASTAP Installation Events
    // ========================================================================
    /// ASTAP installation starting
    AstapInstallStarting { component: String },

    /// ASTAP installation progress (downloading)
    AstapInstallProgress {
        component: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        /// Percentage of current operation (0-100)
        percent: Option<f32>,
        /// Current installation stage name
        stage: Option<String>,
        /// Overall installation progress (0-100)
        overall_percent: Option<f32>,
    },

    /// ASTAP installation extracting
    AstapInstallExtracting {
        component: String,
        /// Percentage of extraction (0-100)
        progress: f32,
        /// Current installation stage name
        stage: Option<String>,
        /// Overall installation progress (0-100)
        overall_percent: Option<f32>,
    },

    /// ASTAP installation completed successfully
    AstapInstallCompleted {
        component: String,
        /// Current installation stage name
        stage: Option<String>,
        /// Overall installation progress (0-100)
        overall_percent: Option<f32>,
    },

    /// ASTAP installation failed
    AstapInstallFailed { component: String, error: String },

    // ========================================================================
    // Catalog Installation Events
    // ========================================================================
    /// Catalog installation starting
    CatalogInstallStarting,

    /// Catalog download progress
    CatalogInstallProgress {
        file_name: String,
        bytes_downloaded: u64,
        total_bytes: Option<u64>,
        percent: Option<f32>,
    },

    /// Catalog file downloaded
    CatalogFileCompleted { file_name: String },

    /// Catalog installation completed
    CatalogInstallCompleted { object_count: usize },

    /// Catalog installation failed
    CatalogInstallFailed { error: String },
}

/// DTO for CaptureState serialization
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum CaptureStateDto {
    Idle,
    Starting,
    Capturing,
    Stopping,
    Error,
}

impl From<CaptureState> for CaptureStateDto {
    fn from(state: CaptureState) -> Self {
        match state {
            CaptureState::Idle => CaptureStateDto::Idle,
            CaptureState::Starting => CaptureStateDto::Starting,
            CaptureState::Capturing => CaptureStateDto::Capturing,
            CaptureState::Stopping => CaptureStateDto::Stopping,
            CaptureState::Error => CaptureStateDto::Error,
        }
    }
}

impl ServerEvent {
    pub fn state_changed(state: CaptureState) -> Self {
        ServerEvent::StateChanged {
            state: state.into(),
        }
    }

    pub fn frame_captured(frame_number: u64, stacked_count: u64) -> Self {
        ServerEvent::FrameCaptured {
            frame_number,
            stacked_count,
        }
    }

    pub fn frame_rejected(
        frame_number: u64,
        stacked_count: u64,
        reason: impl Into<String>,
    ) -> Self {
        ServerEvent::FrameRejected {
            frame_number,
            stacked_count,
            reason: reason.into(),
        }
    }

    pub fn camera_connected(name: impl Into<String>) -> Self {
        ServerEvent::CameraConnected { name: name.into() }
    }

    pub fn camera_disconnected(name: impl Into<String>) -> Self {
        ServerEvent::CameraDisconnected { name: name.into() }
    }

    pub fn error(message: impl Into<String>) -> Self {
        ServerEvent::Error {
            message: message.into(),
        }
    }

    pub fn disk_writer_warning(queue_depth: usize) -> Self {
        ServerEvent::DiskWriterWarning { queue_depth }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        ServerEvent::Warning {
            message: message.into(),
        }
    }

    pub fn plate_solving_started(target_name: Option<String>) -> Self {
        ServerEvent::PlateSolvingStarted { target_name }
    }

    pub fn position_solved(
        ra_degrees: f64,
        dec_degrees: f64,
        ra_string: impl Into<String>,
        dec_string: impl Into<String>,
        stars_matched: usize,
        confidence: f64,
        rotation_deg: f64,
    ) -> Self {
        ServerEvent::PositionSolved {
            ra_degrees,
            dec_degrees,
            ra_string: ra_string.into(),
            dec_string: dec_string.into(),
            stars_matched,
            confidence,
            rotation_deg,
        }
    }

    pub fn position_solve_failed(reason: impl Into<String>) -> Self {
        ServerEvent::PositionSolveFailed {
            reason: reason.into(),
        }
    }

    pub fn push_direction_updated(
        angle_deg: f64,
        distance_deg: f64,
        direction_hint: impl Into<String>,
        is_close: bool,
        fov_deg: Option<f64>,
    ) -> Self {
        ServerEvent::PushDirectionUpdated {
            angle_deg,
            distance_deg,
            direction_hint: direction_hint.into(),
            is_close,
            fov_deg,
        }
    }

    pub fn target_changed(designation: Option<String>, ra_degrees: f64, dec_degrees: f64) -> Self {
        ServerEvent::TargetChanged {
            designation,
            ra_degrees,
            dec_degrees,
        }
    }

    pub fn target_cleared() -> Self {
        ServerEvent::TargetCleared
    }

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

    /// Serialize to JSON string
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"type":"error","message":"Serialization failed"}"#.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changed_serialization() {
        let event = ServerEvent::state_changed(CaptureState::Capturing);
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "state_changed");
        assert_eq!(json["state"], "Capturing");
    }

    #[test]
    fn test_frame_captured_serialization() {
        let event = ServerEvent::frame_captured(42, 10);
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "frame_captured");
        assert_eq!(json["frame_number"], 42);
        assert_eq!(json["stacked_count"], 10);
    }

    #[test]
    fn test_frame_rejected_serialization() {
        let event = ServerEvent::frame_rejected(5, 3, "Bad alignment");
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "frame_rejected");
        assert_eq!(json["frame_number"], 5);
        assert_eq!(json["stacked_count"], 3);
        assert_eq!(json["reason"], "Bad alignment");
    }

    #[test]
    fn test_settings_updated_serialization() {
        let event = ServerEvent::SettingsUpdated;
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "settings_updated");
    }

    #[test]
    fn test_camera_connected_serialization() {
        let event = ServerEvent::camera_connected("Test Camera");
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "camera_connected");
        assert_eq!(json["name"], "Test Camera");
    }

    #[test]
    fn test_camera_disconnected_serialization() {
        let event = ServerEvent::camera_disconnected("Test Camera");
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "camera_disconnected");
        assert_eq!(json["name"], "Test Camera");
    }

    #[test]
    fn test_error_serialization() {
        let event = ServerEvent::error("Something went wrong");
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "error");
        assert_eq!(json["message"], "Something went wrong");
    }

    #[test]
    fn test_disk_writer_warning_serialization() {
        let event = ServerEvent::disk_writer_warning(7);
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "disk_writer_warning");
        assert_eq!(json["queue_depth"], 7);
    }

    #[test]
    fn test_disk_writer_warning_cleared_serialization() {
        let event = ServerEvent::DiskWriterWarningCleared;
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "disk_writer_warning_cleared");
    }

    #[test]
    fn test_warning_serialization() {
        let event = ServerEvent::warning("Dropped 5 events");
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();

        assert_eq!(json["type"], "warning");
        assert_eq!(json["message"], "Dropped 5 events");
    }
}
