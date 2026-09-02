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
use std::sync::{Arc, OnceLock};

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

/// Ceiling on the memory budget for in-flight frame queues, whatever the host has.
///
/// The budget itself is [`frame_queue_budget_bytes`]: the smaller of a fifth of RAM
/// and this. It used to be a flat 2 GB, which is a quarter of an 8 GB Pi 5 and half of
/// a 4 GB one — and the queues are not the only claim on that memory. A 3008x3008
/// colour stack holds a 434 MB accumulator, allocates a 108 MB frame per
/// `MasterStack::compute()`, and the warp and denoise paths take their own scratch on
/// top. On a 4 GB board with a 9 MP sensor that combination is an OOM risk, not merely
/// a swapping one.
pub const FRAME_QUEUE_BUDGET_CAP: usize = 1024 * 1024 * 1024;

/// Budget when the host's RAM cannot be determined.
///
/// Deliberately below the cap: an unknown host is more likely to be small than large,
/// and this is still half of the 2 GB constant it replaces.
const UNKNOWN_MEMORY_BUDGET: usize = 512 * 1024 * 1024;

/// Never go below this, whatever the host reports.
///
/// Mostly cosmetic — the `[2, 256]` clamp in [`capacity_within`] is the real floor —
/// but it keeps a misparsed or absurdly small `MemTotal` from being logged as a
/// zero-byte budget.
const MIN_BUDGET: usize = 64 * 1024 * 1024;

/// Fraction of installed RAM the queues may claim, as a divisor.
const BUDGET_DIVISOR: usize = 5;

/// The three channels the budget is shared across: capture→stacking,
/// capture→storage, stacking→render.
const PIPELINE_CHANNELS: usize = 3;

/// Memory budget for in-flight frame queues on this host.
///
/// `min(MemTotal / 5, 1 GiB)`, floored at 64 MiB, or 512 MiB when `MemTotal` cannot be
/// read. Resolved once — the value cannot change for the life of the process, and the
/// probe reads a file.
///
/// `MemTotal` rather than `MemAvailable` on purpose: available memory moves with
/// whatever else is on the board, so two capture sessions started minutes apart would
/// size their channels differently and a queue-depth report from the field would not be
/// reproducible.
pub fn frame_queue_budget_bytes() -> usize {
    static BUDGET: OnceLock<usize> = OnceLock::new();
    *BUDGET.get_or_init(|| budget_for(read_mem_total_bytes()))
}

/// [`frame_queue_budget_bytes`] for a given `MemTotal`, split out so the policy can be
/// tested against hosts this machine is not.
fn budget_for(mem_total_bytes: Option<usize>) -> usize {
    let budget = match mem_total_bytes {
        Some(total) => (total / BUDGET_DIVISOR).min(FRAME_QUEUE_BUDGET_CAP),
        None => UNKNOWN_MEMORY_BUDGET,
    };
    budget.max(MIN_BUDGET)
}

