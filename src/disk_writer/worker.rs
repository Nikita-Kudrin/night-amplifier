use chrono::{Local, Utc};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument};

use super::config::{DiskWriterMessage, FrameType, WriteRequest, WritingSessionType};
use super::error::DiskWriterError;
use super::utils::write_png;
use crate::fits::write_fits;
use crate::ser::{SerColorId, SerHeader, SerWriter};
use crate::telemetry::metrics as telemetry_metrics;

/// The disk writer background task
pub struct DiskWriter {
    /// Channel receiver for write requests
    pub(crate) receiver: mpsc::Receiver<DiskWriterMessage>,
    /// Shared queue depth counter
    pub(crate) queue_depth: Arc<AtomicUsize>,
    /// Session directory for raw frames
    pub(crate) session_dir: Arc<tokio::sync::RwLock<Option<PathBuf>>>,
    /// Stacked output directory
    pub(crate) stacked_dir: PathBuf,
    /// Active SER writer for planetary sessions
    pub(crate) ser_writer: Option<SerWriter>,
    /// Current session type
    pub(crate) session_type: WritingSessionType,
}

impl DiskWriter {
    /// Create a new disk writer with the given receiver, depth counter, session dir, and stacked dir
    pub fn new_internal(
        receiver: mpsc::Receiver<DiskWriterMessage>,
        queue_depth: Arc<AtomicUsize>,
        session_dir: Arc<tokio::sync::RwLock<Option<PathBuf>>>,
        stacked_dir: PathBuf,
    ) -> Self {
        Self {
            receiver,
            queue_depth,
            session_dir,
            stacked_dir,
            ser_writer: None,
            session_type: WritingSessionType::IndividualFrames,
        }
    }

    /// Run the disk writer task
    pub async fn run(mut self) {
        info!("Disk writer task started");

        while let Some(message) = self.receiver.recv().await {
            match message {
                DiskWriterMessage::StartSession { path, session_type } => {
                    self.session_type = session_type;
                    if session_type == WritingSessionType::VideoContainer {
                        // We'll initialize SerWriter on the first frame to get dimensions/bit depth
                        debug!(path = ?path, "Planetary session started, will use SER container");
                    }
                }
                DiskWriterMessage::WriteFrame(request) => {
                    let result = self.process_request(&request).await;

                    let depth = self.queue_depth.fetch_sub(1, Ordering::SeqCst) - 1;
                    telemetry_metrics::record_disk_writer_queue_depth(depth as u64);

                    if let Err(e) = result {
                        error!(error = %e, frame_number = request.frame_number, "Failed to write frame");
                    }
                }
                DiskWriterMessage::EndSession => {
                    if let Some(writer) = self.ser_writer.take() {
                        info!("Finalizing SER file");
                        if let Err(e) = writer.finalize() {
                            error!(error = %e, "Failed to finalize SER file");
                        }
                    }
                    self.session_type = WritingSessionType::IndividualFrames;
                }
            }
        }

        // Cleanup if task stops unexpectedly
        if let Some(writer) = self.ser_writer.take() {
            let _ = writer.finalize();
        }

        info!("Disk writer task stopped");
    }

    /// Process a single write request
    #[instrument(skip(self, request), fields(
        frame_type = ?request.frame_type,
        frame_number = request.frame_number,
        resolution = %format!("{}x{}x{}", request.frame.width(), request.frame.height(), request.frame.channels())
    ))]
    async fn process_request(&mut self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        match request.frame_type {
            FrameType::Raw => {
                if self.session_type == WritingSessionType::VideoContainer {
                    self.process_ser_frame(request).await
                } else {
                    self.process_fits_raw(request).await
                }
            }
            FrameType::Stacked => self.process_fits_stacked(request).await,
            FrameType::StackedPng => self.process_png_stacked(request).await,
        }
    }

    async fn process_ser_frame(&mut self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_dir = self.session_dir.read().await;
        let session_dir = session_dir.as_ref().ok_or_else(|| {
            DiskWriterError::DirectoryCreationFailed("No active session".to_string())
        })?;

        if self.ser_writer.is_none() {
            let filename = "capture.ser";
            let path = session_dir.join(filename);

            let color_id = match request.frame.channels() {
                1 => SerColorId::Mono,
                3 => SerColorId::Rgb,
                _ => SerColorId::Mono, // Fallback
            };

            // Determine bit depth from metadata or default to 8
            // Our frames are f32, SER supports 8/16. We'll use 16 for better precision if possible,
            let bit_depth = 16;

            let header = SerHeader::new(
                request.frame.width() as u32,
                request.frame.height() as u32,
                color_id,
                bit_depth,
            )
            .with_instrument(&request.metadata.camera.clone().unwrap_or_default());

            info!(path = ?path, ?color_id, bit_depth, "Creating new SER file");
            self.ser_writer = Some(SerWriter::create(path, header).map_err(|e| {
                DiskWriterError::WriteFailed(format!("Failed to create SER file: {}", e))
            })?);
        }

        if let Some(writer) = &mut self.ser_writer {
            // Convert timestamp from FITS date-obs to Windows FILETIME if needed,
            // but for now let's use current time as UTC timestamp.
            let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;

            writer
                .write_frame(&request.frame, Some(timestamp))
                .map_err(|e| {
                    DiskWriterError::WriteFailed(format!("Failed to write SER frame: {}", e))
                })?;
        }

        Ok(())
    }

    async fn process_fits_raw(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_dir = self.session_dir.read().await;
        let session_dir = session_dir.as_ref().ok_or_else(|| {
            DiskWriterError::DirectoryCreationFailed("No active session".to_string())
        })?;

        let filename = format!("frame_{:06}.fits", request.frame_number);
        let path = session_dir.join(filename);

        debug!(path = ?path, "Writing raw FITS file");

        tokio::task::spawn_blocking({
            let frame = request.frame.clone();
            let metadata = request.metadata.clone();
            let path = path.clone();
            move || write_fits(&frame, &path, Some(&metadata))
        })
        .await
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Raw FITS file written successfully");
        Ok(())
    }

    async fn process_fits_stacked(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_name = self
            .session_dir
            .read()
            .await
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| Local::now().format("%d-%m-%Y_%H-%M-%S").to_string());

        let filename = format!("{}.fits", session_name);
        let path = self.stacked_dir.join(filename);

        debug!(path = ?path, "Writing stacked FITS file");

        tokio::task::spawn_blocking({
            let frame = request.frame.clone();
            let metadata = request.metadata.clone();
            let path = path.clone();
            move || write_fits(&frame, &path, Some(&metadata))
        })
        .await
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Stacked FITS file written successfully");
        Ok(())
    }

    async fn process_png_stacked(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_name = self
            .session_dir
            .read()
            .await
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| Local::now().format("%d-%m-%Y_%H-%M-%S").to_string());

        let filename = format!("{}_stretched.png", session_name);
        let path = self.stacked_dir.join(filename);

        debug!(path = ?path, "Writing stretched PNG file");

        tokio::task::spawn_blocking({
            let frame = request.frame.clone();
            let path = path.clone();
            move || write_png(&frame, &path)
        })
        .await
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?
        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Stretched PNG file written successfully");
        Ok(())
    }
}
