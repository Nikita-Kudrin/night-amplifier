//! Decoupled asynchronous capture pipeline
//!
//! The capture loop is decomposed into four independent tasks connected by
//! bounded MPSC channels:
//!
//! - **CaptureTask** (dedicated thread) — acquires frames from the camera
//! - **StorageTask** (dedicated thread) — saves raw frames to disk
//! - **StackingTask** (dedicated thread) — registration + accumulation
//! - **RenderTask** (dedicated thread) — preview rendering + encoding
//!
//! `Arc<Frame>` provides zero-copy frame sharing between channels.
//! Channel capacities are calculated from a 2 GB memory budget divided by
//! the actual frame size.
//!
//! Each spawned OS thread receives a `tokio::runtime::Handle` captured from
//! the async orchestrator, so it can call `handle.block_on()` for async
//! state access and `handle.spawn()` for fire-and-forget async work.

pub mod channel;
mod context;
pub mod pipeline;
mod render_task;
mod solving;
mod stacking_task;
mod storage;

pub mod config_overrides;
pub mod task;
pub mod watchdog;

#[cfg(test)]
pub mod watchdog_tests;

pub use context::{PlanetaryStackingContext, StackingContext};
pub use task::run_capture_loop;
