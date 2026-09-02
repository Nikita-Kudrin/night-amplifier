//! Disk writer with queue for saving captured frames
//!
//! This module provides a background thread that writes frames to disk without
//! blocking the capture loop or the tokio runtime. It uses a bounded channel to
//! queue write requests and monitors queue depth to warn about slow disk
//! performance.

use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{mpsc, Arc, RwLock};
use tracing::error;

mod config;
mod error;
mod handle;
mod utils;
mod worker;

pub use config::{
    DiskWriterConfig, DiskWriterMessage, FrameType, WriteRequest, WritingSessionType,
    QUEUE_WARNING_THRESHOLD,
};
pub use error::DiskWriterError;
pub use handle::{DiskWriterHandle, OpenSession};
pub use worker::DiskWriter;

#[cfg(test)]
pub(crate) use utils::write_rgb8_png;

use crate::telemetry::metrics as telemetry_metrics;

impl DiskWriter {
    /// Create a new disk writer with the given configuration
    ///
    /// Returns the writer task and a handle for sending requests
    pub fn new(config: DiskWriterConfig) -> (Self, DiskWriterHandle) {
        let (sender, receiver) = mpsc::sync_channel(config.max_queue_size);
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let queue_warning = Arc::new(AtomicBool::new(false));
        let enabled = Arc::new(AtomicBool::new(config.enabled));

        // Create directories
        let raw_dir = config.base_dir.join("raw");
        let stacked_dir = config.base_dir.join("stacked");

        if let Err(e) = std::fs::create_dir_all(&raw_dir) {
            error!(error = %e, path = ?raw_dir, "Failed to create raw captures directory");
        }
        if let Err(e) = std::fs::create_dir_all(&stacked_dir) {
            error!(error = %e, path = ?stacked_dir, "Failed to create stacked captures directory");
        }

        let writer = Self::new_internal(receiver, Arc::clone(&queue_depth), stacked_dir.clone());

        let handle = DiskWriterHandle {
            sender,
            queue_depth,
            queue_warning,
            session: Arc::new(RwLock::new(None)),
            enabled,
            stacked_dir,
        };

        // Record initial metrics
        telemetry_metrics::record_disk_writer_queue_capacity(config.max_queue_size as u64);
        telemetry_metrics::record_disk_writer_queue_depth(0);

        (writer, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits::FitsMetadata;
    use crate::frame::Frame;
    use std::path::PathBuf;

    #[test]
    fn test_disk_writer_config_default() {
        let config = DiskWriterConfig::default();
        assert_eq!(config.base_dir, PathBuf::from("captures"));
        assert_eq!(config.max_queue_size, 20);
        assert!(config.enabled);
    }

    #[test]
    fn test_disk_writer_config_builder() {
        let config = DiskWriterConfig::new("/tmp/captures")
            .with_max_queue_size(10)
            .with_enabled(false);

        assert_eq!(config.base_dir, PathBuf::from("/tmp/captures"));
        assert_eq!(config.max_queue_size, 10);
        assert!(!config.enabled);
    }

    #[test]
    fn test_disk_writer_handle_enabled() {
        // Not `DiskWriterConfig::default()`: `DiskWriter::new` creates the capture
        // directories, and the default points at `./captures` relative to wherever the
        // suite was run from.
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw_enabled");
        let (_writer, handle) = DiskWriter::new(DiskWriterConfig::new(&temp_dir));

        assert!(handle.is_enabled());
        handle.set_enabled(false);
        assert!(!handle.is_enabled());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_disk_writer_session_management() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw");
        let config = DiskWriterConfig::new(&temp_dir);
        let (_writer, handle) = DiskWriter::new(config);

        let session_path = handle
            .start_session(WritingSessionType::IndividualFrames, "")
            .unwrap();
        assert!(session_path.exists());

        let dir = handle.session_dir();
        assert!(dir.is_some());
        assert_eq!(dir.unwrap(), session_path);

        let name = handle.session_name();
        assert!(name.is_some());

        let ended = handle.end_session();
        assert!(ended.is_some());
        assert!(handle.session_dir().is_none());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// The suffix is what tells a night's raw folders apart, and it has to be part of
    /// the directory name rather than a file inside it — nothing else about a finished
    /// session survives to say which mode produced it.
    #[test]
    fn start_session_suffixes_the_directory_name() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw_suffix");
        let config = DiskWriterConfig::new(&temp_dir);
        let (_writer, handle) = DiskWriter::new(config);

        let path = handle
            .start_session(WritingSessionType::IndividualFrames, "-live")
            .unwrap();

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.ends_with("-live"),
            "session directory {name} carries no mode suffix"
        );
        assert!(path.exists());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// Two sessions inside one wall-clock second must not land on the same folder. The
    /// timestamp has one-second resolution but a session can be rolled far faster, and
    /// `create_dir_all` succeeds silently on an existing directory — so the second
    /// session used to merge into the first, truncating its `capture.ser` on the way.
    #[test]
    fn two_sessions_in_the_same_second_get_separate_directories() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw_same_second");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir);
        let (_writer, handle) = DiskWriter::new(config);

        let first = handle
            .start_session(WritingSessionType::IndividualFrames, "-stacking")
            .unwrap();
        handle.end_session();
        let second = handle
            .start_session(WritingSessionType::IndividualFrames, "-stacking")
            .unwrap();

        assert_ne!(first, second);
        assert!(first.exists() && second.exists());
        // The mode is still readable off the name: the counter goes before the suffix.
        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                name.ends_with("-stacking"),
                "{name} no longer names the mode that filled it"
            );
        }

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// An empty suffix has to leave the bare timestamp untouched — a stray separator
    /// would show up in every stacked filename, which is derived from this name.
    #[test]
    fn an_empty_suffix_leaves_the_timestamp_alone() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw_nosuffix");
        let config = DiskWriterConfig::new(&temp_dir);
        let (_writer, handle) = DiskWriter::new(config);

