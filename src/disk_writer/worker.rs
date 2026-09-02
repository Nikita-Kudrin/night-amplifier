use chrono::{Local, Utc};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use tracing::{debug, error, info, instrument, warn};

use super::config::{DiskWriterMessage, FrameType, WriteRequest, WritingSessionType};
use super::error::DiskWriterError;
use super::utils::write_rgb8_png;
use crate::fits::{write_fits, write_fits_from_raw, write_fits_u16};
use crate::ser::{SerColorId, SerHeader, SerWriter};
use crate::telemetry::metrics as telemetry_metrics;

/// Recreate a captures directory if it has gone missing since the writer started.
///
/// `stacked_dir` is created once, at server startup; a session's `raw` directory is
/// created once, at session start. Either can still vanish under an observer's feet —
/// an unmounted USB drive, a network share dropping, a tidy-up script — and every
/// following write would otherwise fail with ENOENT for the rest of the process's
/// life. The call is idempotent and cheap (one syscall once the directory is back),
/// so paying it before every write is simpler than tracking "have we already seen
/// this directory disappear".
fn ensure_dir(path: &Path) -> Result<(), DiskWriterError> {
    std::fs::create_dir_all(path).map_err(|e| {
        DiskWriterError::DirectoryCreationFailed(format!("{}: {}", path.display(), e))
    })
}

/// The base name a stacked export is filed under: its raw session's folder name, so the
/// stack and the subs it was built from are findable from each other.
///
/// Falls back to the current time when no session was open — the stacked switch can be
/// on with every raw-frame switch off, and the file still needs a name.
fn stacked_output_name(request: &WriteRequest) -> String {
    request
        .session
        .as_ref()
        .and_then(|s| s.dir.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| Local::now().format("%d-%m-%Y_%H-%M-%S").to_string())
}

/// The disk writer background task.
///
/// Runs on a dedicated OS thread so that file I/O never competes with
/// the tokio blocking-thread pool used by stacking, plate solving, etc.
pub struct DiskWriter {
    /// Channel receiver for write requests
    pub(crate) receiver: mpsc::Receiver<DiskWriterMessage>,
    /// Shared queue depth counter
    pub(crate) queue_depth: Arc<AtomicUsize>,
    /// Stacked output directory
    pub(crate) stacked_dir: PathBuf,
    /// Active SER writer for planetary sessions, and the session directory it belongs to
    pub(crate) ser_writer: Option<ActiveSer>,
}

/// The open SER container, tied to the session directory it was created for.
///
/// The directory is kept alongside the writer so a frame stamped with a different
/// session rolls the container instead of being appended to the previous one. Relying on
/// `EndSession` alone was not enough: that message shares the bounded write queue with
/// the frames, so a disk that has fallen behind can lose it.
pub(crate) struct ActiveSer {
    pub(crate) dir: PathBuf,
    pub(crate) writer: SerWriter,
}

impl DiskWriter {
    /// Create a new disk writer with the given receiver, depth counter, session dir, and stacked dir
    pub fn new_internal(
        receiver: mpsc::Receiver<DiskWriterMessage>,
        queue_depth: Arc<AtomicUsize>,
        stacked_dir: PathBuf,
    ) -> Self {
        Self {
            receiver,
            queue_depth,
            stacked_dir,
            ser_writer: None,
        }
    }

    /// Run the disk writer task (blocking — intended for a dedicated OS thread)
    pub fn run(mut self) {
        info!("Disk writer task started");

        while let Ok(message) = self.receiver.recv() {
            match message {
                DiskWriterMessage::WriteFrame(request) => {
                    let result = self.process_request(&request);

                    let depth = self.queue_depth.fetch_sub(1, Ordering::SeqCst) - 1;
                    telemetry_metrics::record_disk_writer_queue_depth(depth as u64);

                    if let Err(e) = result {
                        error!(error = %e, frame_number = request.frame_number, "Failed to write frame");
                    }
                }
                DiskWriterMessage::EndSession => self.finalize_ser(),
            }
        }

        // Cleanup if task stops unexpectedly
        self.finalize_ser();

        info!("Disk writer task stopped");
    }

