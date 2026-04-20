//! Channel message types for the decoupled capture pipeline
//!
//! The capture pipeline is decomposed into four independent tasks connected
//! by bounded MPSC channels:
//!
//! - **CaptureTask** → `CapturedFrame` → **StackingTask** and **StorageTask**
//! - **StackingTask** → `StackedFrame` → **RenderTask**
//!
//! `Arc<Frame>` is used to share frame data between the stacking and storage
//! channels without memory duplication.

use std::sync::Arc;

use crate::frame::Frame;
use crate::server::state::{CaptureSettings, ConnectedCameraInfo};

/// Maximum memory budget for in-flight frame queues (2 GB).
///
/// This budget is shared across the three channels (capture→stacking,
/// capture→storage, stacking→render). Each channel gets one third of
/// the budget, and the per-channel capacity is calculated based on the
/// actual frame size from the camera sensor.
pub const MAX_FRAME_QUEUE_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Calculate the maximum number of frames that fit in a single channel's
/// share of the memory budget.
///
/// The total budget is divided equally across 3 channels. Each channel's
/// capacity is clamped to `[2, 256]` frames.
pub fn max_queue_capacity(frame_memory_bytes: usize) -> usize {
    let per_channel_budget = MAX_FRAME_QUEUE_MEMORY_BYTES / 3;
    let capacity = per_channel_budget / frame_memory_bytes.max(1);
    capacity.clamp(2, 256)
}

/// A frame captured from the camera, sent through channels to downstream tasks.
///
/// Uses `Arc<Frame>` so the same allocation is shared between the stacking
/// and storage channels without cloning the pixel data.
pub struct CapturedFrame {
    /// The captured frame data (shared reference).
    pub frame: Arc<Frame>,
    /// Sequential frame number within the capture session.
    pub frame_number: u64,
    /// Snapshot of capture settings at the time of capture.
    pub settings: CaptureSettings,
    /// Camera info for metadata (disk saving, etc.).
    pub camera_info: ConnectedCameraInfo,
}

/// A processed frame ready for preview rendering and streaming.
///
/// Produced by the stacking task after registration, accumulation, and
/// compute. The `display_frame` is owned because stacking produces a new
/// frame (either the computed stack or a raw fallback).
pub struct StackedFrame {
    /// The frame to display (stacked result or raw fallback).
    pub display_frame: Frame,
    /// Whether this frame was successfully added to the stack.
    pub was_stacked: bool,
    /// Sequential frame number within the capture session.
    pub frame_number: u64,
    /// Snapshot of capture settings (for render pipeline configuration).
    pub settings: CaptureSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_queue_capacity_typical_frame() {
        // 1920x1080 RGB, f32: ~24 MB
        let frame_bytes = 1920 * 1080 * 3 * 4;
        let capacity = max_queue_capacity(frame_bytes);
        // ~682 MB per channel / 24 MB = ~28 frames
        assert!(capacity >= 2);
        assert!(capacity <= 256);
        assert_eq!(capacity, 28);
    }

    #[test]
    fn test_max_queue_capacity_4k_frame() {
        // 4144x2822 RGB, f32: ~140 MB (large astro sensor)
        let frame_bytes = 4144 * 2822 * 3 * 4;
        let capacity = max_queue_capacity(frame_bytes);
        // ~682 MB / 140 MB = ~4 frames
        assert!(capacity >= 2);
        assert!(capacity <= 256);
    }

    #[test]
    fn test_max_queue_capacity_tiny_frame() {
        // 64x64 mono, f32: 16 KB
        let frame_bytes = 64 * 64 * 1 * 4;
        let capacity = max_queue_capacity(frame_bytes);
        // Would be huge — clamped to 256
        assert_eq!(capacity, 256);
    }

    #[test]
    fn test_max_queue_capacity_zero_frame() {
        let capacity = max_queue_capacity(0);
        // Division by max(1) prevents panic, clamped to 256
        assert_eq!(capacity, 256);
    }

    #[test]
    fn test_max_queue_capacity_single_pixel() {
        // 1x1 mono, f32: 4 bytes
        let frame_bytes = 1 * 1 * 1 * 4;
        let capacity = max_queue_capacity(frame_bytes);
        assert_eq!(capacity, 256);
    }

    #[test]
    fn test_max_queue_capacity_huge_frame() {
        // Extremely large frame that exceeds per-channel budget
        let frame_bytes = 1024 * 1024 * 1024; // 1 GB
        let capacity = max_queue_capacity(frame_bytes);
        // ~682 MB / 1 GB = 0, clamped to 2
        assert_eq!(capacity, 2);
    }

    #[test]
    fn test_max_queue_capacity_minimum_guarantee() {
        // Even when the budget is tight, we always get at least 2 frames
        let frame_bytes = MAX_FRAME_QUEUE_MEMORY_BYTES; // frame = entire budget
        let capacity = max_queue_capacity(frame_bytes);
        assert_eq!(capacity, 2);
    }
}
