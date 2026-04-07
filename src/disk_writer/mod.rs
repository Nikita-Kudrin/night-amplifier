//! Asynchronous disk writer with queue for saving captured frames
//!
//! This module provides a background task that writes frames to disk without
//! blocking the capture loop. It uses a bounded channel to queue write requests
//! and monitors queue depth to warn about slow disk performance.

use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use tokio::sync::mpsc;
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
pub use handle::DiskWriterHandle;
pub use worker::DiskWriter;

use crate::telemetry::metrics as telemetry_metrics;

impl DiskWriter {
    /// Create a new disk writer with the given configuration
    ///
    /// Returns the writer task and a handle for sending requests
    pub fn new(config: DiskWriterConfig) -> (Self, DiskWriterHandle) {
        let (sender, receiver) = mpsc::channel(config.max_queue_size);
        let queue_depth = Arc::new(AtomicUsize::new(0));
        let queue_warning = Arc::new(AtomicBool::new(false));
        let session_dir = Arc::new(tokio::sync::RwLock::new(None));
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

        let writer = Self::new_internal(
            receiver,
            Arc::clone(&queue_depth),
            Arc::clone(&session_dir),
            stacked_dir.clone(),
        );

        let handle = DiskWriterHandle {
            sender,
            queue_depth,
            queue_warning,
            session_dir,
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

    #[tokio::test]
    async fn test_disk_writer_handle_enabled() {
        let config = DiskWriterConfig::default();
        let (_writer, handle) = DiskWriter::new(config);

        assert!(handle.is_enabled());
        handle.set_enabled(false);
        assert!(!handle.is_enabled());
    }

    #[tokio::test]
    async fn test_disk_writer_session_management() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_dw");
        let config = DiskWriterConfig::new(&temp_dir);
        let (_writer, handle) = DiskWriter::new(config);

        let session_path = handle
            .start_session(WritingSessionType::IndividualFrames)
            .await
            .unwrap();
        assert!(session_path.exists());

        let dir = handle.session_dir().await;
        assert!(dir.is_some());
        assert_eq!(dir.unwrap(), session_path);

        let name = handle.session_name().await;
        assert!(name.is_some());

        let ended = handle.end_session().await;
        assert!(ended.is_some());
        assert!(handle.session_dir().await.is_none());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[tokio::test]
    async fn test_queue_depth_tracking() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_queue");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::IndividualFrames)
            .await
            .unwrap();
        let writer_task = tokio::spawn(writer.run());

        let frame = Frame::filled(10, 10, 1, 0.5).unwrap();
        for i in 0..3 {
            let _ = handle
                .queue_raw_frame(frame.clone(), i, FitsMetadata::new())
                .await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        assert_eq!(handle.queue_depth(), 0);

        drop(handle);
        writer_task.abort();
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[tokio::test]
    async fn test_queue_warning_threshold() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_warning");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (_writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::IndividualFrames)
            .await
            .unwrap();
        assert!(!handle.has_queue_warning());

        let frame = Frame::filled(10, 10, 1, 0.5).unwrap();
        for i in 0..(QUEUE_WARNING_THRESHOLD + 2) {
            let _ = handle
                .queue_raw_frame(frame.clone(), i as u64, FitsMetadata::new())
                .await;
        }

        assert!(handle.has_queue_warning());
        handle.clear_queue_warning();
        assert!(!handle.has_queue_warning());

        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[tokio::test]
    async fn test_disk_writer_ser_session() {
        let temp_dir = std::env::temp_dir().join("night_amplifier_test_ser");
        let config = DiskWriterConfig::new(&temp_dir).with_max_queue_size(10);
        let (writer, handle) = DiskWriter::new(config);

        handle
            .start_session(WritingSessionType::VideoContainer)
            .await
            .unwrap();
        let writer_task = tokio::spawn(writer.run());

        let frame = Frame::filled(32, 32, 1, 0.5).unwrap();
        let mut metadata = FitsMetadata::new();
        metadata.camera = Some("Test Camera".to_string());

        for i in 0..5 {
            let _ = handle
                .queue_raw_frame(frame.clone(), i, metadata.clone())
                .await;
        }

        // Give it some time to process
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let session_dir = handle.session_dir().await.unwrap();
        let ser_path = session_dir.join("capture.ser");
        assert!(ser_path.exists(), "SER file should be created");

        handle.end_session().await.unwrap();

        // Drop handle and wait for worker to finish
        drop(handle);
        writer_task.await.unwrap();

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

    #[test]
    fn test_disk_writer_error_display() {
        let err = DiskWriterError::QueueFull;
        assert_eq!(err.to_string(), "Disk writer queue is full");
    }
}
