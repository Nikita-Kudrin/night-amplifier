//! One sample of every [`ServerEvent`] variant. `tests/api_types.rs` classifies each by its
//! topic for the frontend's event catalogue, and fails while a variant of the schema has no
//! sample here, so a new event cannot reach the wire unclassified on the client.

use std::time::Duration;

use super::{
    CameraInfoResponse, CameraListEntry, CameraPhaseEntry, PushDirection, PushToTarget, ServerEvent,
    SolveStage, SolveStatus, StreamCrop, StreamFrameSize, SyncCause,
};
use crate::state::{
    CameraPhase, CameraRole, CaptureState, FrameCounts, InstallStatus, StorageSyncState, StorageSyncStatus,
};
use night_amplifier_core::push_to::{DirectionHint, InstallStage};

const WARMUP: Duration = Duration::from_secs(300);

fn camera() -> CameraListEntry {
    CameraListEntry {
        id: "zwo_0".into(),
        name: "ASI533MC Pro".into(),
        connected: false,
        provider: Some("ZWO".into()),
        index: Some(0),
        role: None,
        phase: None,
        warmup_remaining_s: None,
        info: CameraInfoResponse {
            id: "zwo_0".into(),
            name: "ASI533MC Pro".into(),
            max_width: 3008,
            max_height: 3008,
            pixel_size_x_um: 3.76,
            pixel_size_y_um: 3.76,
            sensor_type: night_amplifier_core::camera::SensorType::Color,
            has_cooler: true,
            has_dew_heater: true,
            min_temp_c: Some(-30.0),
            max_temp_c: Some(30.0),
            bit_depth: 14,
            min_exposure_us: 32,
            max_exposure_us: 2_000_000_000,
            min_gain: 0,
            max_gain: 450,
            sensor_modes: Vec::new(),
        },
    }
}

