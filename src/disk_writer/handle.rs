use chrono::Local;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use tracing::{info, warn};

use super::config::{
    DiskWriterMessage, FrameType, WriteRequest, WritingSessionType, QUEUE_WARNING_THRESHOLD,
};
use super::error::DiskWriterError;
use crate::camera::RawFrame;
use crate::fits::FitsMetadata;
use crate::frame::Frame;
use crate::telemetry::metrics as telemetry_metrics;

/// The capture session currently open for writing.
///
/// Directory and container type are one fact and are read as one: every queued frame is
/// stamped with both, so the worker never has to consult live state to decide where a
/// frame goes or what format it takes. That is what lets a session be rolled while the
/// queue still holds frames from the session before it.
#[derive(Debug, Clone)]
pub struct OpenSession {
    pub dir: PathBuf,
    pub session_type: WritingSessionType,
}

/// Handle to the disk writer for sending write requests
#[derive(Clone)]
pub struct DiskWriterHandle {
    /// Channel sender for write requests
    pub(crate) sender: mpsc::SyncSender<DiskWriterMessage>,
    /// Current queue depth
    pub(crate) queue_depth: Arc<AtomicUsize>,
    /// Warning flag for queue overflow
    pub(crate) queue_warning: Arc<AtomicBool>,
    /// The open capture session, if any
    pub(crate) session: Arc<RwLock<Option<OpenSession>>>,
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
    ///
    /// `name_suffix` is appended to the timestamp so the folder records which capture
    /// mode filled it. The caller owns the vocabulary — `disk_writer` has no notion of
    /// capture modes and takes the string as given.
    pub fn start_session(
        &self,
        session_type: WritingSessionType,
        name_suffix: &str,
    ) -> std::io::Result<PathBuf> {
        let session = self.create_session(session_type, name_suffix)?;
        let path = session.dir.clone();
        self.open(session.dir, session.session_type);

        info!(session_dir = ?path, ?session_type, "Started new capture session");
        Ok(path)
    }

    /// Create a session directory and hand back the session **without** publishing it
    /// as the handle's current one.
    ///
    /// For a producer that owns its own session for its whole life rather than sharing
    /// the handle's slot — the guide loop, which runs alongside a main capture that has
    /// its own directory open. Every `WriteRequest` already carries the session it
    /// belongs to (see [`super::config::WriteRequest::session`]), so the worker files
    /// both correctly; the shared slot is the only thing that assumed one at a time.
    pub fn create_session(
        &self,
        session_type: WritingSessionType,
        name_suffix: &str,
    ) -> std::io::Result<OpenSession> {
        let raw_dir = self
            .stacked_dir
            .parent()
            .unwrap_or(Path::new("."))
            .join("raw");
        let timestamp = Local::now().format("%d-%m-%Y_%H-%M-%S").to_string();
        let session_path = unused_session_path(&raw_dir, &timestamp, name_suffix);

        std::fs::create_dir_all(&session_path)?;
        Ok(OpenSession {
            dir: session_path,
            session_type,
        })
    }

    /// Reopen an existing directory as an unpublished session, for a producer resuming
    /// after a dropout. Counterpart to [`Self::create_session`].
    pub fn reopen_session(
        &self,
        dir: PathBuf,
        session_type: WritingSessionType,
    ) -> std::io::Result<OpenSession> {
        std::fs::create_dir_all(&dir)?;
        Ok(OpenSession { dir, session_type })
    }

    /// Publish the session that frames queued from now on belong to.
    fn open(&self, dir: PathBuf, session_type: WritingSessionType) {
        *self.session.write().unwrap_or_else(|e| e.into_inner()) =
            Some(OpenSession { dir, session_type });
    }

