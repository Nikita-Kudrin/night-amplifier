//! Decoupled asynchronous capture pipeline: four dedicated-thread tasks connected by
//! bounded MPSC channels — **CaptureTask** (acquires), **StorageTask** (saves raw),
//! **StackingTask** (registration + accumulation), **RenderTask** (preview + encode).
//! `Arc<Frame>` gives zero-copy sharing; capacities derive from a memory budget over
//! actual frame size. Each thread holds a `tokio::runtime::Handle` for
//! `block_on()`/`spawn()`. **Push-To** is two more thread tasks over one-slot channels
//! that drop rather than queue (`push_to_tasks`). The guide camera bypasses all this:
//! **GuideTask** is one thread, no channels — nothing it produces is stacked or queued.

pub mod analysis;
pub mod channel;
mod context;
mod drop_log;
mod frame_gate;
pub mod guide_task;
pub mod pipeline;
pub(crate) mod push_to_tasks;
mod render_task;
#[cfg(test)]
pub(crate) use render_task::run_render_task;
pub mod solving;
mod stacking_task;
mod stage_config;
pub(crate) mod stall;
pub mod storage;
mod stream_encoding;

pub mod config_overrides;
pub mod task;
pub mod watchdog;

#[cfg(test)]
pub mod watchdog_tests;

pub use analysis::{AnalysisContext, PreviewAnalysis};
pub use context::{
    LiveStacker, PlanetaryStackingContext, StackSettings, StackingCarryover, StackingContext,
};
pub use drop_log::DropLog;
pub use frame_gate::{FrameAdmission, FrameGate, RejectionReason};
pub use task::run_capture_loop;
