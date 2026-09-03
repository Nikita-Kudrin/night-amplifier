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

/// How many messages one pipeline channel holds that its consumer has not taken yet.
/// `SyncSender` exposes no length, so this tracks it alongside: incremented *before*
/// a send is attempted (given back if it fails), decremented for every message taken
/// out — including ones a drain discards.
///
/// Two callers, different reasons: stacking→render reads it to decide whether to
/// build a display copy at all (`MasterStack::compute()`'s copy is 434MB read + 108MB
/// written on a 3008² colour stack, and the render task used to `drain_to_latest` and
/// throw half away unread — waste landing on the thread dropping camera frames, i.e.
/// lost sky). The two capture channels only *report* depth, distinguishing "slow"
/// from "stalled once".
///
/// Increment leads the send because the count must never sit *below* the true depth:
/// for the render channel that's unrecoverable — `want_display` is `pending() == 0`,
/// so a count stuck at one on an empty channel stops the stacking task from ever
/// building another display frame, which stops the render task from ever
/// decrementing again. Counting after `try_send` leaves exactly that window; counting
/// first can only overshoot, costing one skipped display copy the next iteration
/// corrects. Reading it is advisory — nothing downstream depends on it being exact.
#[derive(Clone, Debug, Default)]
pub struct QueueDepth(Arc<AtomicUsize>);

impl QueueDepth {
    /// Messages queued on this channel and not yet picked up.
    pub fn pending(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }

    /// Claim a slot, before attempting the send that fills it.
    pub fn sent(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// Give a slot back: one message taken off the channel, or one send that did not
    /// happen.
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

/// Longest the capture→stacking channel may hold buffered sky, in microseconds. Memory
/// budget alone answers "how many frames fit", not what an observer cares about: a
/// deeper queue doesn't fix a throughput deficit, it just runs the stack further behind
/// the shutter for the same drop rate (memory alone puts 19 frames / 2.9s of lag on a
/// 20-core, 62.5GiB host at 152ms exposures). Two seconds is what a live view can
/// absorb without visibly trailing the mount.
const MAX_STACKING_QUEUE_LATENCY_US: u64 = 2_000_000;

/// Channel depths for one capture session, one per channel the pipeline runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PipelineCapacities {
    /// Depth of the capture→stacking `CapturedFrame` channel.
    ///
    /// Bounded by [`MAX_STACKING_QUEUE_LATENCY_US`] as well as by memory: every frame
    /// queued here is a frame of lag between the shutter and the live view.
    pub stacking: usize,
    /// Depth of the capture→storage `CapturedFrame` channel.
    ///
    /// Memory only. Depth here is disk backlog, not lag — nobody is watching the FITS
    /// files land, and a frame dropped for want of a slot is lost sky that no amount of
    /// promptness recovers.
    pub storage: usize,
    /// Depth of the stacking→render `StackedFrame` channel.
    ///
    /// Memory only, and largely academic: `QueueDepth` gates the stacking task to one
    /// in-flight display frame, so this is a ceiling rather than an operating point.
    pub render: usize,
}

/// Size each channel from the payload it actually carries, and the stacking channel
/// from the latency it would introduce too. Capture channels move `Arc<RawFrame>`
/// (18MB for 9MP 16-bit); the render channel moves a debayered f32 `Frame` (108MB
/// colour). Sizing all three from the display frame (the old approach) made the raw
/// channels 6x shallower than intended; sizing from the raw frame would overcommit
/// render by as much the other way. Both capture channels are charged the full raw
/// size despite sharing one `Arc`, so real footprint is at most what this budgets.
pub fn pipeline_capacities(
    raw_bytes: usize,
    display_bytes: usize,
    exposure_us: u64,
) -> PipelineCapacities {
    capacities_within(
        frame_queue_budget_bytes(),
        raw_bytes,
        display_bytes,
        exposure_us,
    )
}

/// [`pipeline_capacities`] against an explicit budget, for tests that must not depend
/// on the RAM of the machine they run on.
fn capacities_within(
    budget: usize,
    raw_bytes: usize,
    display_bytes: usize,
    exposure_us: u64,
) -> PipelineCapacities {
    let storage = capacity_within(budget, raw_bytes);
    PipelineCapacities {
        stacking: storage.min(frames_within_latency(exposure_us)).max(2),
        storage,
        render: capacity_within(budget, display_bytes),
    }
}

/// Frames that fit inside [`MAX_STACKING_QUEUE_LATENCY_US`] at this exposure.
///
/// A zero or missing exposure yields `usize::MAX` rather than zero: an unknown cadence
/// must fall back to the memory budget, not collapse the channel. The `[2, ..]` floor is
/// applied by the caller for the same reason [`capacity_within`] applies it — the probe
/// frame is sent before the consumer threads exist.
fn frames_within_latency(exposure_us: u64) -> usize {
    if exposure_us == 0 {
        return usize::MAX;
    }
    (MAX_STACKING_QUEUE_LATENCY_US / exposure_us)
        .try_into()
        .unwrap_or(usize::MAX)
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
        let depth = QueueDepth::default();
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
        let depth = QueueDepth::default();
        depth.taken();
        depth.taken();
        assert_eq!(depth.pending(), 0, "depth must saturate at zero, not wrap");

        depth.sent();
        assert_eq!(depth.pending(), 1, "and must still count normally after");
    }

