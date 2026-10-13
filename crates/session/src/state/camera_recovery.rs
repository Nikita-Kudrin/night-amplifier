//! What automatic recovery has told clients about each role's camera: reconnecting, given
//! up, or nothing. Recorded before its event goes out and restated on every sync: lost, the
//! end of a recovery left "Reconnecting" up for good, and a lost give-up hid the real error
//! behind it.

use std::sync::Mutex;

use tokio::sync::broadcast;

use super::CameraRole;
use crate::events::ServerEvent;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RecoveryStatus {
    #[default]
    None,
    Reconnecting {
        name: String,
        attempt: u32,
        of: u32,
        next_attempt_in_s: u64,
    },
    /// Stays until a camera connects into the role again: the camera is gone, and the
    /// observer has to act.
    Failed {
        name: String,
        attempts: u32,
        reason: String,
    },
}

impl RecoveryStatus {
    fn event(&self, role: CameraRole) -> ServerEvent {
        match self {
            RecoveryStatus::None => ServerEvent::CameraRecoveryCleared { role },
            RecoveryStatus::Reconnecting { name, attempt, of, next_attempt_in_s } => {
                ServerEvent::camera_reconnecting(name.clone(), role, *attempt, *of, *next_attempt_in_s)
            }
            RecoveryStatus::Failed { name, attempts, reason } => {
                ServerEvent::camera_reconnect_failed(name.clone(), role, *attempts, reason.clone())
            }
        }
    }
}

/// Each role's [`RecoveryStatus`], every change announced from under the lock so a clear
/// racing a supervisor's notice reaches clients in the order they were made.
pub struct CameraRecoveryStatus {
    statuses: Mutex<[RecoveryStatus; CameraRole::COUNT]>,
    events: broadcast::Sender<ServerEvent>,
}

impl CameraRecoveryStatus {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            statuses: Mutex::new(Default::default()),
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, [RecoveryStatus; CameraRole::COUNT]> {
        self.statuses.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, role: CameraRole) -> RecoveryStatus {
        self.lock()[role as usize].clone()
    }

    /// Every role's status, as the events that replace a client's copy.
    pub fn events(&self) -> Vec<ServerEvent> {
        let statuses = self.lock();
        CameraRole::all()
            .into_iter()
            .map(|role| statuses[role as usize].event(role))
            .collect()
    }

    /// Record `role`'s status and announce it; an unchanged one is not announced again.
    pub fn set(&self, role: CameraRole, status: RecoveryStatus) {
        let mut statuses = self.lock();
        if statuses[role as usize] == status {
            return;
        }
        statuses[role as usize] = status;
        let _ = self.events.send(statuses[role as usize].event(role));
    }

    pub fn clear(&self, role: CameraRole) {
        self.set(role, RecoveryStatus::None);
    }

    /// The observer dismissed `role`'s give-up: clear it for every client, as a connect into
    /// the role would. Only a give-up, checked under the lock: a recovery running meanwhile
    /// keeps its notice. Returns whether there was one.
    pub fn dismiss_failure(&self, role: CameraRole) -> bool {
        let mut statuses = self.lock();
        if !matches!(statuses[role as usize], RecoveryStatus::Failed { .. }) {
            return false;
        }
        statuses[role as usize] = RecoveryStatus::None;
        let _ = self.events.send(ServerEvent::CameraRecoveryCleared { role });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reconnecting(attempt: u32) -> RecoveryStatus {
        RecoveryStatus::Reconnecting { name: "Ares".into(), attempt, of: 9, next_attempt_in_s: 5 }
    }

    #[test]
    fn each_role_is_restated_and_none_is_a_clear() {
        let (tx, _rx) = broadcast::channel(8);
        let recovery = CameraRecoveryStatus::new(tx);
        recovery.set(CameraRole::Guide, reconnecting(2));

        let snapshot = recovery.events();
        assert!(matches!(snapshot[0], ServerEvent::CameraRecoveryCleared { role: CameraRole::Main }));
        assert!(matches!(
            snapshot[1],
            ServerEvent::CameraReconnecting { role: CameraRole::Guide, attempt: 2, of: 9, .. }
        ));
    }

    #[test]
    fn a_change_is_announced_once_and_a_clear_only_after_something_to_clear() {
        let (tx, mut rx) = broadcast::channel(8);
        let recovery = CameraRecoveryStatus::new(tx);

        recovery.clear(CameraRole::Main);
        assert!(rx.try_recv().is_err(), "nothing was being reported");

        recovery.set(CameraRole::Main, reconnecting(2));
        recovery.set(CameraRole::Main, reconnecting(2));
        recovery.set(
            CameraRole::Main,
            RecoveryStatus::Failed { name: "Ares".into(), attempts: 9, reason: "unplugged".into() },
        );
        recovery.clear(CameraRole::Main);

        let mut announced = Vec::new();
        while let Ok(event) = rx.try_recv() {
            announced.push(event);
        }
        assert!(matches!(
            announced.as_slice(),
            [
                ServerEvent::CameraReconnecting { attempt: 2, .. },
                ServerEvent::CameraReconnectFailed { role: CameraRole::Main, attempts: 9, .. },
                ServerEvent::CameraRecoveryCleared { role: CameraRole::Main },
            ]
        ));
        assert_eq!(recovery.get(CameraRole::Main), RecoveryStatus::None);
    }

    /// A give-up stays until a camera connects into the role, which an observer imaging on
    /// without the guide camera never does: dismissing it is what ends it for every client.
    #[test]
    fn a_dismissed_give_up_is_cleared_for_everyone_but_a_running_recovery_is_kept() {
        let (tx, mut rx) = broadcast::channel(8);
        let recovery = CameraRecoveryStatus::new(tx);
        recovery.set(CameraRole::Main, reconnecting(2));
        recovery.set(
            CameraRole::Guide,
            RecoveryStatus::Failed { name: "Ares".into(), attempts: 9, reason: "unplugged".into() },
        );
        while rx.try_recv().is_ok() {}

        assert!(!recovery.dismiss_failure(CameraRole::Main), "still reconnecting");
        assert!(recovery.dismiss_failure(CameraRole::Guide));
        assert!(!recovery.dismiss_failure(CameraRole::Guide), "already dismissed");

        assert_eq!(recovery.get(CameraRole::Main), reconnecting(2));
        assert_eq!(recovery.get(CameraRole::Guide), RecoveryStatus::None);
        assert!(matches!(rx.try_recv(), Ok(ServerEvent::CameraRecoveryCleared { role: CameraRole::Guide })));
        assert!(rx.try_recv().is_err());
    }
}
