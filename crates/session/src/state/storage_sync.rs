//! Whether what each camera's last capture saved is on the storage device yet — what an
//! observer waits for after Stop before powering off. 2026-10-16's Orange Pi lost power
//! seconds after one and kept 0-byte stack files: every write had succeeded into the OS's
//! cache. Per role, since each camera syncs when *it* stops and the other may run on.
//! Recorded before it is announced and restated on every sync, so a page opened after the
//! Stop, or one that lagged through it, still learns whether the save landed.

use std::sync::Mutex;

use night_amplifier_core::disk_writer::{DiskWriterHandle, SyncGroup};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::warn;

use super::CameraRole;
use crate::events::ServerEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StorageSyncState {
    /// Nothing to report: no capture saved anything yet, or one is running and saving.
    #[default]
    Idle,
    /// The capture ended; its files are being synced to the device.
    Syncing,
    /// Every file the capture saved is on the device.
    Synced,
    /// Some write or sync failed: those files may be missing or empty.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StorageSyncStatus {
    pub state: StorageSyncState,
    /// Files synced to the device.
    pub files: usize,
    /// Writes and syncs that failed.
    pub failed: usize,
    /// The first failure.
    pub error: Option<String>,
}

/// The writer's group for a role's files: the imaging camera writes through the handle's
/// shared session, the guide loop through sessions of its own.
fn group(role: CameraRole) -> SyncGroup {
    match role {
        CameraRole::Main => SyncGroup::Shared,
        CameraRole::Guide => SyncGroup::Owned,
    }
}

/// A role's status, and which run of its camera that status speaks for.
#[derive(Default)]
struct Entry {
    status: StorageSyncStatus,
    /// Bumped by every [`StorageSync::reset`]. The guide loop unregisters before its trailing
    /// sync, so a Start or a recovery's restart can begin a new run while that sync is still
    /// out; its late "saved" must not land over a camera saving again.
    run: u64,
}

pub struct StorageSync {
    entries: Mutex<[Entry; CameraRole::COUNT]>,
    events: broadcast::Sender<ServerEvent>,
}