/// Installed RAM in bytes, or `None` where it cannot be read.
///
/// No `cfg(target_os)`: a host without `/proc/meminfo` fails the read and lands on the
/// unknown-memory path, which is the same answer the cfg would have given.
fn read_mem_total_bytes() -> Option<usize> {
    parse_mem_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// Pull `MemTotal` out of `/proc/meminfo` content.
///
/// Split out from the read so the parser is testable on any host.
///
/// The unit is checked rather than assumed. `/proc/meminfo` has reported kB for the
/// life of the file, but silently treating an unrecognised suffix as kB would misread
/// the budget by a factor of 1024 in whichever direction the kernel moved — reporting
/// "unknown" is the safe failure. The arithmetic runs in `u64` and saturates on the way
/// back to `usize`, because a 32-bit ARM build cannot hold a large `kB` value scaled by
/// 1024.
fn parse_mem_total(meminfo: &str) -> Option<usize> {
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let mut fields = line.split_whitespace().skip(1);
    let value: u64 = fields.next()?.parse().ok()?;
    match fields.next() {
        Some("kB") => Some(value.saturating_mul(1024).try_into().unwrap_or(usize::MAX)),
        _ => None,
    }
}

/// Frames that fit in one channel's share of `budget`, clamped to `[2, 256]`.
///
/// The lower clamp overrides the budget rather than respecting it, and has to: the
/// probe frame is sent into the stacking and storage channels before their consumer
/// threads are spawned, so a capacity that could reach zero would deadlock the start of
/// every session. Worst case on a 9 MP OSC sensor is two 18 MB raw frames in each of
/// the two capture channels plus two 108 MB display frames, ~288 MB, which every board
/// this runs on can hold.
fn capacity_within(budget: usize, frame_memory_bytes: usize) -> usize {
    let per_channel_budget = budget / PIPELINE_CHANNELS;
    let capacity = per_channel_budget / frame_memory_bytes.max(1);
    capacity.clamp(2, 256)
}

/// Channel depths for one capture session, one per payload the pipeline moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineCapacities {
    /// Depth of the two `CapturedFrame` channels (capture→stacking, capture→storage).
    pub raw: usize,
    /// Depth of the `StackedFrame` channel (stacking→render).
    pub render: usize,
}

/// Size each channel from the payload it actually carries.
///
/// The two capture channels move `Arc<RawFrame>` — sensor bytes, 18 MB for a 9 MP
/// 16-bit frame. The render channel moves a debayered f32 `Frame`, 108 MB for the same
/// sensor in colour. Sizing all three from the display frame, as this used to, made the
/// two raw channels six times shallower than the budget intended; sizing all three from
/// the raw frame would overcommit the render channel by as much in the other direction.
///
/// Both capture channels are charged the full raw size even though they share one
/// `Arc` per frame, so the real footprint is at most what this budgets for.
pub fn pipeline_capacities(raw_bytes: usize, display_bytes: usize) -> PipelineCapacities {
    capacities_within(frame_queue_budget_bytes(), raw_bytes, display_bytes)
}

