//! Decoupled asynchronous capture pipeline: four independent dedicated-thread tasks
//! connected by bounded MPSC channels — **CaptureTask** (acquires frames),
//! **StorageTask** (saves raw to disk), **StackingTask** (registration +
//! accumulation), **RenderTask** (preview render + encode). `Arc<Frame>` gives
//! zero-copy sharing between channels; capacities derive from a memory budget over
//! actual frame size. Each thread carries a `tokio::runtime::Handle` from the async
//! orchestrator, for `handle.block_on()`/`handle.spawn()`.
//!
//! The guide camera bypasses all of that: **GuideTask** is one thread with no channels,
//! because nothing it produces is stacked or queued. See `guide_task`.

pub mod analysis;
pub mod channel;
mod context;
mod drop_log;
mod frame_gate;
pub mod guide_task;
pub mod pipeline;
mod render_task;
pub mod solving;
mod stacking_task;
mod stage_config;
pub mod storage;

pub mod config_overrides;
pub mod task;
pub mod watchdog;

#[cfg(test)]
pub mod watchdog_tests;

pub use analysis::{AnalysisContext, PreviewAnalysis};
pub use drop_log::DropLog;
pub use context::{PlanetaryStackingContext, StackingCarryover, StackingContext};
pub use frame_gate::{FrameAdmission, FrameGate, RejectionReason};
pub use task::run_capture_loop;
