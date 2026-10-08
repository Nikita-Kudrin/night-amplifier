//! The connected cameras and everything known about each, under one lock.
//!
//! These were six collections on `AppState`, four behind tokio locks, so a disconnect
//! edited them one at a time with the lock order written only in comments, and the
//! capture threads `block_on`'d to read a phase. One `std` mutex, never held across an
//! `await`, makes each change a transaction — a camera's entry, selection, status and
//! phase leave together — and every read synchronous. Holding a camera per *role*
//! makes "one Main, one Guide" structural rather than a check.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{CameraPhase, CameraRole, CameraSlot};
use crate::camera::{CameraInfo, CameraStatus};
use crate::server::camera_health::RestartHistory;
use crate::telemetry::metrics as telemetry_metrics;

/// Connected camera information
#[derive(Debug, Clone)]
pub struct ConnectedCameraInfo {
    /// Camera ID
    pub id: String,
    /// Provider name
    pub provider: String,
    /// Provider index
    pub index: usize,
    /// Which position this camera occupies. At most one camera holds each role, and
    /// the role decides which slot owns its handle and which stream it feeds.
    pub role: CameraRole,
    /// Camera info
    pub info: CameraInfo,
}

/// Keyed by role as well as name: two bodies of one model, one per role, are two devices
/// on two cables, and must not share a fault streak or a restart record.
type CameraKey = (CameraRole, String);

#[derive(Default)]
struct Entries {
    connected: [Option<ConnectedCameraInfo>; CameraRole::COUNT],
    phases: [CameraPhase; CameraRole::COUNT],
    /// What the settings panel is editing: the imaging camera connected last.
    selected: Option<String>,
    /// Latest status sample per camera name, for cooled cameras.
    statuses: HashMap<String, CameraStatus>,
    /// Consecutive faults and when the streak was last extended. Every fault detector
    /// feeds this one streak — see `camera_health`.
    fault_streaks: HashMap<CameraKey, (u32, Instant)>,
    /// Whether restarting a stalled stream in place has lately worked. Outlives the
    /// capture loop, whose next reopen it decides on.
    restart_histories: HashMap<CameraKey, RestartHistory>,
}

impl Entries {
    fn count(&self) -> usize {
        self.connected.iter().flatten().count()
    }

    fn by_id(&self, camera_id: &str) -> Option<&ConnectedCameraInfo> {
        self.connected
            .iter()
            .flatten()
            .find(|info| info.id == camera_id)
    }
}

/// The rig: who is connected in which role, and each slot's handle and monitor.
#[derive(Default)]
pub struct CameraRoster {
    slots: [CameraSlot; CameraRole::COUNT],
    entries: Mutex<Entries>,
}

impl CameraRoster {
    fn lock(&self) -> MutexGuard<'_, Entries> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The slot owning `role`'s handle, monitor and reconnect guard.
    pub fn slot(&self, role: CameraRole) -> &CameraSlot {
        &self.slots[role as usize]
    }

    /// Register `camera` in its role, returning the entry it displaced. Selects it when
    /// `select` — a recovered camera keeps a selection it never lost.
    ///
    /// A displaced entry under another id means `vacate_role` was skipped. One id in both
    /// roles is refused earlier, by `connect`'s check under `camera_connect_lock`.
    pub fn install(
        &self,
        camera: ConnectedCameraInfo,
        select: bool,
    ) -> Option<ConnectedCameraInfo> {
        let mut entries = self.lock();
        debug_assert!(
            entries.connected[camera.role.other() as usize]
                .as_ref()
                .is_none_or(|other| other.id != camera.id),
            "{} is already the {} camera",
            camera.id,
            camera.role.other().label()
        );
        if select {
            entries.selected = Some(camera.id.clone());
        }
        let role = camera.role as usize;
        let displaced = entries.connected[role].replace(camera);
        telemetry_metrics::record_cameras_count(entries.count() as u64);
        displaced
    }

    /// Unregister `camera_name` from `role`, with its selection and status, and mark the
    /// role `Disconnected` once it is empty.
    ///
    /// A role another camera has taken meanwhile keeps that camera and its phase.
    pub fn remove(&self, role: CameraRole, camera_name: &str) -> Option<ConnectedCameraInfo> {
        let mut entries = self.lock();
        let removed =
            entries.connected[role as usize].take_if(|info| info.info.name == camera_name);
        if removed
            .as_ref()
            .is_some_and(|info| entries.selected.as_ref() == Some(&info.id))
        {
            entries.selected = None;
        }
        entries.statuses.remove(camera_name);
        if entries.connected[role as usize].is_none() {
            entries.phases[role as usize] = CameraPhase::Disconnected;
        }
        telemetry_metrics::record_cameras_count(entries.count() as u64);
        removed
    }

