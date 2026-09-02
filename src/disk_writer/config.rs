use std::path::PathBuf;

use super::handle::{DiskWriterHandle, OpenSession};
use crate::camera::RawFrame;
use crate::fits::FitsMetadata;
use crate::frame::Frame;
use std::sync::Arc;

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

/// Type of frame being saved, along with its payload
#[derive(Clone)]
pub enum FrameType {
    /// Raw captured frame (FITS or SER depending on session type)
    Raw(Arc<RawFrame>),
    /// Stacked result frame (FITS)
    Stacked(Arc<Frame>),
    /// Stretched stacked frame, already rendered to interleaved 8-bit RGB (PNG for sharing).
    ///
    /// Carries finished bytes rather than a `Frame`: they come from
    /// `crate::server::encoding::frame_to_rgb8_downsampled`, the same conversion
    /// the live view streams through, so the saved file matches on-screen output
    /// instead of skipping the encoder-only stages (spatial denoise, display
    /// quantization) a bare `Frame` render would miss. Rendering happens in
    /// `server::capture::storage` — a server-layer concern — and only the
    /// resulting bytes cross into this module, keeping `disk_writer` itself
    /// unaware of the render/encoding pipeline.
    StackedPng {
        rgb8: Arc<Vec<u8>>,
        width: u32,
        height: u32,
    },
}

/// Hand-written rather than derived: `Raw`/`Stacked` delegate to `RawFrame`/`Frame`,
/// which already print `data_len` instead of their pixel buffers (see their own
/// `Debug` impls) — but a derive can't apply that same care to `StackedPng`'s bare
/// `Arc<Vec<u8>>`. `Vec<u8>` has no such override, so a derived impl here would print
/// every byte of a multi-megapixel RGB buffer on every `#[instrument]`-logged write —
/// which is exactly what happened before this impl existed: one call to
/// `process_request` was enough to put a multi-megabyte line in the log.
impl std::fmt::Debug for FrameType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(frame) => f.debug_tuple("Raw").field(frame).finish(),
            Self::Stacked(frame) => f.debug_tuple("Stacked").field(frame).finish(),
            Self::StackedPng {
                rgb8,
                width,
                height,
            } => f
                .debug_struct("StackedPng")
                .field("rgb8_len", &rgb8.len())
                .field("width", width)
                .field("height", height)
                .finish(),
        }
    }
}

/// A request to write a frame to disk
#[derive(Debug, Clone)]
pub struct WriteRequest {
    /// Type of frame and its payload
    pub frame_type: FrameType,
    /// The session that was open when this frame was queued.
    ///
    /// Filled in by [`DiskWriterHandle::queue_frame`]; whatever a caller sets here is
    /// overwritten, since only the handle knows what is open at that moment.
    ///
    /// Resolved at enqueue time rather than at write time, so a session rolled while
    /// the queue still holds frames — which is what a mid-capture mode change does —
    /// cannot retarget frames that belong to the session before it, nor write them in
    /// the wrong container. `None` only when nothing was open, which is an error for a
    /// raw frame and a fallback to a bare timestamp for a stacked one.
    pub session: Option<OpenSession>,
    /// Frame number (for raw frames)
    pub frame_number: u64,
    /// Metadata for FITS headers
    pub metadata: FitsMetadata,
    /// Sensor type of the camera that captured this frame
    pub sensor_type: crate::camera::SensorType,
    /// Bayer pattern of the camera that captured this frame (if applicable)
    pub bayer_pattern: Option<crate::CfaPattern>,
}

/// Message sent to the disk writer worker
///
/// There is no "start session" message: every frame carries the session it belongs to,
/// so the worker learns of a new one from the first frame written into it. Only the
/// *end* of a session needs announcing, and only because a SER container that is never
/// finalized is unreadable.
#[derive(Debug)]
pub enum DiskWriterMessage {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: `FrameType::StackedPng` used to derive `Debug`, which for a
    /// bare `Arc<Vec<u8>>` prints every byte — a multi-megapixel RGB buffer turned one
    /// `#[instrument]`-logged write into a multi-megabyte log line. The custom impl
    /// must report a length instead of walking the buffer.
    #[test]
    fn stacked_png_debug_reports_length_not_bytes() {
        let rgb8 = vec![7u8; 2712 * 1538 * 3];
        let len = rgb8.len();
        let frame_type = FrameType::StackedPng {
            rgb8: Arc::new(rgb8),
            width: 2712,
            height: 1538,
        };

        let debug_str = format!("{:?}", frame_type);
        assert!(
            debug_str.len() < 200,
            "StackedPng's Debug output is {} bytes long — it is walking the pixel \
             buffer instead of reporting its length: {debug_str:.200}",
            debug_str.len()
        );
        assert!(debug_str.contains(&len.to_string()));
        assert!(!debug_str.contains("7, 7, 7"));
    }
}