    /// The gate must not latch shut when a take lands between the send and the count.
    /// This is the ordering `run_stacking_task` used to have — `try_send(msg)` then
    /// `render_depth.sent()` — where a receiver waking in that window ran `taken()`
    /// against a zero counter, saturated there, and the sender's `sent()` landed on an
    /// already-empty channel. Damage was permanent: `want_display` is `pending() == 0`,
    /// so the stacking task stopped building display frames and the live view froze for
    /// the rest of the session. Counting first makes the interleaving harmless — worst
    /// case is a transient overshoot the failed-send arm gives back.
    #[test]
    fn a_take_that_lands_between_the_send_and_the_count_does_not_latch_the_gate() {
        let depth = QueueDepth::default();

        // stacking: claim the slot, then hand over the message.
        depth.sent();
        // render:   recv() wakes, drain_to_latest finds nothing more, taken().
        depth.taken();

        assert_eq!(
            depth.pending(),
            0,
            "the channel is empty and the depth must say so; want_display gates on it"
        );
    }

    /// The same property driven through a real `sync_channel` with the exact send and
    /// receive sequences both tasks use.
    ///
    /// `yield_now` stands in for the preemption that opened the window in production; it
    /// makes the interleaving reachable without changing either side's logic. Against the
    /// old ordering this stalled after a handful of frames; it must now run to completion
    /// every time.
    #[test]
    fn the_display_gate_does_not_latch_shut() {
        use std::sync::mpsc::{sync_channel, TrySendError};

        let depth = QueueDepth::default();
        let (tx, rx) = sync_channel::<u64>(4);

        let rx_depth = depth.clone();
        let receiver = std::thread::spawn(move || {
            // `run_render_task`: recv, drain_to_latest, one `taken()` per message.
            while rx.recv().is_ok() {
                let mut skipped = 0;
                while rx.try_recv().is_ok() {
                    skipped += 1;
                }
                for _ in 0..=skipped {
                    rx_depth.taken();
                }
            }
        });

        // The stacking task only produces a frame when the camera hands it one, so the
        // sender waits for the gate rather than spinning past it. Without that pacing
        // the loop finishes before the receiver thread is ever scheduled and the test
        // measures nothing.
        const ITERATIONS: u64 = 200;
        let mut built = 0u64;
        for n in 0..ITERATIONS {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            while depth.pending() != 0 && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            if depth.pending() != 0 {
                break;
            }
            depth.sent();
            // Stands in for the preemption that opened the window in production.
            std::thread::yield_now();
            match tx.try_send(n) {
                Ok(()) => built += 1,
                Err(TrySendError::Full(_)) => depth.taken(),
                Err(TrySendError::Disconnected(_)) => {
                    depth.taken();
                    break;
                }
            }
        }
        drop(tx);
        receiver.join().unwrap();

        assert_eq!(
            built, ITERATIONS,
            "the gate latched shut after {built} of {ITERATIONS} frames"
        );
    }