    /// The camera occupying `role`, if any.
    pub fn in_role(&self, role: CameraRole) -> Option<ConnectedCameraInfo> {
        self.lock().connected[role as usize].clone()
    }

    /// A connected camera, by id.
    pub fn get(&self, camera_id: &str) -> Option<ConnectedCameraInfo> {
        self.lock().by_id(camera_id).cloned()
    }

    pub fn contains(&self, camera_id: &str) -> bool {
        self.lock().by_id(camera_id).is_some()
    }

    /// Every connected camera, imaging camera first.
    pub fn connected(&self) -> Vec<ConnectedCameraInfo> {
        self.lock().connected.iter().flatten().cloned().collect()
    }

    /// Every connected camera with its phase, read together.
    pub fn connected_with_phases(&self) -> Vec<(ConnectedCameraInfo, CameraPhase)> {
        let entries = self.lock();
        CameraRole::all()
            .into_iter()
            .filter_map(|role| {
                let info = entries.connected[role as usize].clone()?;
                Some((info, entries.phases[role as usize]))
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lock().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn selected(&self) -> Option<String> {
        self.lock().selected.clone()
    }

    /// The lifecycle phase of `role`'s camera; `Disconnected` when the slot is empty.
    pub fn phase(&self, role: CameraRole) -> CameraPhase {
        self.lock().phases[role as usize]
    }

    pub fn set_phase(&self, role: CameraRole, phase: CameraPhase) {
        self.lock().phases[role as usize] = phase;
    }

    /// Move `role` from `from` to `to`; `false`, changing nothing, when it has left `from`.
    ///
    /// For a decision taken on an earlier read: a capture starting meanwhile must not be
    /// overwritten by the monitor's settle or a cooler edit's return to `Precooling`.
    pub fn transition(&self, role: CameraRole, from: CameraPhase, to: CameraPhase) -> bool {
        let mut entries = self.lock();
        let phase = &mut entries.phases[role as usize];
        if *phase != from {
            return false;
        }
        *phase = to;
        true
    }

    pub fn status(&self, camera_name: &str) -> Option<CameraStatus> {
        self.lock().statuses.get(camera_name).cloned()
    }

    pub fn record_status(&self, camera_name: &str, status: CameraStatus) {
        self.lock().statuses.insert(camera_name.to_string(), status);
    }

    /// Extend a camera's fault streak and return its new length. A streak older than
    /// `ttl` has expired and restarts at 1.
    pub fn bump_fault_streak(
        &self,
        role: CameraRole,
        camera_name: &str,
        ttl: Duration,
        now: Instant,
    ) -> u32 {
        let mut entries = self.lock();
        let entry = entries
            .fault_streaks
            .entry((role, camera_name.to_string()))
            .or_insert((0, now));
        if now.duration_since(entry.1) > ttl {
            entry.0 = 0;
        }
        entry.0 += 1;
        entry.1 = now;
        entry.0
    }

    pub fn clear_fault_streak(&self, role: CameraRole, camera_name: &str) {
        self.lock()
            .fault_streaks
            .remove(&(role, camera_name.to_string()));
    }

    /// The streak's length and when it was last extended, if one is recorded.
    pub fn fault_streak(&self, role: CameraRole, camera_name: &str) -> Option<(u32, Instant)> {
        self.lock()
            .fault_streaks
            .get(&(role, camera_name.to_string()))
            .copied()
    }

    pub(crate) fn restart_history(
        &self,
        role: CameraRole,
        camera_name: &str,
    ) -> Option<RestartHistory> {
        self.lock()
            .restart_histories
            .get(&(role, camera_name.to_string()))
            .copied()
    }

    pub(crate) fn restart_failed(&self, role: CameraRole, camera_name: &str, now: Instant) {
        self.lock()
            .restart_histories
            .entry((role, camera_name.to_string()))
            .or_default()
            .record_failure(now);
    }

    pub(crate) fn forget_restart_history(&self, role: CameraRole, camera_name: &str) {
        self.lock()
            .restart_histories
            .remove(&(role, camera_name.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(id: &str, name: &str, role: CameraRole) -> ConnectedCameraInfo {
        ConnectedCameraInfo {
            id: id.to_string(),
            provider: "mock".to_string(),
            index: 0,
            role,
            info: CameraInfo {
                name: name.to_string(),
                ..Default::default()
            },
        }
    }

    /// A role holds one camera: a second install into it hands back the first.
    #[test]
    fn a_role_holds_one_camera() {
        let roster = CameraRoster::default();
        assert!(roster
            .install(camera("mock_0", "Ares", CameraRole::Main), true)
            .is_none());
        roster.install(camera("mock_1", "Neptune", CameraRole::Guide), false);

        let displaced = roster.install(camera("mock_2", "Uranus", CameraRole::Main), true);

        assert_eq!(displaced.map(|c| c.id).as_deref(), Some("mock_0"));
        assert_eq!(roster.len(), 2);
        assert_eq!(roster.selected().as_deref(), Some("mock_2"));
        let ids: Vec<_> = roster.connected().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["mock_2", "mock_1"], "imaging camera first");
    }

    /// Entry, selection, status and phase leave in one transaction.
    #[test]
    fn removing_a_camera_takes_its_selection_status_and_phase() {
        let roster = CameraRoster::default();
        roster.install(camera("mock_0", "Ares", CameraRole::Main), true);
        roster.set_phase(CameraRole::Main, CameraPhase::WarmingUp);
        roster.record_status("Ares", CameraStatus::default());

        let removed = roster.remove(CameraRole::Main, "Ares");

        assert_eq!(removed.map(|c| c.id).as_deref(), Some("mock_0"));
        assert!(roster.is_empty());
        assert_eq!(roster.selected(), None);
        assert!(roster.status("Ares").is_none());
        assert_eq!(roster.phase(CameraRole::Main), CameraPhase::Disconnected);
    }

    /// A late report about a camera the role no longer holds leaves its successor alone.
    #[test]
    fn removing_a_departed_camera_keeps_its_successor() {
        let roster = CameraRoster::default();
        roster.install(camera("mock_1", "Uranus", CameraRole::Main), true);
        roster.set_phase(CameraRole::Main, CameraPhase::Capturing);

        assert!(roster.remove(CameraRole::Main, "Ares").is_none());

        assert!(roster.contains("mock_1"));
        assert_eq!(roster.selected().as_deref(), Some("mock_1"));
        assert_eq!(roster.phase(CameraRole::Main), CameraPhase::Capturing);
    }

    #[test]
    fn a_transition_from_a_phase_already_left_changes_nothing() {
        let roster = CameraRoster::default();
        roster.set_phase(CameraRole::Main, CameraPhase::Capturing);

        assert!(!roster.transition(CameraRole::Main, CameraPhase::Precooling, CameraPhase::Idle));
        assert_eq!(roster.phase(CameraRole::Main), CameraPhase::Capturing);

        assert!(roster.transition(CameraRole::Main, CameraPhase::Capturing, CameraPhase::Idle));
        assert_eq!(roster.phase(CameraRole::Main), CameraPhase::Idle);
        assert_eq!(roster.phase(CameraRole::Guide), CameraPhase::Disconnected);
    }

    #[test]
    fn a_fault_streak_expires_after_its_ttl_and_is_per_role() {
        let roster = CameraRoster::default();
        let ttl = Duration::from_secs(60);
        let start = Instant::now();

        assert_eq!(
            roster.bump_fault_streak(CameraRole::Main, "Ares", ttl, start),
            1
        );
        assert_eq!(
            roster.bump_fault_streak(CameraRole::Main, "Ares", ttl, start + ttl),
            2
        );
        assert_eq!(
            roster.bump_fault_streak(CameraRole::Guide, "Ares", ttl, start + ttl),
            1
        );
        let late = start + ttl * 2 + Duration::from_secs(1);
        assert_eq!(
            roster.bump_fault_streak(CameraRole::Main, "Ares", ttl, late),
            1
        );

        roster.clear_fault_streak(CameraRole::Main, "Ares");
        assert!(roster.fault_streak(CameraRole::Main, "Ares").is_none());
        assert!(roster.fault_streak(CameraRole::Guide, "Ares").is_some());
    }
}
