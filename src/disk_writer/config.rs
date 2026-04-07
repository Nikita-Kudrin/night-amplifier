use crate::fits::FitsMetadata;
use crate::frame::Frame;
use std::path::PathBuf;

/// Maximum queue depth before warning
pub const QUEUE_WARNING_THRESHOLD: usize = 5;

/// Session type determines the storage format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WritingSessionType {
    /// Individual FITS files (Deep Sky, Comet)
    #[default]
    IndividualFrames,
    /// Video container (SER for Planetary)
    VideoContainer,
}

/// Type of frame being saved
#[derive(Debug, Clone)]
pub enum FrameType {
    /// Raw captured frame (FITS or SER depending on session type)
    Raw,
    /// Stacked result frame (FITS)
    Stacked,
    /// Stretched stacked frame (PNG for sharing)
    StackedPng,
}

/// A request to write a frame to disk
#[derive(Debug, Clone)]
pub struct WriteRequest {
    /// The frame data to write
    pub frame: Frame,
    /// Type of frame
    pub frame_type: FrameType,
    /// Frame number (for raw frames)
    pub frame_number: u64,
    /// Metadata for FITS headers
    pub metadata: FitsMetadata,
}

/// Message sent to the disk writer worker
#[derive(Debug)]
pub enum DiskWriterMessage {
    /// Start a new capture session
    StartSession {
        /// Directory for the session
        path: PathBuf,
        /// Type of session (Individual or Video)
        session_type: WritingSessionType,
    },
    /// Queue a frame for writing
    WriteFrame(WriteRequest),
    /// End the current capture session
    EndSession,
}

/// Configuration for the disk writer
#[derive(Debug, Clone)]
pub struct DiskWriterConfig {
    /// Base directory for captures (default: "./captures")
    pub base_dir: PathBuf,
    /// Maximum queue size (default: 20)
    pub max_queue_size: usize,
    /// Whether saving is enabled
    pub enabled: bool,
}

impl Default for DiskWriterConfig {
    fn default() -> Self {
        Self {
            base_dir: PathBuf::from("captures"),
            max_queue_size: 20,
            enabled: true,
        }
    }
}

impl DiskWriterConfig {
    /// Create a new configuration with the specified base directory
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            ..Default::default()
        }
    }

    /// Set maximum queue size
    pub fn with_max_queue_size(mut self, size: usize) -> Self {
        self.max_queue_size = size;
        self
    }

    /// Set enabled state
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}