    /// Both ends hold clones; they have to see one count.
    #[test]
    fn clones_share_one_count() {
        let sender = QueueDepth::default();
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
    /// Short enough that the latency bound never binds, so a test about the memory
    /// budget stays a test about the memory budget.
    const SHORT_EXPOSURE_US: u64 = 1_000;

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
        let caps = capacities_within(GIB, RAW_9MP, DISPLAY_9MP, SHORT_EXPOSURE_US);
        assert!(
            caps.storage > caps.render,
            "raw {} vs render {}: the raw channels are still sized off the display frame",
            caps.storage,
            caps.render
        );
        assert_eq!(caps.storage, capacity_within(GIB, RAW_9MP));
        assert_eq!(caps.stacking, caps.storage);
        assert_eq!(caps.render, capacity_within(GIB, DISPLAY_9MP));
    }

    /// Superpixel debayering makes the display frame a quarter of a full demosaic, so
    /// the render channel gets deeper — but the sensor still puts the same bytes on the
    /// wire and the capture channels must not move with it.
    #[test]
    fn a_smaller_display_frame_does_not_resize_the_raw_channels() {
        let full = capacities_within(GIB, RAW_9MP, DISPLAY_9MP, SHORT_EXPOSURE_US);
        let superpixel = capacities_within(GIB, RAW_9MP, DISPLAY_9MP / 4, SHORT_EXPOSURE_US);
        assert_eq!(full.storage, superpixel.storage);
        assert_eq!(full.stacking, superpixel.stacking);
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
        let caps = capacities_within(budget, RAW_9MP, DISPLAY_9MP, SHORT_EXPOSURE_US);

        // Both capture channels charged in full, though they share one `Arc` per frame,
        // and at the exposure that lets the stacking channel reach its memory ceiling.
        let queues = (caps.stacking + caps.storage) * RAW_9MP + caps.render * DISPLAY_9MP;
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

    /// The trade the capacity split does not make on its own.
    ///
    /// Sizing the capture channels from memory alone put 19 frames in front of the
    /// stacking thread on this host — 2.9 s of buffered sky at the 152 ms exposure the
    /// production traces were taken at. A deeper queue does not raise throughput; it
    /// delays the drop and pays for the delay in preview lag.
    #[test]
    fn the_stacking_channel_is_bounded_by_lag_as_well_as_by_memory() {
        let memory_bound = capacity_within(GIB, RAW_9MP);
        assert!(
            memory_bound > 2,
            "fixture must have memory headroom for the latency bound to bite"
        );

        // 152 ms — the exposure the traces this bound came from were taken at.
        let caps = capacities_within(GIB, RAW_9MP, DISPLAY_9MP, 152_000);
        assert!(
            caps.stacking < memory_bound,
            "{} frames of stacking queue at 152 ms is {} ms of lag",
            caps.stacking,
            caps.stacking * 152
        );
        assert!(
            caps.stacking as u64 * 152_000 <= MAX_STACKING_QUEUE_LATENCY_US,
            "{} frames at 152 ms exceeds the latency bound",
            caps.stacking
        );

        // Disk backlog is not lag: the storage channel keeps its memory-sized depth,
        // because a frame dropped there is sky that never reaches the FITS files.
        assert_eq!(caps.storage, memory_bound);
    }

    /// A long exposure must not be starved by the latency bound, and a short one must
    /// not be uncapped by it.
    #[test]
    fn the_latency_bound_never_takes_the_stacking_channel_below_two() {
        for exposure_us in [0, 1, 1_000, 152_000, 2_000_000, 300_000_000, u64::MAX] {
            let caps = capacities_within(GIB, RAW_9MP, DISPLAY_9MP, exposure_us);
            assert!(
                caps.stacking >= 2,
                "exposure {exposure_us} us gave {} slots",
                caps.stacking
            );
            assert!(
                caps.stacking <= capacity_within(GIB, RAW_9MP),
                "exposure {exposure_us} us exceeded the memory budget"
            );
        }
        // An unknown cadence falls back to the memory budget rather than collapsing.
        assert_eq!(
            capacities_within(GIB, RAW_9MP, DISPLAY_9MP, 0).stacking,
            capacity_within(GIB, RAW_9MP)
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