impl StorageSync {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            entries: Mutex::new(Default::default()),
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, [Entry; CameraRole::COUNT]> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self, role: CameraRole) -> StorageSyncStatus {
        self.lock()[role as usize].status.clone()
    }

    /// Every role's status, as the sync snapshot restates it.
    pub fn events(&self) -> Vec<ServerEvent> {
        let entries = self.lock();
        CameraRole::all()
            .into_iter()
            .map(|role| ServerEvent::storage_sync(role, &entries[role as usize].status))
            .collect()
    }

    /// `role`'s camera is saving again: its last verdict, and any sync still out, no longer
    /// cover the disk.
    pub fn reset(&self, role: CameraRole) {
        let mut entries = self.lock();
        entries[role as usize].run += 1;
        self.announce(&mut entries[role as usize], StorageSyncStatus::default(), role);
    }

    /// Wait until everything `role`'s camera gave `writer` is on the device, announcing the
    /// wait and then its outcome — unless the camera started saving again meanwhile.
    /// Blocking: call it after that camera's last write was queued.
    pub fn sync(&self, role: CameraRole, writer: &DiskWriterHandle) {
        let run = {
            let mut entries = self.lock();
            let syncing = StorageSyncStatus { state: StorageSyncState::Syncing, ..Default::default() };
            self.announce(&mut entries[role as usize], syncing, role);
            entries[role as usize].run
        };
        let status = match writer.sync(group(role)) {
            Ok(report) if report.synced == 0 && report.failed == 0 => StorageSyncStatus::default(),
            Ok(report) => StorageSyncStatus {
                state: if report.failed == 0 { StorageSyncState::Synced } else { StorageSyncState::Failed },
                files: report.synced,
                failed: report.failed,
                error: report.error,
            },
            Err(e) => {
                warn!(role = role.label(), error = %e, "Could not sync the capture to disk");
                StorageSyncStatus { state: StorageSyncState::Failed, error: Some(e.to_string()), ..Default::default() }
            }
        };
        let mut entries = self.lock();
        if entries[role as usize].run != run {
            return;
        }
        self.announce(&mut entries[role as usize], status, role);
    }

    /// Sent from under the lock, so racing changes reach clients in the order they were
    /// made; an unchanged status announces nothing.
    fn announce(&self, entry: &mut Entry, status: StorageSyncStatus, role: CameraRole) {
        if entry.status == status {
            return;
        }
        entry.status = status;
        let _ = self.events.send(ServerEvent::storage_sync(role, &entry.status));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::disk_writer::{DiskWriter, DiskWriterConfig, WritingSessionType};
    use night_amplifier_core::fits::FitsMetadata;

    fn state_of(event: ServerEvent) -> StorageSyncState {
        match event {
            ServerEvent::StorageSync { state, .. } => state,
            other => panic!("not a storage_sync: {other:?}"),
        }
    }

    #[test]
    fn a_sync_announces_the_wait_then_the_files_it_settled() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());
        let (tx, mut rx) = broadcast::channel(8);
        let sync = StorageSync::new(tx);
        let frame = night_amplifier_core::frame::Frame::filled(4, 4, 3, 0.5).unwrap();
        handle.queue_stacked_frame(std::sync::Arc::new(frame), FitsMetadata::new()).unwrap();

        sync.sync(CameraRole::Main, &handle);

        assert_eq!(state_of(rx.try_recv().unwrap()), StorageSyncState::Syncing);
        assert_eq!(state_of(rx.try_recv().unwrap()), StorageSyncState::Synced);
        assert_eq!(
            sync.status(CameraRole::Main),
            StorageSyncStatus { state: StorageSyncState::Synced, files: 1, failed: 0, error: None }
        );
        let restated: Vec<_> = sync.events().into_iter().map(state_of).collect();
        assert_eq!(restated, [StorageSyncState::Synced, StorageSyncState::Idle], "main, then guide");

        sync.reset(CameraRole::Main);
        assert_eq!(state_of(rx.try_recv().unwrap()), StorageSyncState::Idle);
        sync.reset(CameraRole::Main);
        assert!(rx.try_recv().is_err(), "an unchanged status announces nothing");
        drop(handle);
        writer_task.join().unwrap();
    }

    /// A capture that saved nothing has nothing to promise; "safe" with 0 files is noise.
    #[test]
    fn a_capture_that_saved_nothing_settles_back_to_idle() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());
        let (tx, _rx) = broadcast::channel(8);
        let sync = StorageSync::new(tx);

        sync.sync(CameraRole::Main, &handle);

        assert_eq!(sync.status(CameraRole::Main), StorageSyncStatus::default());
        drop(handle);
        writer_task.join().unwrap();
    }

    #[test]
    fn a_failed_write_or_a_closed_writer_is_never_reported_as_safe() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());
        let (tx, _rx) = broadcast::channel(8);
        let sync = StorageSync::new(tx);
        let session = handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        std::fs::write(&session, b"").unwrap();
        let frame = std::sync::Arc::new(night_amplifier_core::camera::RawFrame {
            data: night_amplifier_core::camera::BufferPool::new().get(8 * 8 * 2),
            width: 8,
            height: 8,
            format: night_amplifier_core::camera::ImageFormat::Raw16,
        });
        handle.queue_raw_frame(frame, 1, FitsMetadata::new(), night_amplifier_core::camera::SensorType::Mono, None).unwrap();

        sync.sync(CameraRole::Main, &handle);
        let failed = sync.status(CameraRole::Main);
        drop(handle);
        writer_task.join().unwrap();
        let (_writer, closed) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        drop(_writer);
        sync.sync(CameraRole::Main, &closed);

        assert_eq!((failed.state, failed.failed), (StorageSyncState::Failed, 1));
        assert!(failed.error.is_some());
        assert_eq!(sync.status(CameraRole::Main).state, StorageSyncState::Failed);
    }

    /// The guide loop unregisters before its trailing sync, so a Start (or a recovery's
    /// restart) can reset the role while that sync is still running. Its late verdict must
    /// not announce "saved" over a loop that is saving again.
    #[test]
    fn a_sync_that_ends_after_the_camera_started_again_does_not_speak_for_the_new_run() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let (tx, _rx) = broadcast::channel(8);
        let sync = std::sync::Arc::new(StorageSync::new(tx));
        let guide = handle.create_session(WritingSessionType::IndividualFrames, "-guide").unwrap();
        let frame = std::sync::Arc::new(night_amplifier_core::camera::RawFrame {
            data: night_amplifier_core::camera::BufferPool::new().get(8 * 8 * 2),
            width: 8,
            height: 8,
            format: night_amplifier_core::camera::ImageFormat::Raw16,
        });
        let sensor = night_amplifier_core::camera::SensorType::Mono;
        handle.queue_raw_frame_in(Some(guide), frame, 1, FitsMetadata::new(), sensor, None).unwrap();

        // The writer is not running yet, so the first loop's trailing sync is still waiting.
        let stopping = {
            let (sync, handle) = (sync.clone(), handle.clone());
            std::thread::spawn(move || sync.sync(CameraRole::Guide, &handle))
        };
        while sync.status(CameraRole::Guide).state != StorageSyncState::Syncing {
            std::thread::yield_now();
        }
        sync.reset(CameraRole::Guide);
        let writer_task = std::thread::spawn(move || writer.run());
        stopping.join().unwrap();
        let after = sync.status(CameraRole::Guide);
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(after.state, StorageSyncState::Idle, "the new run was announced as {after:?}");
    }
}
