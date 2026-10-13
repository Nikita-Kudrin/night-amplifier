//! How a `/ws/events` client stays in step: every [`ServerEvent`] is either a *topic* — state
//! the server can always restate, changed before its event is sent — or a *notice*, a
//! one-off whose loss is acceptable. A sync (on connect, after a lag) restates every topic:
//! a [`TopicKind::Value`] by its events (`AppState::sync_snapshot`), a [`TopicKind::Refetch`]
//! by the client re-reading its REST endpoint on [`ServerEvent::Synced`].

use serde::Serialize;

use super::ServerEvent;

/// How a sync restates a [`Topic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopicKind {
    /// The event carries the state; the sync snapshot sends it again.
    Value,
    /// The event only announces a change; the client re-reads a REST endpoint after a sync.
    Refetch,
}

/// Declares [`Topic`], its kinds and [`Topic::ALL`] from one list, so no topic can be left
/// out of `ALL`, which the sync snapshot iterates.
macro_rules! topics {
    ($($(#[$doc:meta])* $topic:ident => $kind:ident,)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Topic {
            $($(#[$doc])* $topic,)+
        }

        impl Topic {
            pub const ALL: &'static [Topic] = &[$(Topic::$topic,)+];

            pub fn kind(self) -> TopicKind {
                match self {
                    $(Topic::$topic => TopicKind::$kind,)+
                }
            }
        }
    };
}

topics! {
    CaptureState => Value,
    /// After `CaptureState`: the client zeroes its counts on `state_changed: Starting`.
    CaptureCounts => Value,
    CameraPhases => Value,
    /// After `CameraPhases`, which clears the warning of a camera it shows healthy.
    CameraRecovery => Value,
    ViewedCamera => Value,
    StackCrop => Value,
    DiskWriter => Value,
    /// Before the other Push-To topics: a client's `target_cleared` also clears those.
    PushToTarget => Value,
    PushToDirection => Value,
    PushToSolve => Value,
    PushToBlocker => Value,
    AstapInstall => Value,
    CatalogInstall => Value,
    /// `GET /api/settings`.
    Settings => Refetch,
    /// `GET /api/ai-compute`.
    AiCompute => Refetch,
    /// `GET /api/cameras`.
    CameraList => Refetch,
}

/// Why a client is being synced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SyncCause {
    Connect,
    /// The client fell behind the event channel and missed events.
    Lagged,
}

impl ServerEvent {
    pub fn synced(cause: SyncCause, dropped: u64) -> Self {
        ServerEvent::Synced { cause, dropped }
    }

    /// The topic this event states or announces; `None` for a notice. No wildcard arm: a new
    /// event does not compile until it is classified.
    pub fn topic(&self) -> Option<Topic> {
        use ServerEvent as E;
        match self {
            E::StateChanged { .. } => Some(Topic::CaptureState),
            E::CameraPhases { .. } | E::CameraPhaseChanged { .. } => Some(Topic::CameraPhases),
            E::ViewedCameraChanged { .. } => Some(Topic::ViewedCamera),
            E::StackCropChanged { .. } => Some(Topic::StackCrop),
            E::SettingsUpdated => Some(Topic::Settings),
            E::AiComputeChanged => Some(Topic::AiCompute),
            E::CameraConnected { .. } | E::CameraDisconnected { .. } | E::CameraDiscovered { .. } => {
                Some(Topic::CameraList)
            }

            E::AstapInstallStatus { .. }
            | E::AstapInstallStarting { .. }
            | E::AstapInstallProgress { .. }
            | E::AstapInstallExtracting { .. }
            | E::AstapInstallCompleted { .. }
            | E::AstapInstallFailed { .. } => Some(Topic::AstapInstall),
            E::CatalogInstallStatus { .. }
            | E::CatalogInstallStarting
            | E::CatalogInstallProgress { .. }
            | E::CatalogFileCompleted { .. }
            | E::CatalogInstallCompleted { .. }
            | E::CatalogInstallFailed { .. } => Some(Topic::CatalogInstall),

            E::FrameCaptured { .. }
            | E::FrameRejected { .. }
            | E::FrameDropped { .. }
            | E::CaptureCounts { .. } => Some(Topic::CaptureCounts),
            E::DiskWriterWarning { .. } | E::DiskWriterWarningCleared => Some(Topic::DiskWriter),
            // `capture_resumed` also drives a one-off notice, which a lost one goes without.
            E::CameraReconnecting { .. }
            | E::CameraReconnectFailed { .. }
            | E::CaptureResumed { .. }
            | E::CameraRecoveryCleared { .. } => Some(Topic::CameraRecovery),
            E::TargetChanged(_) | E::TargetCleared => Some(Topic::PushToTarget),
            E::PushDirectionUpdated(_) | E::PushDirectionCleared => Some(Topic::PushToDirection),
            E::PlateSolvingStatus(_)
            | E::PlateSolvingStarted { .. }
            | E::PlateSolvingProgress { .. }
            | E::PositionSolved { .. }
            | E::PositionSolveFailed { .. }
            | E::PlateSolvingCancelled
            | E::PlateSolvingRestarted { .. } => Some(Topic::PushToSolve),
            E::PushToBlocked { .. } => Some(Topic::PushToBlocker),

            E::Synced { .. }
            | E::Error { .. }
            | E::Warning { .. }
            | E::FocusModeLeft
            | E::CameraStatusUpdated { .. }
            | E::CameraPersistentlyUnresponsive { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CameraRole, CaptureState};

    #[test]
    fn synced_names_its_cause_and_what_was_dropped() {
        let json = |event: ServerEvent| serde_json::from_str::<serde_json::Value>(&event.to_json()).unwrap();

        assert_eq!(
            json(ServerEvent::synced(SyncCause::Connect, 0)),
            serde_json::json!({"type": "synced", "cause": "connect", "dropped": 0})
        );
        assert_eq!(
            json(ServerEvent::synced(SyncCause::Lagged, 144)),
            serde_json::json!({"type": "synced", "cause": "lagged", "dropped": 144})
        );
    }

    #[test]
    fn events_are_classified_by_the_state_they_carry() {
        let cases = [
            (ServerEvent::state_changed(CaptureState::Idle), Some(Topic::CaptureState)),
            (ServerEvent::CameraPhases { cameras: Vec::new() }, Some(Topic::CameraPhases)),
            (
                ServerEvent::camera_phase_changed("Cam", CameraRole::Main, crate::state::CameraPhase::Idle, None),
                Some(Topic::CameraPhases),
            ),
            (ServerEvent::ViewedCameraChanged { camera: CameraRole::Main }, Some(Topic::ViewedCamera)),
            (ServerEvent::SettingsUpdated, Some(Topic::Settings)),
            (ServerEvent::AiComputeChanged, Some(Topic::AiCompute)),
            (ServerEvent::camera_disconnected("Cam"), Some(Topic::CameraList)),
            (
                ServerEvent::AstapInstallStatus { status: crate::state::InstallStatus::Idle },
                Some(Topic::AstapInstall),
            ),
            (ServerEvent::CatalogInstallStarting, Some(Topic::CatalogInstall)),
            (ServerEvent::frame_dropped(1, 30), Some(Topic::CaptureCounts)),
            (ServerEvent::DiskWriterWarningCleared, Some(Topic::DiskWriter)),
            (ServerEvent::capture_resumed("Cam", 12), Some(Topic::CameraRecovery)),
            (ServerEvent::CameraRecoveryCleared { role: CameraRole::Guide }, Some(Topic::CameraRecovery)),
            (ServerEvent::camera_persistently_unresponsive("Cam", 3), None),
            (ServerEvent::FocusModeLeft, None),
            (ServerEvent::error("boom"), None),
            (ServerEvent::synced(SyncCause::Lagged, 3), None),
        ];
        for (event, topic) in cases {
            assert_eq!(event.topic(), topic, "{event:?}");
        }
    }

    #[test]
    fn value_topics_are_restated_by_events_and_the_rest_refetched() {
        let kinds: Vec<_> = Topic::ALL.iter().map(|topic| (*topic, topic.kind())).collect();

        assert!(kinds.contains(&(Topic::StackCrop, TopicKind::Value)));
        assert!(kinds.contains(&(Topic::Settings, TopicKind::Refetch)));
        assert!(kinds.contains(&(Topic::CameraList, TopicKind::Refetch)));
    }
}
