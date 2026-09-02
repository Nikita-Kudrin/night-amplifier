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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::camera::RawFrame;
use crate::frame::Frame;
use crate::server::state::{CaptureSettings, ConnectedCameraInfo};

/// How many frames the stacking task has handed the render task that it has not
/// picked up yet.
///
/// # Why this exists
///
/// `MasterStack::compute()` copies the running mean out of the accumulator into a fresh
/// frame — 434 MB read and 108 MB allocated and written, on a 3008x3008 colour stack.
/// The stacking task ran it every iteration, and the render task then called
/// `drain_to_latest` and threw about half of them away unread. The waste landed on the
/// one thread that is dropping camera frames, and a dropped camera frame is lost sky.
///
/// `std::sync::mpsc::SyncSender` exposes no length, so the sender cannot ask the channel
/// whether the last frame was consumed. This is that length: incremented on a successful
/// send, decremented for every message taken out — including the ones `drain_to_latest`
/// discards, which is why the render task decrements per message rather than once per
/// iteration.
///
/// Reading it is advisory. A frame can be taken between the check and the send, in which
/// case the stacking task merely does work it could have skipped; nothing downstream
/// depends on the count being exact, and the render task's own drain still handles a
/// queue that grew anyway.
#[derive(Clone, Debug, Default)]
pub struct RenderQueueDepth(Arc<AtomicUsize>);

impl RenderQueueDepth {
    /// Frames queued for the render task and not yet picked up.
    pub fn pending(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    /// Record a frame handed to the render channel.
    pub fn sent(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a frame taken off the render channel.
    ///
    /// Saturating, because the count is advisory and a decrement racing a reset must not
    /// wrap into billions and pin the stacking task into never computing again.
    pub fn taken(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }
}

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
    /// The captured raw frame data (shared reference).
    pub frame: Arc<RawFrame>,
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
/// compute. `display_frame` is an `Arc<Frame>` so the raw-fallback path (stacking
/// disabled or not yet initialised, wanderer reset) can share the captured
/// allocation instead of copying it, and so plate solving can hold the same
/// frame without a second copy. The render task takes ownership back with
/// `Arc::try_unwrap` when it holds the only handle, which is the common case.
pub struct StackedFrame {
    /// The frame to display (stacked result or raw fallback).
    pub display_frame: Arc<Frame>,
    /// Whether `display_frame` is the accumulated stack rather than a single sub.
    ///
    /// Distinct from `was_stacked`: a frame that fails registration leaves the
    /// stack untouched but still displayable, so the live view keeps showing it.
    pub showing_stack: bool,
    /// Whether this frame was successfully added to the stack.
    pub was_stacked: bool,
    /// Sequential frame number within the capture session.
    pub frame_number: u64,
    /// Snapshot of capture settings (for render pipeline configuration).
    pub settings: CaptureSettings,
    /// Frames in the accumulated stack, or `0` when this is a single sub.
    ///
    /// The render task's analysis cache refreshes on proportional growth in this, not on
    /// elapsed frames, because the statistics it holds fall as `1/sqrt(N)`.
    pub stack_depth: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_counts_sends_and_takes() {
        let depth = RenderQueueDepth::default();
        assert_eq!(depth.pending(), 0);
        depth.sent();
        depth.sent();
        assert_eq!(depth.pending(), 2);
        depth.taken();
        assert_eq!(depth.pending(), 1);
        depth.taken();
        assert_eq!(depth.pending(), 0);
    }

    /// The failure mode this guards is not a wrong number, it is a **frozen live view**.
    ///
    /// The stacking task only builds a display frame when `pending()` is zero. If a
    /// decrement ever wrapped below zero it would land on `usize::MAX`, the check would
    /// never pass again, and the preview would stop updating for the rest of the session
    /// with every other part of the pipeline still running normally.
    #[test]
    fn a_take_without_a_send_cannot_wrap_the_depth() {
        let depth = RenderQueueDepth::default();
        depth.taken();
        depth.taken();
        assert_eq!(depth.pending(), 0, "depth must saturate at zero, not wrap");

        depth.sent();
        assert_eq!(depth.pending(), 1, "and must still count normally after");
    }

    /// Both ends hold clones; they have to see one count.
    #[test]
    fn clones_share_one_count() {
        let sender = RenderQueueDepth::default();
        let receiver = sender.clone();
        sender.sent();
        assert_eq!(receiver.pending(), 1);
        receiver.taken();
        assert_eq!(sender.pending(), 0);
    }

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
