//! The sync snapshot: what a `/ws/events` client is sent on connect and after a lag.

use super::AppState;
use crate::events::{ServerEvent, Topic};

impl AppState {
    /// Every [`Value`](crate::events::TopicKind::Value) topic's current state, as the events
    /// that replace a client's copy. Read after the client subscribed, so a change racing it
    /// is either in here or follows on the socket.
    pub fn sync_snapshot(&self) -> Vec<ServerEvent> {
        Topic::ALL.iter().flat_map(|&topic| self.topic_snapshot(topic)).collect()
    }

    fn topic_snapshot(&self, topic: Topic) -> Vec<ServerEvent> {
        match topic {
            Topic::CaptureState => vec![ServerEvent::state_changed(self.capture_state())],
            Topic::CaptureCounts => vec![self.stats.counts_event()],
            Topic::CameraPhases => vec![self.camera_phases_event()],
            Topic::CameraRecovery => self.recovery_status.events(),
            Topic::ViewedCamera => vec![self.viewed_camera.event()],
            Topic::StackCrop => vec![self.stream_crop.event()],
            Topic::DiskWriter => vec![self.disk_warning.event()],
            Topic::AstapInstall => vec![self.installs.event(super::Installer::Astap)],
            Topic::CatalogInstall => vec![self.installs.event(super::Installer::Catalog)],
            Topic::PushToTarget => vec![self.push_to_announcer.target_event()],
            Topic::PushToDirection => vec![self.push_to_announcer.direction_event()],
            Topic::PushToSolve => vec![self.push_to_announcer.solve_event()],
            Topic::PushToBlocker => vec![self.push_to_announcer.blocker_event()],
            Topic::Settings | Topic::AiCompute | Topic::CameraList => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::TopicKind;

    #[test]
    fn the_snapshot_restates_every_value_topic_and_no_refetched_one() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let snapshot = state.sync_snapshot();

        for &topic in Topic::ALL {
            let restated = snapshot.iter().any(|event| event.topic() == Some(topic));
            match topic.kind() {
                TopicKind::Value => assert!(restated, "{topic:?} is not restated"),
                TopicKind::Refetch => assert!(!restated, "{topic:?} is refetched, not sent"),
            }
        }
        assert!(snapshot.iter().all(|event| event.topic().is_some()), "{snapshot:?}");
    }

    /// A topic's snapshot speaks only in its own events, so restating one topic never
    /// overwrites another's state on the client.
    #[test]
    fn each_value_topic_is_restated_by_its_own_events_only() {
        let (state, _disk_writer) = AppState::new_for_testing();

        for &topic in Topic::ALL {
            for event in state.topic_snapshot(topic) {
                assert_eq!(event.topic(), Some(topic), "{topic:?} restated by {event:?}");
            }
        }
    }

    fn snapshot_of(state: &AppState, topic: Topic) -> Vec<serde_json::Value> {
        state
            .topic_snapshot(topic)
            .iter()
            .map(|event| serde_json::from_str(&event.to_json()).unwrap())
            .collect()
    }

    /// Nothing to report is still restated, so a client drops a stale count, warning or
    /// reconnect notice it holds.
    #[test]
    fn counts_recovery_and_the_disk_are_restated_when_there_is_nothing_to_report() {
        use serde_json::json;
        let (state, _disk_writer) = AppState::new_for_testing();

        assert_eq!(
            snapshot_of(&state, Topic::CaptureCounts),
            [json!({"type": "capture_counts", "frame_count": 0, "stacked_count": 0, "rejected_count": 0,
                    "dropped_count": 0, "delivered_count": 0, "last_rejection_reason": null})]
        );
        assert_eq!(
            snapshot_of(&state, Topic::CameraRecovery),
            [
                json!({"type": "camera_recovery_cleared", "role": "main"}),
                json!({"type": "camera_recovery_cleared", "role": "guide"}),
            ]
        );
        assert_eq!(snapshot_of(&state, Topic::DiskWriter), [json!({"type": "disk_writer_warning_cleared"})]);
    }

    /// By the time a client has an event, a sync restates what it said.
    #[test]
    fn each_change_is_recorded_before_its_event_goes_out() {
        use crate::state::{CameraRole, RecoveryStatus};
        use serde_json::json;
        let (state, _disk_writer) = AppState::new_for_testing();
        let mut events = state.subscribe_events();

        state.frame_delivered();
        state.frame_captured(false, true, Some("trailed stars"));
        state.frame_dropped();
        assert!(matches!(events.try_recv(), Ok(ServerEvent::FrameCaptured { .. })));
        assert!(matches!(events.try_recv(), Ok(ServerEvent::FrameDropped { .. })));
        assert_eq!(
            snapshot_of(&state, Topic::CaptureCounts),
            [json!({"type": "capture_counts", "frame_count": 1, "stacked_count": 0, "rejected_count": 1,
                    "dropped_count": 1, "delivered_count": 1, "last_rejection_reason": "trailed stars"})]
        );

        state.disk_warning.raise(40);
        assert!(matches!(events.try_recv(), Ok(ServerEvent::DiskWriterWarning { queue_depth: 40 })));
        assert_eq!(snapshot_of(&state, Topic::DiskWriter), [json!({"type": "disk_writer_warning", "queue_depth": 40})]);

        let failed = RecoveryStatus::Failed { name: "Ares".into(), attempts: 7, reason: "unplugged".into() };
        state.recovery_status.set(CameraRole::Guide, failed);
        assert!(matches!(events.try_recv(), Ok(ServerEvent::CameraReconnectFailed { .. })));
        assert_eq!(
            snapshot_of(&state, Topic::CameraRecovery)[1],
            json!({"type": "camera_reconnect_failed", "name": "Ares", "role": "guide", "attempts": 7, "reason": "unplugged"})
        );
    }
}