impl ServerEvent {
    pub fn examples() -> Vec<ServerEvent> {
        use ServerEvent as E;
        let crop = StreamCrop { x: 8, y: 8, width: 2992, height: 2992, source_width: 3008, source_height: 3008 };
        let progress = E::CatalogInstallProgress {
            file_name: "d50.1476".into(),
            bytes_downloaded: 1024,
            total_bytes: Some(4096),
            percent: Some(25.0),
        };
        vec![
            E::state_changed(CaptureState::Capturing),
            E::frame_captured(42, 40, 2, Some("trailed stars")),
            E::frame_rejected(43, 40, 3, "camera timeout"),
            E::SettingsUpdated,
            E::camera_connected("ASI533MC Pro"),
            E::camera_disconnected("ASI533MC Pro"),
            E::camera_discovered(camera()),
            E::camera_status_updated("ASI533MC Pro", -9.8, Some(42.0), true, false, Some(-10.0)),
            E::camera_phase_changed("ASI533MC Pro", CameraRole::Main, CameraPhase::WarmingUp, Some(WARMUP)),
            E::CameraPhases {
                cameras: vec![CameraPhaseEntry {
                    name: "ASI533MC Pro".into(),
                    role: CameraRole::Main,
                    phase: CameraPhase::Capturing.into(),
                    warmup_remaining_s: None,
                }],
            },
            E::ViewedCameraChanged { camera: CameraRole::Guide },
            E::StackCropChanged { crop: Some(crop), frame: Some(StreamFrameSize { width: 1496, height: 1496 }) },
            E::camera_persistently_unresponsive("ASI533MC Pro", 3),
            E::camera_reconnecting("ASI533MC Pro", CameraRole::Main, 1, 5, 2),
            E::camera_reconnect_failed("ASI533MC Pro", CameraRole::Main, 5, "unplugged"),
            E::capture_resumed("ASI533MC Pro", 40),
            E::CameraRecoveryCleared { role: CameraRole::Main },
            E::FocusModeLeft,
            E::AiComputeChanged,
            E::error("Camera lost"),
            E::disk_writer_warning(40),
            E::DiskWriterWarningCleared,
            E::storage_sync(CameraRole::Main, &StorageSyncStatus { state: StorageSyncState::Synced, files: 166, failed: 0, error: None }),
            E::frame_dropped(3, 120),
            E::capture_counts(
                FrameCounts { frames: 43, stacked: 40, rejected: 3, delivered: 120, dropped: 3 },
                Some("trailed stars".into()),
            ),
            E::warning("Disk nearly full"),
            E::synced(SyncCause::Lagged, 144),
            E::PlateSolvingStarted { target_name: Some("M 31".into()) },
            E::PlateSolvingProgress { stage: "blind".into(), attempt: 2, total: 3 },
            E::position_solved(10.68, 41.27, "00h 42m 44s", "+41° 16′ 09″", Some(812), 0.98, 12.5),
            E::position_solve_failed("too few stars"),
            E::plate_solving_cancelled(),
            E::plate_solving_restarted("binning changed"),
            E::push_to_blocked(Some("no solver installed".into())),
            E::PushDirectionUpdated(PushDirection {
                angle_deg: 37.0,
                distance_deg: 2.6,
                direction_hint: DirectionHint::NorthEast,
                is_close: false,
                fov_deg: Some(1.45),
                solved_camera: Some("ASI533MC Pro".into()),
                stale: false,
            }),
            E::PushDirectionCleared,
            E::PlateSolvingStatus(SolveStatus {
                in_progress: true,
                target_name: Some("M 31".into()),
                stage: Some(SolveStage { stage: "blind".into(), attempt: 2, total: 3 }),
                last_result: None,
            }),
            E::TargetChanged(PushToTarget {
                name: Some("Andromeda Galaxy".into()),
                designation: Some("M 31".into()),
                ra_degrees: 10.68,
                dec_degrees: 41.27,
            }),
            E::TargetCleared,
            E::AstapInstallStarting { component: "astap".into() },
            E::AstapInstallProgress {
                component: "astap".into(),
                bytes_downloaded: 1024,
                total_bytes: Some(4096),
                percent: Some(25.0),
                stage_id: Some(InstallStage::DownloadingCli),
                overall_percent: Some(10.0),
            },
            E::AstapInstallExtracting {
                component: "astap".into(),
                progress: 50.0,
                stage_id: Some(InstallStage::ExtractingCli),
                overall_percent: Some(60.0),
            },
            E::AstapInstallCompleted {
                component: "astap".into(),
                stage_id: None,
                overall_percent: Some(100.0),
            },
            E::AstapInstallFailed { component: "astap".into(), error: "disk full".into() },
            E::CatalogInstallStarting,
            progress.clone(),
            E::CatalogFileCompleted { file_name: "d50.1476".into() },
            E::CatalogInstallCompleted { object_count: 13_000 },
            E::CatalogInstallFailed { error: "disk full".into() },
            E::AstapInstallStatus { status: InstallStatus::Completed },
            E::CatalogInstallStatus { status: InstallStatus::Running { progress: Some(Box::new(progress)) } },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Topic;

    fn tag(event: &ServerEvent) -> String {
        let json: serde_json::Value = serde_json::from_str(&event.to_json()).unwrap();
        json["type"].as_str().unwrap().to_owned()
    }

    #[test]
    fn every_example_is_its_own_variant() {
        let tags: Vec<_> = ServerEvent::examples().iter().map(tag).collect();
        let unique: std::collections::BTreeSet<_> = tags.iter().collect();

        assert_eq!(unique.len(), tags.len(), "{tags:?}");
    }

    #[test]
    fn every_topic_is_stated_by_some_event() {
        let examples = ServerEvent::examples();

        for &topic in Topic::ALL {
            assert!(examples.iter().any(|event| event.topic() == Some(topic)), "no event states {topic:?}");
        }
    }
}
