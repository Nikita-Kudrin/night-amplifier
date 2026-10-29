//! What the writer has written but not yet seen reach the storage device, and the sync that
//! settles it — see [`super::DiskWriterHandle::sync`].
//!
//! Files are synced when a capture ends, not as each is written: the kernel writes most of
//! them out in the background meanwhile, so the sync mostly finds clean pages, and a slow
//! SD card never makes the writer fall behind the camera (a backed-up queue drops frames).
//! The fsyncs run on a thread of their own: on the writer's, the guide camera's stop held
//! the imaging camera's frames for as long as its sync took, and its queue dropped subs.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use tracing::{error, info};

use crate::durable;

/// Whose writes a sync settles: the capture writing through the handle's shared session
/// (the imaging camera, its stack included), or the producer owning its sessions (the guide
/// loop). Each is synced when *it* stops, so ending one neither waits on nor reports the
/// files of the other, which may still be running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncGroup {
    #[default]
    Shared,
    Owned,
}

impl SyncGroup {
    pub(crate) const COUNT: usize = 2;

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Shared => 0,
            Self::Owned => 1,
        }
    }
}

/// What a sync settled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Files written since the previous sync, now on the device.
    pub synced: usize,
    /// Writes and syncs that failed since then: a file named in one may be missing or empty.
    pub failed: usize,
    /// The first failure, for the observer.
    pub error: Option<String>,
}