/// [`pipeline_capacities`] against an explicit budget, for tests that must not depend
/// on the RAM of the machine they run on.
fn capacities_within(budget: usize, raw_bytes: usize, display_bytes: usize) -> PipelineCapacities {
    PipelineCapacities {
        raw: capacity_within(budget, raw_bytes),
        render: capacity_within(budget, display_bytes),
    }
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

    const GIB: usize = 1024 * 1024 * 1024;
    const MIB: usize = 1024 * 1024;

    /// A 9 MP OSC sensor, the shape this budget is sized for: 3008x3008 16-bit off the
    /// wire, the same frame debayered to three f32 planes.
    const RAW_9MP: usize = 3008 * 3008 * 2;
    const DISPLAY_9MP: usize = 3008 * 3008 * 3 * 4;

    fn meminfo(mem_total_line: &str) -> String {
        format!(
            "MemTotal:{mem_total_line}\nMemFree:          123456 kB\nBuffers:            8888 kB\n"
        )
    }

    #[test]
    fn mem_total_parses_a_real_meminfo() {
        // Trimmed from a 8 GB Raspberry Pi 5.
        let meminfo = "MemTotal:        8244936 kB\n\
                       MemFree:         6119912 kB\n\
                       MemAvailable:    7548292 kB\n\
                       Buffers:           50692 kB\n";
        assert_eq!(parse_mem_total(meminfo), Some(8_244_936 * 1024));
    }

    /// The budget is deliberately independent of what else is running on the board, so
    /// a much lower `MemAvailable` sitting right beside it must not be picked up.
    #[test]
    fn mem_total_ignores_mem_available() {
        let meminfo = "MemAvailable:     512000 kB\nMemTotal:        4096000 kB\n";
        assert_eq!(parse_mem_total(meminfo), Some(4_096_000 * 1024));
    }

    /// Assuming kB would misread this by 1024x. "Unknown" is the safe answer.
    #[test]
    fn mem_total_rejects_an_unknown_unit() {
        assert_eq!(parse_mem_total(&meminfo("        4096 MB")), None);
        assert_eq!(parse_mem_total(&meminfo("        4096")), None);
    }

    #[test]
    fn mem_total_handles_input_it_cannot_read() {
        assert_eq!(parse_mem_total(""), None, "empty file");
        assert_eq!(
            parse_mem_total("MemFree: 123 kB\n"),
            None,
            "no MemTotal line"
        );
        assert_eq!(
            parse_mem_total(&meminfo("        abc kB")),
            None,
            "non-numeric value"
        );
        assert_eq!(parse_mem_total("MemTotal:\n"), None, "value missing");
        assert_eq!(
            parse_mem_total("MemTotalSwap:    4096 kB\n"),
            None,
            "MemTotal is a prefix of another key"
        );
    }

    /// `value * 1024` does not fit a 32-bit `usize`, and this runs on 32-bit ARM.
    #[test]
    fn mem_total_saturates_instead_of_overflowing() {
        let huge = format!("MemTotal: {} kB\n", u64::MAX);
        assert_eq!(parse_mem_total(&huge), Some(usize::MAX));
    }

    #[test]
    fn the_cap_wins_on_a_large_host() {
        assert_eq!(budget_for(Some(8 * GIB)), GIB);
        assert_eq!(budget_for(Some(32 * GIB)), GIB);
    }

    /// The case the flat 2 GB constant got wrong: half the board's RAM for queues alone.
    #[test]
    fn twenty_percent_wins_on_a_small_host() {
        assert_eq!(budget_for(Some(4 * GIB)), 4 * GIB / 5);
        assert!(budget_for(Some(4 * GIB)) < GIB);
    }

    #[test]
    fn a_tiny_host_lands_on_the_floor() {
        assert_eq!(budget_for(Some(256 * MIB)), 64 * MIB);
        assert_eq!(budget_for(Some(0)), 64 * MIB);
    }

    #[test]
    fn unknown_memory_falls_back_to_512_mib() {
        assert_eq!(budget_for(None), 512 * MIB);
    }

    /// The property, not the three sampled points: whatever the host reports, the
    /// queues never claim more than a fifth of it and never more than the cap.
    #[test]
    fn the_budget_never_exceeds_a_fifth_of_ram_or_the_cap() {
        for total in [
            64 * MIB,
            512 * MIB,
            GIB,
            2 * GIB,
            4 * GIB,
            8 * GIB,
            64 * GIB,
            usize::MAX,
        ] {
            let budget = budget_for(Some(total));
            assert!(
                budget <= FRAME_QUEUE_BUDGET_CAP,
                "{total} bytes of RAM produced a {budget} byte budget, over the cap"
            );
            assert!(
                budget <= total / 5 || budget == 64 * MIB,
                "{total} bytes of RAM produced {budget}, over a fifth, without being the floor"
            );
        }
    }

    /// The host's own answer has to obey the same bounds — this is the only test that
    /// touches the real `/proc/meminfo`, and the only one whose input CI controls.
    #[test]
    fn the_resolved_budget_is_within_bounds_on_this_host() {
        let budget = frame_queue_budget_bytes();
        assert!((64 * MIB..=FRAME_QUEUE_BUDGET_CAP).contains(&budget));
        assert_eq!(
            budget,
            frame_queue_budget_bytes(),
            "the cache is not stable"
        );
    }

    /// The defect this split exists for.
    ///
    /// The two capture channels carry `Arc<RawFrame>` — 18 MB — and the render channel
    /// carries the debayered f32 frame at 108 MB. Sizing all three from the display
    /// frame made the capture channels 6x shallower than the budget intended, and a
    /// capture channel that fills is a dropped frame, which is lost sky.
    #[test]
    fn the_raw_channels_are_deeper_than_the_render_channel() {
        let caps = capacities_within(GIB, RAW_9MP, DISPLAY_9MP);
        assert!(
            caps.raw > caps.render,
            "raw {} vs render {}: the raw channels are still sized off the display frame",
            caps.raw,
            caps.render
        );
        assert_eq!(caps.raw, capacity_within(GIB, RAW_9MP));
        assert_eq!(caps.render, capacity_within(GIB, DISPLAY_9MP));
    }

    /// Superpixel debayering makes the display frame a quarter of a full demosaic, so
    /// the render channel gets deeper — but the sensor still puts the same bytes on the
    /// wire and the capture channels must not move with it.
    #[test]
    fn a_smaller_display_frame_does_not_resize_the_raw_channels() {
        let full = capacities_within(GIB, RAW_9MP, DISPLAY_9MP);
        let superpixel = capacities_within(GIB, RAW_9MP, DISPLAY_9MP / 4);
        assert_eq!(full.raw, superpixel.raw);
        assert!(superpixel.render > full.render);
    }

    /// Not a nicety: the probe frame is sent into the stacking and storage channels
    /// before their consumer threads are spawned, so a capacity that could reach zero
    /// would deadlock the start of every session.
    #[test]
    fn capacity_is_never_below_two() {
        for budget in [0, 64 * MIB, 512 * MIB, GIB] {
            for frame_bytes in [0, 4, 24 * MIB, GIB, usize::MAX] {
                let capacity = capacity_within(budget, frame_bytes);
                assert!(
                    capacity >= 2,
                    "budget {budget} with {frame_bytes} byte frames gave {capacity}"
                );
                assert!(
                    capacity <= 256,
                    "budget {budget} with {frame_bytes} byte frames gave {capacity}"
                );
            }
        }
    }

    /// The claim the budget is for, checked as arithmetic rather than asserted in a
    /// comment: everything the three queues can hold at once on the smallest board this
    /// targets, plus the stacking allocations that share the board with them.
    ///
    /// The queues are not the whole picture — a 3008x3008 colour stack also holds a
    /// 434 MB accumulator and allocates a 108 MB frame per `MasterStack::compute()` —
    /// so the test that matters is the sum, against the RAM of a 4 GB Pi 5. The old
    /// 2 GB constant put the queues alone at half the board before any of this.
    #[test]
    fn the_worst_case_queue_footprint_fits_a_4gb_board() {
        let ram = 4 * GIB;
        let budget = budget_for(Some(ram));
        let caps = capacities_within(budget, RAW_9MP, DISPLAY_9MP);

        // Both capture channels charged in full, though they share one `Arc` per frame.
        let queues = 2 * caps.raw * RAW_9MP + caps.render * DISPLAY_9MP;
        assert!(
            queues <= budget,
            "{queues} bytes of queue against a {budget} byte budget"
        );

        const ACCUMULATOR: usize = 434 * MIB;
        let peak = queues + ACCUMULATOR + DISPLAY_9MP;
        assert!(
            peak < ram / 2,
            "{peak} bytes at peak on a 4 GB board leaves too little headroom"
        );
    }

    #[test]
    fn a_frame_larger_than_the_budget_gets_the_minimum() {
        assert_eq!(capacity_within(GIB, GIB), 2);
        assert_eq!(capacity_within(GIB, usize::MAX), 2);
    }

    /// Division by `max(1)` rather than a panic; a zero-byte frame is a bug upstream,
    /// but it must not take the capture session down.
    #[test]
    fn a_zero_byte_frame_clamps_rather_than_dividing_by_zero() {
        assert_eq!(capacity_within(GIB, 0), 256);
        assert_eq!(capacity_within(0, 0), 2);
    }

    #[test]
    fn a_typical_frame_divides_the_channel_share() {
        // 1920x1080 RGB f32, ~24 MB, against a 1 GiB budget: 341 MB per channel.
        let frame_bytes = 1920 * 1080 * 3 * 4;
        assert_eq!(capacity_within(GIB, frame_bytes), 14);
    }

    #[test]
    fn a_tiny_frame_clamps_to_the_upper_bound() {
        assert_eq!(capacity_within(GIB, 64 * 64 * 4), 256);
    }
}