        let path = handle
            .start_session(WritingSessionType::IndividualFrames, "")
            .unwrap();

        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            name.len(),
            "DD-MM-YYYY_HH-MM-SS".len(),
            "expected a bare timestamp, got {name}"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_queue_depth_tracking() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_queue");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::IndividualFrames, "")
            .unwrap();
        let writer_task = std::thread::spawn(move || writer.run());

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(200);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 10,
            height: 10,
            format: crate::camera::ImageFormat::Raw16,
        });

        for i in 0..5 {
            let _ = handle.queue_raw_frame(
                std::sync::Arc::clone(&frame),
                i,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            );
        }

        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(handle.queue_depth(), 0);

        drop(handle);
        writer_task.join().unwrap();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_queue_warning_threshold() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_warning");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (_writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::IndividualFrames, "")
            .unwrap();
        assert!(!handle.has_queue_warning());

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(200);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 10,
            height: 10,
            format: crate::camera::ImageFormat::Raw16,
        });
        for i in 0..(QUEUE_WARNING_THRESHOLD + 2) {
            let _ = handle.queue_raw_frame(
                std::sync::Arc::clone(&frame),
                i as u64,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            );
        }

        assert!(handle.has_queue_warning());
        handle.clear_queue_warning();
        assert!(!handle.has_queue_warning());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_disk_writer_ser_session() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_ser");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::VideoContainer, "")
            .unwrap();
        let writer_task = std::thread::spawn(move || writer.run());

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(32 * 32 * 2);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        });
        let mut metadata = FitsMetadata::new();
        metadata.camera = Some("Test Camera".to_string());

        for i in 0..5 {
            let _ = handle.queue_raw_frame(
                std::sync::Arc::clone(&frame),
                i,
                metadata.clone(),
                crate::camera::SensorType::Mono,
                None,
            );
        }

        // Give it some time to process
        std::thread::sleep(std::time::Duration::from_millis(500));

        let session_dir = handle.session_dir().unwrap();
        let ser_path = session_dir.join("capture.ser");
        assert!(ser_path.exists(), "SER file should be created");

        handle.end_session().unwrap();

        // Drop handle and wait for worker to finish
        drop(handle);
        writer_task.join().unwrap();

        // Check if file size is reasonable (Header 178 + 5 frames * 32*32*2 bytes for 16-bit)
        let metadata = std::fs::metadata(&ser_path).unwrap();
        let expected_min_size = 178 + (5 * 32 * 32 * 2);
        assert!(
            metadata.len() >= expected_min_size as u64,
            "File size {} should be at least {}",
            metadata.len(),
            expected_min_size
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// A frame is filed under the session that was open when it was *queued*, not the
    /// one open when the worker gets to it. Resolving the destination at write time put
    /// the whole in-flight backlog — everything a slow disk is still holding — into the
    /// folder of the mode the capture had just switched to.
    #[test]
    fn backlogged_frames_stay_in_the_session_they_were_captured_in() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_roll_backlog");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        let stacking_dir = handle
            .start_session(WritingSessionType::IndividualFrames, "-stacking")
            .unwrap();

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(32 * 32 * 2);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        });
        // Queued while Stacking is the open session; the worker has not started, so it
        // is still in the queue when the mode changes — exactly the backlog a slow disk
        // keeps.
        handle
            .queue_raw_frame(
                std::sync::Arc::clone(&frame),
                1,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            )
            .unwrap();

        handle.abandon_session();
        let live_dir = handle
            .start_session(WritingSessionType::IndividualFrames, "-live")
            .unwrap();
        assert_ne!(stacking_dir, live_dir);

        let writer_task = std::thread::spawn(move || writer.run());
        drop(handle);
        writer_task.join().unwrap();

        assert!(
            stacking_dir.join("frame_000001.fits").exists(),
            "a frame captured in Stacking was filed under {live_dir:?} instead"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// A full write queue must not decide which container a frame lands in. The
    /// container type used to reach the worker in a `StartSession` message sharing the
    /// queue with the frames, sent with `try_send` — so a disk that had fallen behind
    /// swallowed it, and the session that followed inherited the previous one's format.
    /// It travels with each frame now, which is why there is no such message any more.
    #[test]
    fn a_full_queue_does_not_change_the_container_a_session_writes() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_full_queue_roll");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(2);
        let (writer, handle) = DiskWriter::new(config);

        // Planetary session: StartSession{VideoContainer} occupies one queue slot.
        handle
            .start_session(WritingSessionType::VideoContainer, "-stacking")
            .unwrap();

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(32 * 32 * 2);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        });
        // Fills the queue, as a disk that cannot keep up would.
        handle
            .queue_raw_frame(
                std::sync::Arc::clone(&frame),
                1,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            )
            .unwrap();

        // A roll to Deep Sky / individual FITS, with no room left in the queue to
        // announce it.
        handle.abandon_session();
        let live_dir = handle
            .start_session(WritingSessionType::IndividualFrames, "-live")
            .unwrap();
        handle
            .queue_raw_frame(
                std::sync::Arc::clone(&frame),
                2,
                FitsMetadata::new(),
                crate::camera::SensorType::Mono,
                None,
            )
            .ok();

        let writer_task = std::thread::spawn(move || writer.run());
        drop(handle);
        writer_task.join().unwrap();

        assert!(
            !live_dir.join("capture.ser").exists(),
            "the rolled Deep Sky session wrote a SER container: the format followed the \
             worker's last-heard session instead of the frame's own"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// Rolling a planetary session mid-capture has to close the old container and open
    /// one in the new folder. Nothing announces the roll to the worker any more, so it
    /// has to notice from the frames themselves — otherwise every frame after the switch
    /// keeps appending to the previous mode's `capture.ser` and the new folder stays
    /// empty while claiming to hold the session.
    #[test]
    fn a_planetary_roll_moves_the_container_to_the_new_session() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_ser_roll");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(32 * 32 * 2);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        });
        let queue = |suffix: &str, n: u64| {
            let dir = handle
                .start_session(WritingSessionType::VideoContainer, suffix)
                .unwrap();
            handle
                .queue_raw_frame(
                    std::sync::Arc::clone(&frame),
                    n,
                    FitsMetadata::new(),
                    crate::camera::SensorType::Mono,
                    None,
                )
                .unwrap();
            dir
        };

        let stacking_dir = queue("-stacking", 1);
        handle.abandon_session();
        let live_dir = queue("-live", 2);

        let writer_task = std::thread::spawn(move || writer.run());
        drop(handle);
        writer_task.join().unwrap();

        assert!(stacking_dir.join("capture.ser").exists());
        assert!(
            live_dir.join("capture.ser").exists(),
            "frames captured after the roll kept appending to the previous session's \
             container, leaving {live_dir:?} empty"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// The frame count only reaches a SER header when the container is finalized, so a
    /// session that ends without that leaves an unreadable file. `end_session` waits for
    /// the writer precisely so a busy queue cannot swallow the news.
    #[test]
    fn ending_a_session_finalizes_its_container() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_ser_finalize");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(2);
        let (writer, handle) = DiskWriter::new(config);
        let writer_task = std::thread::spawn(move || writer.run());

        let session_dir = handle
            .start_session(WritingSessionType::VideoContainer, "-stacking")
            .unwrap();

        let pool = crate::camera::BufferPool::new();
        let buf = pool.get(32 * 32 * 2);
        let frame = std::sync::Arc::new(crate::camera::RawFrame {
            data: buf,
            width: 32,
            height: 32,
            format: crate::camera::ImageFormat::Raw16,
        });
        for i in 0..3 {
            handle
                .queue_raw_frame(
                    std::sync::Arc::clone(&frame),
                    i,
                    FitsMetadata::new(),
                    crate::camera::SensorType::Mono,
                    None,
                )
                .ok();
        }

        handle.end_session();

        // `end_session` waits for the message to be accepted, not for the worker to work
        // through the frames queued ahead of it. The handle stays alive throughout, so
        // nothing but that message can be finalizing the container.
        let ser_path = session_dir.join("capture.ser");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let frame_count = loop {
            let count = crate::ser::SerReader::open(&ser_path)
                .map(|r| r.frame_count())
                .unwrap_or(0);
            if count > 0 || std::time::Instant::now() >= deadline {
                break count;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert!(
            frame_count > 0,
            "the container reports {frame_count} frames: its header was never finalized"
        );

        drop(handle);
        writer_task.join().unwrap();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_disk_writer_error_display() {
        let err = DiskWriterError::QueueFull;
        assert_eq!(err.to_string(), "Disk writer queue is full");
    }

    /// Regression guard: `stacked_dir` is only created once, at `DiskWriter::new`.
    /// Something outside the process can still remove it mid-session — an unmounted
    /// USB drive, a tidy-up script — and every following write used to fail with
    /// ENOENT for the rest of the process's life. A write after the directory
    /// reappears (or, as here, is simply removed once and never touched again) must
    /// recreate it rather than staying broken.
    #[test]
    fn test_disk_writer_recreates_missing_stacked_dir() {
        let temp_dir =
            std::env::temp_dir().join("night_amplifier_test_missing_stacked_dir");
        let _ = std::fs::remove_dir_all(&temp_dir);
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::IndividualFrames, "")
            .unwrap();
        let writer_task = std::thread::spawn(move || writer.run());

        let stacked_dir = temp_dir.join("stacked");
        assert!(stacked_dir.exists(), "precondition: DiskWriter::new should have created it");
        std::fs::remove_dir_all(&stacked_dir).unwrap();
        assert!(!stacked_dir.exists(), "precondition: directory must actually be gone");

        let frame = Frame::filled(4, 4, 3, 0.5).unwrap();
        handle
            .queue_stacked_frame(std::sync::Arc::new(frame), FitsMetadata::new())
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(300));

        drop(handle);
        writer_task.join().unwrap();

        let entries: Vec<_> = std::fs::read_dir(&stacked_dir)
            .expect("stacked dir was not recreated")
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "stacked FITS file was not written after the missing directory should \
             have been recreated"
        );

        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