    /// Reopen an existing session directory instead of creating a new one.
    ///
    /// Used when a capture resumes after an automatic reconnect: the frames on
    /// either side of the dropout belong to one observation, and a fresh
    /// `start_session` would scatter them across timestamped folders.
    pub fn resume_session(
        &self,
        session_path: PathBuf,
        session_type: WritingSessionType,
    ) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&session_path)?;
        self.open(session_path.clone(), session_type);
        info!(session_dir = ?session_path, ?session_type, "Resumed capture session");
        Ok(session_path)
    }

    /// Start a session only if saving is on and none is open.
    ///
    /// `POST /api/settings` can turn saving on partway through a capture, long
    /// after `initialize_capture_session` ran. Without this the writer accepted
    /// frames it had nowhere to put and logged "No active session" for every
    /// one of them.
    pub fn ensure_session(
        &self,
        session_type: WritingSessionType,
        name_suffix: &str,
    ) -> std::io::Result<()> {
        if !self.is_enabled() || self.session_dir().is_some() {
            return Ok(());
        }
        self.start_session(session_type, name_suffix)?;
        Ok(())
    }

    /// End the current capture session, finalizing a SER container if one is open.
    ///
    /// Blocks until the worker has been told. Finalization is what writes a SER's frame
    /// count into its header, so a container whose `EndSession` went missing is
    /// unreadable — this used to go out with `try_send` and be dropped silently
    /// whenever the disk had fallen behind. Call it from a blocking context: at capture
    /// end, which is the only place it belongs, the producing threads have already been
    /// joined, so the queue is draining and the wait is bounded by one frame write.
    pub fn end_session(&self) -> Option<PathBuf> {
        let session = self.take_session();
        if session.is_some() && self.sender.send(DiskWriterMessage::EndSession).is_err() {
            warn!("Disk writer stopped before the session could be ended");
        }
        session.map(|s| s.dir)
    }

    /// Stop writing into the open session without ending it.
    ///
    /// For rolling a session mid-capture, where the wait `end_session` accepts would
    /// land on an API request thread. Nothing is lost by not signalling: the frames
    /// queued so far still carry the old directory, and the worker closes out a SER
    /// container as soon as it sees a frame belonging to a different session — or at
    /// capture end, whichever comes first.
    pub fn abandon_session(&self) -> Option<PathBuf> {
        self.take_session().map(|s| s.dir)
    }

    fn take_session(&self) -> Option<OpenSession> {
        self.session
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Get the current session directory
    pub fn session_dir(&self) -> Option<PathBuf> {
        self.session
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.dir.clone())
    }

    /// Get the session name (directory name) for the current session
    pub fn session_name(&self) -> Option<String> {
        self.session_dir()
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
    }

    /// Queue a frame for writing
    ///
    /// Stamps the request with the session that is open now — directory and container
    /// type both, overwriting whatever the caller put there. The worker files the frame
    /// accordingly whenever it gets to it, so a session rolled in between — a mid-capture
    /// mode change — cannot pull a queued frame into the folder of a mode it was not
    /// captured in, nor write it in a format that session never used.
    pub fn queue_frame(&self, request: WriteRequest) -> Result<bool, DiskWriterError> {
        let session = self
            .session
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.queue_frame_in(session, request)
    }

    /// Queue a frame against a session the caller owns, rather than the handle's shared
    /// one.
    ///
    /// The guide loop holds its own session for the life of the connection while a main
    /// capture holds the shared one; both write through here and the worker keeps them in
    /// separate directories, because the session travels on the request.
    pub fn queue_frame_in(
        &self,
        session: Option<OpenSession>,
        mut request: WriteRequest,
    ) -> Result<bool, DiskWriterError> {
        if !self.is_enabled() {
            return Ok(false);
        }

        request.session = session;

        let depth = self.queue_depth.fetch_add(1, Ordering::SeqCst) + 1;
        telemetry_metrics::record_disk_writer_queue_depth(depth as u64);

        if depth > QUEUE_WARNING_THRESHOLD {
            // `queue_warning` is a latch the consumer clears once the backlog drains, so
            // swapping it reports the *start* of an episode rather than every frame
            // inside one. At the short exposures raw-saving Live view allows, the latter
            // was a line per frame for as long as the disk stayed behind.
            if !self.queue_warning.swap(true, Ordering::SeqCst) {
                warn!(
                    queue_depth = depth,
                    "Disk writer queue depth exceeds threshold"
                );
            }
        }

        let message = DiskWriterMessage::WriteFrame(request);
        match self.sender.try_send(message) {
            Ok(()) => Ok(true),
            // Not logged here: every caller reports the returned error, and the raw
            // path rate-limits it. Logging both put two unthrottled lines in the log for
            // each frame a busy disk turned away.
            Err(mpsc::TrySendError::Full(_)) => {
                self.queue_depth.fetch_sub(1, Ordering::SeqCst);
                telemetry_metrics::record_disk_writer_queue_depth(
                    self.queue_depth.load(Ordering::SeqCst) as u64,
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

    /// Queue a raw frame for writing into the handle's current session.
    pub fn queue_raw_frame(
        &self,
        frame: std::sync::Arc<RawFrame>,
        frame_number: u64,
        metadata: FitsMetadata,
        sensor_type: crate::camera::SensorType,
        bayer_pattern: Option<crate::CfaPattern>,
    ) -> Result<bool, DiskWriterError> {
        let session = self
            .session
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        self.queue_raw_frame_in(session, frame, frame_number, metadata, sensor_type, bayer_pattern)
    }

    /// Queue a raw frame into a session the caller owns. See [`Self::queue_frame_in`].
    pub fn queue_raw_frame_in(
        &self,
        session: Option<OpenSession>,
        frame: std::sync::Arc<RawFrame>,
        frame_number: u64,
        metadata: FitsMetadata,
        sensor_type: crate::camera::SensorType,
        bayer_pattern: Option<crate::CfaPattern>,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame_in(
            session,
            WriteRequest {
                frame_type: FrameType::Raw(frame),
                session: None,
                frame_number,
                metadata,
                sensor_type,
                bayer_pattern,
            },
        )
    }

    pub fn queue_stacked_frame(
        &self,
        frame: std::sync::Arc<Frame>,
        metadata: FitsMetadata,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame(WriteRequest {
            frame_type: FrameType::Stacked(frame),
            session: None,
            frame_number: 0,
            metadata,
            sensor_type: crate::camera::SensorType::Color,
            bayer_pattern: None,
        })
    }

    /// Queue an already-rendered stretched PNG for writing.
    ///
    /// `rgb8` must be interleaved 8-bit RGB at `width` x `height`, produced by
    /// `crate::server::encoding::frame_to_rgb8_downsampled` (or an equivalent
    /// call through the same encoder) so the file matches what the live view
    /// rendered — see `FrameType::StackedPng`.
    pub fn queue_stacked_png(
        &self,
        rgb8: std::sync::Arc<Vec<u8>>,
        width: u32,
        height: u32,
        stacked_count: u64,
    ) -> Result<bool, DiskWriterError> {
        self.queue_frame(WriteRequest {
            frame_type: FrameType::StackedPng {
                rgb8,
                width,
                height,
            },
            session: None,
            frame_number: stacked_count,
            metadata: FitsMetadata::new(),
            sensor_type: crate::camera::SensorType::Color,
            bayer_pattern: None,
        })
    }
}

/// A session path under `raw_dir` that no earlier session is already using.
///
/// The timestamp has one-second resolution, and a session can be rolled far faster than
/// that — flipping capture mode twice inside a second used to land back on the first
/// folder. `create_dir_all` succeeds silently on an existing directory, so the two
/// segments merged, and for a planetary session `SerWriter::create` truncated the
/// `capture.ser` the first one had written.
///
/// The counter goes *before* the suffix so the name still ends with the mode it names —
/// `CaptureMode::from_session_dir_name` reads it with `ends_with`.
fn unused_session_path(raw_dir: &Path, timestamp: &str, name_suffix: &str) -> PathBuf {
    let first = raw_dir.join(format!("{}{}", timestamp, name_suffix));
    if !first.exists() {
        return first;
    }
    for n in 2..=u32::MAX {
        let candidate = raw_dir.join(format!("{}_{}{}", timestamp, n, name_suffix));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}