    /// Process a single write request
    #[instrument(skip(self, request), fields(
        frame_type = ?request.frame_type,
        frame_number = request.frame_number
    ))]
    fn process_request(&mut self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Storage);
        match request.frame_type {
            FrameType::Raw(_) => match request.session.as_ref().map(|s| s.session_type) {
                Some(WritingSessionType::VideoContainer) => self.process_ser_frame(request),
                _ => self.process_fits_raw(request),
            },
            FrameType::Stacked(_) => self.process_fits_stacked(request),
            FrameType::StackedPng { .. } => self.process_png_stacked(request),
        }
    }

    /// Close the open SER container, writing its frame count into the header.
    ///
    /// A container left unfinalized is unreadable, so every path that stops writing to
    /// one goes through here: the session ending, a session roll, and the writer thread
    /// shutting down.
    fn finalize_ser(&mut self) {
        let Some(active) = self.ser_writer.take() else {
            return;
        };
        info!(dir = ?active.dir, "Finalizing SER file");
        if let Err(e) = active.writer.finalize() {
            error!(error = %e, "Failed to finalize SER file");
        }
    }

    fn process_ser_frame(&mut self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_dir = request.session.as_ref().map(|s| &s.dir).ok_or_else(|| {
            DiskWriterError::DirectoryCreationFailed("No active session".to_string())
        })?;

        // The frame belongs to a different session than the open container: close that
        // one out rather than appending a frame from another folder to it.
        if self
            .ser_writer
            .as_ref()
            .is_some_and(|active| active.dir != *session_dir)
        {
            self.finalize_ser();
        }

        if self.ser_writer.is_none() {
            ensure_dir(session_dir)?;
            let path = session_dir.join("capture.ser");

            let (frame_width, frame_height, frame_channels) = match &request.frame_type {
                FrameType::Raw(r) => (
                    r.width,
                    r.height,
                    if r.format == crate::camera::ImageFormat::Rgb24 {
                        3
                    } else {
                        1
                    },
                ),
                FrameType::Stacked(s) => (s.width() as u32, s.height() as u32, s.channels()),
                // Unreachable in practice: `process_request` only ever routes
                // `FrameType::Raw` into `process_ser_frame`. Handled explicitly
                // rather than folded into the arm above because `StackedPng`
                // carries pre-rendered RGB8 bytes, not a `Frame`.
                FrameType::StackedPng { width, height, .. } => (*width, *height, 3),
            };

            let color_id = match frame_channels {
                1 => match request.sensor_type {
                    crate::camera::SensorType::Mono => SerColorId::Mono,
                    crate::camera::SensorType::Color => {
                        match request.bayer_pattern {
                            Some(crate::CfaPattern::Rggb) => SerColorId::BayerRggb,
                            Some(crate::CfaPattern::Grbg) => SerColorId::BayerGrbg,
                            Some(crate::CfaPattern::Gbrg) => SerColorId::BayerGbrg,
                            Some(crate::CfaPattern::Bggr) => SerColorId::BayerBggr,
                            None => SerColorId::BayerRggb, // Fallback
                        }
                    }
                },
                3 => SerColorId::Rgb,
                _ => SerColorId::Mono, // Fallback
            };

            // Determine bit depth from metadata or default to 16 for f32 frames
            let bit_depth = match &request.frame_type {
                FrameType::Raw(r) => {
                    if r.format == crate::camera::ImageFormat::Raw16 {
                        16
                    } else {
                        8
                    }
                }
                _ => 16,
            };

            let header = SerHeader::new(frame_width, frame_height, color_id, bit_depth)
                .with_instrument(&request.metadata.camera.clone().unwrap_or_default());

            info!(path = ?path, ?color_id, bit_depth, "Creating new SER file for planetary session");
            let writer = SerWriter::create(path, header).map_err(|e| {
                DiskWriterError::WriteFailed(format!("Failed to create SER file: {}", e))
            })?;
            self.ser_writer = Some(ActiveSer {
                dir: session_dir.clone(),
                writer,
            });
        }

        if let Some(ActiveSer { writer, .. }) = &mut self.ser_writer {
            let frame_width = match &request.frame_type {
                FrameType::Raw(r) => r.width,
                FrameType::Stacked(s) => s.width() as u32,
                FrameType::StackedPng { width, .. } => *width,
            };
            let frame_height = match &request.frame_type {
                FrameType::Raw(r) => r.height,
                FrameType::Stacked(s) => s.height() as u32,
                FrameType::StackedPng { height, .. } => *height,
            };

            // Check dimensions for consistency
            if frame_width != writer.header().width || frame_height != writer.header().height {
                warn!(
                    frame_dims = ?(frame_width, frame_height),
                    ser_dims = ?(writer.header().width, writer.header().height),
                    "Frame dimensions changed during SER session, rejecting frame"
                );
                return Err(DiskWriterError::WriteFailed(
                    "Dimension mismatch for SER session".to_string(),
                ));
            }

            let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;

            debug!(
                frame_number = request.frame_number,
                "Writing frame to SER container"
            );

            match &request.frame_type {
                FrameType::Raw(r) => {
                    writer
                        .write_raw_bytes(r.data_slice(), Some(timestamp))
                        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;
                }
                FrameType::Stacked(s) => {
                    writer
                        .write_frame(s, Some(timestamp))
                        .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;
                }
                FrameType::StackedPng { .. } => {
                    // Unreachable in practice: `process_request` only ever routes
                    // `FrameType::Raw` into `process_ser_frame`, and a PNG export
                    // carries pre-rendered RGB8 bytes anyway, not a `Frame` a SER
                    // container could accept.
                    return Err(DiskWriterError::WriteFailed(
                        "Stretched PNG frames cannot be written to a SER container".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    fn process_fits_raw(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_dir = request.session.as_ref().map(|s| &s.dir).ok_or_else(|| {
            DiskWriterError::DirectoryCreationFailed("No active session".to_string())
        })?;

        ensure_dir(session_dir)?;

        let filename = format!("frame_{:06}.fits", request.frame_number);
        let path = session_dir.join(filename);

        debug!(path = ?path, "Writing raw FITS file");

        let raw_frame = match &request.frame_type {
            FrameType::Raw(f) => f,
            _ => {
                return Err(DiskWriterError::WriteFailed(
                    "Invalid frame type for raw write".to_string(),
                ))
            }
        };

        write_fits_from_raw(raw_frame, &path, Some(&request.metadata))
            .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Raw FITS file written successfully (16-bit)");
        Ok(())
    }

    fn process_fits_stacked(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_name = stacked_output_name(request);

        ensure_dir(&self.stacked_dir)?;

        let filename = format!("{}.fits", session_name);
        let path = self.stacked_dir.join(filename);

        debug!(path = ?path, "Writing stacked FITS file");

        let stacked_frame = match &request.frame_type {
            FrameType::Stacked(f) => f,
            _ => {
                return Err(DiskWriterError::WriteFailed(
                    "Invalid frame type for stacked write".to_string(),
                ))
            }
        };

        write_fits(stacked_frame, &path, Some(&request.metadata))
            .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Stacked FITS file written successfully");
        Ok(())
    }

    fn process_png_stacked(&self, request: &WriteRequest) -> Result<(), DiskWriterError> {
        let session_name = stacked_output_name(request);

        ensure_dir(&self.stacked_dir)?;

        let filename = format!("{}_stretched.png", session_name);
        let path = self.stacked_dir.join(filename);

        debug!(path = ?path, "Writing stretched PNG file");

        let (rgb8, width, height) = match &request.frame_type {
            FrameType::StackedPng {
                rgb8,
                width,
                height,
            } => (rgb8, *width, *height),
            _ => {
                return Err(DiskWriterError::WriteFailed(
                    "Invalid frame type for PNG write".to_string(),
                ))
            }
        };

        write_rgb8_png(rgb8, width, height, &path)
            .map_err(|e| DiskWriterError::WriteFailed(e.to_string()))?;

        debug!(path = ?path, "Stretched PNG file written successfully");
        Ok(())
    }
}