impl SyncReport {
    fn fail(&mut self, error: String) {
        self.failed += 1;
        self.error.get_or_insert(error);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncTarget<'a> {
    File(&'a Path),
    Dir(&'a Path),
}

#[derive(Debug, Default)]
pub(crate) struct PendingSync {
    files: BTreeSet<PathBuf>,
    failures: SyncReport,
}

impl PendingSync {
    pub(crate) fn written(&mut self, path: &Path) {
        self.files.insert(path.to_path_buf());
    }

    pub(crate) fn failed(&mut self, error: impl ToString) {
        self.failures.fail(error.to_string());
    }

    /// Each step bumps `activity`, so a caller waiting on a long sync sees it progress.
    fn sync(&mut self, activity: &AtomicU64) -> SyncReport {
        self.sync_with(|target| {
            let synced = match target {
                SyncTarget::File(path) => durable::sync_file(path),
                SyncTarget::Dir(path) => durable::sync_dir(path),
            };
            activity.fetch_add(1, Ordering::SeqCst);
            synced
        })
    }

    /// Every file, then every directory holding a new name: the file's own, and the one
    /// above it, where a session folder created with its first frame is named.
    fn sync_with(&mut self, mut sync: impl FnMut(SyncTarget) -> io::Result<()>) -> SyncReport {
        let files = std::mem::take(&mut self.files);
        let mut report = std::mem::take(&mut self.failures);
        let mut dirs = BTreeSet::new();
        for file in &files {
            match sync(SyncTarget::File(file)) {
                Ok(()) => report.synced += 1,
                Err(e) => report.fail(format!("{}: {e}", file.display())),
            }
            let dir = durable::parent_dir(file);
            dirs.insert(dir);
            dirs.insert(durable::parent_dir(dir));
        }
        for dir in dirs {
            if let Err(e) = sync(SyncTarget::Dir(dir)) {
                report.fail(format!("{}: {e}", dir.display()));
            }
        }
        report
    }
}

/// What one group wrote since its last sync, handed by the writer to the sync thread.
pub(crate) struct SyncJob {
    pub(crate) group: SyncGroup,
    pub(crate) pending: PendingSync,
    pub(crate) reply: mpsc::Sender<SyncReport>,
}

impl SyncJob {
    pub(crate) fn run(mut self, activity: &AtomicU64) {
        let started = Instant::now();
        let report = self.pending.sync(activity);
        let (group, elapsed_ms) = (self.group, started.elapsed().as_millis() as u64);
        match &report.error {
            None => info!(?group, files = report.synced, elapsed_ms, "Captures synced to disk"),
            Some(error) => error!(?group, files = report.synced, failed = report.failed, %error, elapsed_ms, "Captures not all on disk"),
        }
        let _ = self.reply.send(report);
    }
}

/// The thread that runs [`SyncJob`]s in turn, ending with the writer that feeds it. Never
/// joined: one stuck in a hung device must not keep the writer from shutting down. If it
/// cannot be started, sends to it fail and the writer runs the job itself.
pub(crate) fn spawn_syncer(activity: Arc<AtomicU64>) -> mpsc::Sender<SyncJob> {
    let (jobs, queue) = mpsc::channel::<SyncJob>();
    let spawned = std::thread::Builder::new()
        .name("disk-sync".into())
        .spawn(move || queue.into_iter().for_each(|job| job.run(&activity)));
    if let Err(e) = spawned {
        error!(error = %e, "Could not start the disk sync thread; the writer syncs itself");
    }
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_writer::{DiskWriter, DiskWriterConfig, DiskWriterError, WritingSessionType, STALL_PATIENCE};
    use crate::fits::FitsMetadata;
    use std::time::Duration;

    #[test]
    fn every_file_is_synced_before_the_directories_that_name_it() {
        let mut pending = PendingSync::default();
        let frame = Path::new("captures/raw/m57-stacking/frame_000001.fits");
        let stack = Path::new("captures/stacked/m57-stacking.fits");
        pending.written(frame);
        pending.written(stack);
        pending.written(frame);

        let mut calls = Vec::new();
        let report = pending.sync_with(|target| {
            calls.push(match target {
                SyncTarget::File(p) => format!("file {}", p.display()),
                SyncTarget::Dir(p) => format!("dir {}", p.display()),
            });
            Ok(())
        });

        assert_eq!(report, SyncReport { synced: 2, failed: 0, error: None });
        assert_eq!(
            calls,
            [
                "file captures/raw/m57-stacking/frame_000001.fits",
                "file captures/stacked/m57-stacking.fits",
                "dir captures",
                "dir captures/raw",
                "dir captures/raw/m57-stacking",
                "dir captures/stacked",
            ]
        );
    }

    #[test]
    fn a_sync_reports_what_failed_since_the_last_one_and_starts_over() {
        let mut pending = PendingSync::default();
        pending.failed("Failed to write file: disk full");
        pending.written(Path::new("raw/s/frame_000001.fits"));
        pending.written(Path::new("raw/s/frame_000002.fits"));

        let report = pending.sync_with(|target| match target {
            SyncTarget::File(p) if p.ends_with("frame_000002.fits") => Err(io::Error::other("EIO")),
            _ => Ok(()),
        });

        assert_eq!(report.synced, 1);
        assert_eq!(report.failed, 2);
        assert_eq!(report.error.as_deref(), Some("Failed to write file: disk full"));
        assert_eq!(pending.sync_with(|_| Ok(())), SyncReport::default());
    }

    fn tiny_raw_frame() -> Arc<crate::camera::RawFrame> {
        Arc::new(crate::camera::RawFrame {
            data: crate::camera::BufferPool::new().get(32 * 32 * 2),
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        })
    }

    fn queue_raw(handle: &crate::disk_writer::DiskWriterHandle, number: u64) -> bool {
        handle
            .queue_raw_frame(tiny_raw_frame(), number, FitsMetadata::new(), crate::camera::SensorType::Mono, None)
            .unwrap()
    }

    /// The end of a capture as `run_capture_loop` drives it: frames, the final stack, the
    /// session's end, then a sync that answers only once every one of them was synced.
    #[test]
    fn a_sync_after_a_session_covers_its_frames_and_its_stack() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        let session = handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        assert!(queue_raw(&handle, 1));
        assert!(queue_raw(&handle, 2));
        let stack = crate::frame::Frame::filled(16, 16, 3, 0.5).unwrap();
        handle.queue_stacked_frame(Arc::new(stack), FitsMetadata::new()).unwrap();
        handle.queue_stacked_png(Arc::new(vec![128; 16 * 16 * 3]), 16, 16, 2).unwrap();
        handle.end_session();

        let report = handle.sync(SyncGroup::Shared).unwrap();
        let again = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report, SyncReport { synced: 4, failed: 0, error: None });
        assert_eq!(again, SyncReport::default(), "a second sync has nothing new to settle");
        let name = session.file_name().unwrap().to_string_lossy().to_string();
        for path in [
            session.join("frame_000001.fits"),
            session.join("frame_000002.fits"),
            dir.path().join("stacked").join(format!("{name}.fits")),
            dir.path().join("stacked").join(format!("{name}_stretched.png")),
        ] {
            assert!(std::fs::metadata(&path).unwrap().len() > 0, "{path:?} is empty");
        }
    }

    /// A planetary capture's container is one file however many frames it holds, and is
    /// synced complete: `EndSession` finalized its header before the sync came.
    #[test]
    fn a_sync_after_a_video_session_covers_the_finalized_container() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        let session = handle.start_session(WritingSessionType::VideoContainer, "-planetary").unwrap();
        for n in 1..=3 {
            assert!(queue_raw(&handle, n));
        }
        handle.end_session();
        let report = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report, SyncReport { synced: 1, failed: 0, error: None });
        let reader = crate::ser::SerReader::open(session.join("capture.ser")).unwrap();
        assert_eq!(reader.header().frame_count, 3);
    }

    /// The storage task finishes the container when its frames end, so its header counts
    /// them while the session stays open for a stack that may come minutes later; the end
    /// of the session leaves it as it was.
    #[test]
    fn a_finished_container_counts_its_frames_with_the_session_still_open() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        let session = handle.start_session(WritingSessionType::VideoContainer, "-planetary").unwrap();
        for n in 1..=3 {
            assert!(queue_raw(&handle, n));
        }
        handle.finish_container(SyncGroup::Owned);
        let after_other_group = handle.sync(SyncGroup::Shared).unwrap();
        let open_count = crate::ser::SerReader::open(session.join("capture.ser")).unwrap().header().frame_count;
        handle.finish_container(SyncGroup::Shared);
        let _ = handle.sync(SyncGroup::Shared).unwrap();
        let finished = std::fs::read(session.join("capture.ser")).unwrap();
        assert_eq!(handle.session_dir().as_deref(), Some(session.as_path()), "the session stays open");
        handle.end_session();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(after_other_group.failed, 0);
        assert_eq!(open_count, 3, "a sync of the open container counts its frames too");
        assert_eq!(u32::from_le_bytes(finished[38..42].try_into().unwrap()), 3);
        assert_eq!(std::fs::read(session.join("capture.ser")).unwrap(), finished, "ending the session rewrote it");
    }

    /// A container still open when a sync comes has its buffered frames pushed out first:
    /// synced from under a `BufWriter`, the file on disk held the header alone.
    #[test]
    fn a_sync_covers_the_frames_buffered_in_an_open_container() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        let session = handle.start_session(WritingSessionType::VideoContainer, "-planetary").unwrap();
        // Two 32x32 Raw16 frames fit the writer's buffer, so nothing reached the file yet.
        assert!(queue_raw(&handle, 1));
        assert!(queue_raw(&handle, 2));
        let report = handle.sync(SyncGroup::Shared).unwrap();
        let on_disk = std::fs::metadata(session.join("capture.ser")).unwrap().len();
        handle.end_session();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report, SyncReport { synced: 1, failed: 0, error: None });
        let header = crate::ser::SerHeader::new(32, 32, crate::ser::SerColorId::Mono, 16).to_bytes().len();
        assert_eq!(on_disk as usize, header + 2 * 32 * 32 * 2);
    }

    /// A container that cannot be finalized is unreadable, so its failure reaches the
    /// sync that would otherwise call it saved. Opened on `/dev/full`, which takes the
    /// buffered frame and refuses the flush finalizing does.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_container_that_cannot_be_finalized_is_reported_by_the_next_sync() {
        use crate::ser::{SerColorId, SerHeader, SerWriter};

        let dir = tempfile::tempdir().unwrap();
        let (mut writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let session = handle.start_session(WritingSessionType::VideoContainer, "-planetary").unwrap();
        let mut container = SerWriter::create("/dev/full", SerHeader::new(32, 32, SerColorId::Mono, 16)).unwrap();
        container.write_raw_bytes(&[0; 32 * 32 * 2], None).unwrap();
        writer.ser_writer = Some(super::super::worker::ActiveSer {
            dir: session,
            group: SyncGroup::Shared,
            path: "/dev/full".into(),
            writer: container,
        });
        let writer_task = std::thread::spawn(move || writer.run());

        handle.end_session();
        let report = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report.failed, 1, "{report:?}");
        let error = report.error.unwrap();
        assert!(error.contains("/dev/full") && error.contains("No space left"), "{error}");
    }

    /// The guide loop runs on while a main capture stops, and the other way round: each
    /// sync settles and counts only its own producer's files.
    #[test]
    fn a_sync_settles_only_its_own_producers_files() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        let guide = handle.create_session(WritingSessionType::IndividualFrames, "-guide").unwrap();
        assert!(queue_raw(&handle, 1));
        for n in 1..=3 {
            let queued = handle.queue_raw_frame_in(
                Some(guide.clone()),
                tiny_raw_frame(),
                n,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            );
            assert_eq!(queued, Ok(true));
        }
        let guide_stopped = handle.sync(SyncGroup::Owned).unwrap();
        handle.end_session();
        let main_stopped = handle.sync(SyncGroup::Shared).unwrap();
        let guide_again = handle.sync(SyncGroup::Owned).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(guide_stopped.synced, 3);
        assert_eq!(main_stopped.synced, 1);
        assert_eq!(guide_again, SyncReport::default());
    }

    /// A frame that never reached the disk is named by the sync, so the observer is not
    /// told everything is safe when it is not.
    #[test]
    fn a_failed_write_is_reported_by_the_next_sync() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        let session = handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        // A plain file where the session folder goes: the frame cannot be written.
        std::fs::write(&session, b"").unwrap();
        assert!(queue_raw(&handle, 1));
        let report = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report.synced, 0);
        assert_eq!(report.failed, 1);
        assert!(report.error.is_some());
    }

    /// The final stack is a capture's one irreplaceable output: a queue still full of raw
    /// frames at Stop must make it wait, not turn it away as it does a raw frame.
    #[test]
    fn the_final_stack_waits_for_room_in_a_full_queue() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()).with_max_queue_size(1));
        handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        assert!(queue_raw(&handle, 1));
        assert!(handle.queue_raw_frame(tiny_raw_frame(), 2, FitsMetadata::new(), crate::camera::SensorType::Mono, None).is_err());

        let stacker = {
            let handle = handle.clone();
            std::thread::spawn(move || {
                let stack = crate::frame::Frame::filled(16, 16, 1, 0.5).unwrap();
                handle.queue_stacked_frame(Arc::new(stack), FitsMetadata::new())
            })
        };
        // The depth counts a request before it is sent: at 2 the stack is waiting on the queue.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while handle.queue_depth() < 2 {
            assert!(std::time::Instant::now() < deadline, "the stack was never offered");
            std::thread::yield_now();
        }
        assert!(!stacker.is_finished(), "the stack did not wait for room");
        let writer_task = std::thread::spawn(move || writer.run());
        assert_eq!(stacker.join().unwrap(), Ok(true));
        handle.end_session();
        let report = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(report, SyncReport { synced: 2, failed: 0, error: None });
    }

    /// A sync that cannot finish until `release`: a FIFO stands in for a device slow to
    /// acknowledge a flush (opening it for writing waits for a reader, as `sync_file` does).
    #[cfg(unix)]
    struct SlowDevice {
        fifo: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl SlowDevice {
        fn pending_in(dir: &Path, writer: &mut DiskWriter, group: SyncGroup) -> Self {
            let fifo = dir.join("slow_device");
            assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
            writer.pending[group.index()].written(&fifo);
            Self { fifo }
        }

        fn release(&self) {
            drop(std::fs::OpenOptions::new().read(true).open(&self.fifo).unwrap());
        }
    }

    /// The guide loop stopping syncs while the imaging camera may still be capturing through
    /// the same writer: its frames must keep reaching the disk meanwhile, or its queue fills
    /// and drops subs for as long as the slow card takes.
    #[cfg(unix)]
    #[test]
    fn a_sync_in_progress_does_not_hold_up_the_other_groups_frames() {
        let dir = tempfile::tempdir().unwrap();
        let (mut writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let device = SlowDevice::pending_in(dir.path(), &mut writer, SyncGroup::Owned);
        let writer_task = std::thread::spawn(move || writer.run());
        let session = handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();

        let guide_stop = {
            let handle = handle.clone();
            std::thread::spawn(move || handle.sync(SyncGroup::Owned))
        };
        std::thread::sleep(Duration::from_millis(200));
        assert!(queue_raw(&handle, 1));
        let frame = session.join("frame_000001.fits");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !frame.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let written_meanwhile = frame.exists();

        device.release();
        let _ = guide_stop.join().unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert!(written_meanwhile, "the imaging camera's frame waited out the guide camera's sync");
    }

    /// The final stack waits for room (it is the capture's one irreplaceable output), but a
    /// writer that never frees any — a drive pulled mid-write, a hung card — must not hold the
    /// capture in `Stopping` for ever, with the camera's handle and cooler held and Start
    /// refused. Here the writer never runs at all.
    #[test]
    fn the_final_stack_gives_up_on_a_writer_that_never_makes_room() {
        let dir = tempfile::tempdir().unwrap();
        let (_writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()).with_max_queue_size(1));
        handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        assert!(queue_raw(&handle, 1));

        let started = Instant::now();
        let stack = crate::frame::Frame::filled(16, 16, 1, 0.5).unwrap();
        let queued = handle.queue_stacked_frame(Arc::new(stack), FitsMetadata::new());

        assert_eq!(queued, Err(DiskWriterError::Unresponsive));
        assert!(started.elapsed() >= STALL_PATIENCE, "gave up after {:?}", started.elapsed());
        assert_eq!(handle.queue_depth(), 1, "the refused stack is not counted as queued");
    }

    /// Patience runs from the writer's last step, not the first wait: a slow card that is
    /// still writing is waited out well past it, or a long backlog would lose the stack.
    #[test]
    fn the_final_stack_waits_out_a_writer_that_is_slow_but_progressing() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()).with_max_queue_size(1));
        handle.start_session(WritingSessionType::IndividualFrames, "-stacking").unwrap();
        assert!(queue_raw(&handle, 1));
        let activity = Arc::clone(&handle.activity);
        let slow_card = std::thread::spawn(move || {
            // Steps that never free the slot, for twice the patience, then the writer runs.
            let until = Instant::now() + 2 * STALL_PATIENCE;
            while Instant::now() < until {
                activity.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(STALL_PATIENCE / 4);
            }
            writer.run();
        });

        let stack = crate::frame::Frame::filled(16, 16, 1, 0.5).unwrap();
        let queued = handle.queue_stacked_frame(Arc::new(stack), FitsMetadata::new());
        let report = handle.sync(SyncGroup::Shared);
        drop(handle);
        slow_card.join().unwrap();

        assert_eq!(queued, Ok(true));
        assert_eq!(report.map(|r| r.synced), Ok(2));
    }

    /// A sync against a writer that never answers ends, so the observer is told the save did
    /// not land instead of watching "Saving to disk" for the rest of the night.
    #[test]
    fn a_sync_gives_up_on_a_writer_that_never_answers() {
        let dir = tempfile::tempdir().unwrap();
        let (_writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()).with_max_queue_size(1));

        assert_eq!(handle.sync(SyncGroup::Shared), Err(DiskWriterError::Unresponsive), "with room in the queue");
        assert_eq!(handle.sync(SyncGroup::Shared), Err(DiskWriterError::Unresponsive), "with the queue full");
    }

    /// A sync while a container is open covers it as it is then; whatever is appended and
    /// the header finalizing writes come after, so the next sync must cover the file again.
    #[test]
    fn a_container_synced_while_open_is_synced_again_once_finalized() {
        let dir = tempfile::tempdir().unwrap();
        let (writer, handle) = DiskWriter::new(DiskWriterConfig::new(dir.path()));
        let writer_task = std::thread::spawn(move || writer.run());

        handle.start_session(WritingSessionType::VideoContainer, "-planetary").unwrap();
        assert!(queue_raw(&handle, 1));
        let while_open = handle.sync(SyncGroup::Shared).unwrap();
        assert!(queue_raw(&handle, 2));
        handle.end_session();
        let finalized = handle.sync(SyncGroup::Shared).unwrap();
        let after = handle.sync(SyncGroup::Shared).unwrap();
        drop(handle);
        writer_task.join().unwrap();

        assert_eq!(while_open.synced, 1);
        assert_eq!(finalized.synced, 1, "the finalized container was left to the write cache");
        assert_eq!(after, SyncReport::default());
    }
}
