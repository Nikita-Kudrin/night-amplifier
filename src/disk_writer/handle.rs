use chrono::Local;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use tracing::{error, info, warn};

use super::config::{
    DiskWriterMessage, FrameType, WriteRequest, WritingSessionType, QUEUE_WARNING_THRESHOLD,
};
use super::error::DiskWriterError;
use crate::fits::FitsMetadata;
use crate::frame::Frame;
use crate::telemetry::metrics as telemetry_metrics;

/// Handle to the disk writer for sending write requests
#[derive(Clone)]
pub struct DiskWriterHandle {
    /// Channel sender for write requests
    pub(crate) sender: mpsc::SyncSender<DiskWriterMessage>,
    /// Current queue depth
    pub(crate) queue_depth: Arc<AtomicUsize>,
    /// Warning flag for queue overflow
    pub(crate) queue_warning: Arc<AtomicBool>,
    /// Session directory path for raw frames
    pub(crate) session_dir: Arc<RwLock<Option<PathBuf>>>,
    /// Whether saving is enabled
    pub(crate) enabled: Arc<AtomicBool>,
    /// Stacked output directory
    pub(crate) stacked_dir: PathBuf,
}

impl DiskWriterHandle {
    /// Get current queue depth
    pub fn queue_depth(&self) -> usize {
        self.queue_depth.load(Ordering::SeqCst)
    }

    /// Check if queue warning is active
    pub fn has_queue_warning(&self) -> bool {
        self.queue_warning.load(Ordering::SeqCst)
    }

    /// Clear queue warning flag
    pub fn clear_queue_warning(&self) {
        self.queue_warning.store(false, Ordering::SeqCst);
    }

    /// Check if saving is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Enable or disable saving
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Start a new capture session, creating the session directory
    pub fn start_session(&self, session_type: WritingSessionType) -> std::io::Result<PathBuf> {
        let timestamp = Local::now().format("%d-%m-%Y_%H-%M-%S").to_string();
        let session_path = self
            .stacked_dir
            .parent()
            .unwrap_or(Path::new("."))
            .join("raw")
            .join(&timestamp);

        std::fs::create_dir_all(&session_path)?;

        *self.session_dir.write().unwrap_or_else(|e| e.into_inner()) = Some(session_path.clone());

        // Notify the worker to start a session
        let _ = self.sender.try_send(DiskWriterMessage::StartSession {
            path: session_path.clone(),
            session_type,
        });

        info!(session_dir = ?session_path, ?session_type, "Started new capture session");
        Ok(session_path)
    }

    /// End the current capture session
    pub fn end_session(&self) -> Option<PathBuf> {
        let path = self
            .session_dir
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if path.is_some() {
            // Notify the worker to end the session (finalize SER, etc.)
            let _ = self.sender.try_send(DiskWriterMessage::EndSession);
        }
        path
    }

    /// Get the current session directory
    pub fn session_dir(&self) -> Option<PathBuf> {
        self.session_dir
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Get the session name (directory name) for the current session
    pub fn session_name(&self) -> Option<String> {
        self.session_dir
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
    }

    /// Queue a frame for writing
    pub fn queue_frame(&self, request: WriteRequest) -> Result<bool, DiskWriterError> {
        if !self.is_enabled() {
            return Ok(false);
        }

        let depth = self.queue_depth.fetch_add(1, Ordering::SeqCst) + 1;
        telemetry_metrics::record_disk_writer_queue_depth(depth as u64);

        if depth > QUEUE_WARNING_THRESHOLD {
            self.queue_warning.store(true, Ordering::SeqCst);
            warn!(
                queue_depth = depth,
                "Disk writer queue depth exceeds threshold"
            );
        }

        let message = DiskWriterMessage::WriteFrame(request);
        match self.sender.try_send(message) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(msg)) => {
                let req = match msg {
                    DiskWriterMessage::WriteFrame(r) => r,
                    _ => unreachable!(),
                };
                self.queue_depth.fetch_sub(1, Ordering::SeqCst);
                telemetry_metrics::record_disk_writer_queue_depth(
                    self.queue_depth.load(Ordering::SeqCst) as u64,
                );
                error!(
                    "Disk writer queue full, dropping frame {}",
                    req.frame_number
                );
                Err(DiskWriterError::QueueFull)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.queue_depth.fetch_sub(1, Ordering::SeqCst);
                telemetry_metrics::record_disk_writer_queue_depth(
                    self.queue_depth.load(Ordering::SeqCst) as u64,
                );
                Err(DiskWriterError::WriterClosed)
            }
        }
    }

    /// Queue a raw frame for writing
    pub fn queue_raw_frame(
        &self,
        frame: Frame,
        frame_number: u64,
        metadata: FitsMetadata,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame(WriteRequest {
            frame,
            frame_type: FrameType::Raw,
            frame_number,
            metadata,
        })
    }

    /// Queue a stacked result for writing (FITS format)
    pub fn queue_stacked_frame(
        &self,
        frame: Frame,
        metadata: FitsMetadata,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame(WriteRequest {
            frame,
            frame_type: FrameType::Stacked,
            frame_number: 0,
            metadata,
        })
    }

    /// Queue a stretched stacked frame for writing (PNG format for sharing)
    pub fn queue_stacked_png(
        &self,
        frame: Frame,
        stacked_count: u64,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame(WriteRequest {
            frame,
            frame_type: FrameType::StackedPng,
            frame_number: stacked_count,
            metadata: FitsMetadata::new(),
        })
    }
}
