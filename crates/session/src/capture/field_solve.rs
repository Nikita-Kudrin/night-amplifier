//! One plate solve per stack, so the background model keeps catalog objects out (Pro).
//!
//! Shape alone cannot tell a faint nebula from a glow entering the same edge; the catalog
//! can. Once the stack is [`FIRST_DEPTH`] deep, its snapshot is solved in the background
//! through the [`FieldObjectsPlugin`], bounded by [`BUDGET`], and the background model
//! renders without the footprints until they land — nothing waits on the solve, and a
//! field too zoomed-in to solve simply never gets them. One job per stack: a new stack
//! (new session, Wanderer reset, a resize) starts a new one.
//!
//! Push-To comes first: the job does not start while a Push-To solve runs, and a Push-To
//! solve starting cancels it (two ASTAPs at once would halve both on a Pi). A cancel —
//! that, or a Stop — is not a verdict on the stack: the job waits for the next sub and
//! tries again, so a resumed capture, which keeps its stack, still gets protected.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use night_amplifier_core::background::{FieldProtection, ProtectedRegion};
use night_amplifier_core::detection::luminance_frame;
use night_amplifier_core::frame::Frame;
use night_amplifier_core::push_to::PushToError;
use tracing::{debug, info, warn};

use crate::state::{AppState, CameraRole};

/// Stack depth before the first attempt: more stars than a sub, for zoomed-in fields.
pub const FIRST_DEPTH: usize = 6;

/// A frame-quality failure (too few stars) retries this many subs deeper.
pub const RETRY_STEP: usize = 10;

/// Attempts per stack before giving up for it.
pub const MAX_ATTEMPTS: u32 = 3;

/// The whole solve, every rung included. Hinted rungs answer in seconds; a stated-FOV
/// full-sky rung is bounded at 60 s; the blind ladder (50-120 s on a desktop for these
/// fields, minutes on a Pi) is never asked.
pub const BUDGET: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the stack to reach `next_depth`.
    Waiting { next_depth: usize, attempts: u32 },
    Running { attempts: u32 },
    Done,
    GaveUp,
}

#[derive(Debug)]
struct Inner {
    /// Which stack the job belongs to; a result for an older one is dropped.
    epoch: u64,
    phase: Phase,
    protection: Option<Arc<FieldProtection>>,
}

/// The session's field solve: its phase and, once landed, the protection it produced.
#[derive(Debug)]
pub struct FieldSolve(Mutex<Inner>);

/// A claimed attempt; [`FieldSolve::finish`] it with the same ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    epoch: u64,
}

impl Default for FieldSolve {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            epoch: 0,
            phase: Phase::Waiting { next_depth: FIRST_DEPTH, attempts: 0 },
            protection: None,
        }))
    }
}

impl FieldSolve {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new stack: forget the last one's footprints and solve again.
    pub fn reset(&self) {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.phase = Phase::Waiting { next_depth: FIRST_DEPTH, attempts: 0 };
        inner.protection = None;
    }

    /// The protected footprints of the current stack, in its own (uncropped) pixels.
    pub fn protection(&self) -> Option<Arc<FieldProtection>> {
        self.lock().protection.clone()
    }

    /// Claim an attempt if the stack at `depth` is due one.
    pub fn claim(&self, depth: usize) -> Option<Ticket> {
        let mut inner = self.lock();
        let Phase::Waiting { next_depth, attempts } = inner.phase else {
            return None;
        };
        if depth < next_depth {
            return None;
        }
        inner.phase = Phase::Running { attempts: attempts + 1 };
        Some(Ticket { epoch: inner.epoch })
    }

    /// Hand a claim back unused (no plugin, or the stack could not be projected): the
    /// next frame tries again.
    pub fn release(&self, ticket: Ticket, depth: usize) {
        let mut inner = self.lock();
        if let (true, Phase::Running { attempts }) = (inner.epoch == ticket.epoch, inner.phase) {
            inner.phase = Phase::Waiting { next_depth: depth, attempts: attempts - 1 };
        }
    }

    /// Record an attempt's outcome for a `width` x `height` stack at `depth`. A result
    /// for a stack that has since been replaced is dropped.
    pub fn finish(
        &self,
        ticket: Ticket,
        outcome: Result<Vec<ProtectedRegion>, PushToError>,
        (width, height): (usize, usize),
        depth: usize,
    ) {
        let mut inner = self.lock();
        let Phase::Running { attempts } = inner.phase else {
            return;
        };
        if inner.epoch != ticket.epoch {
            return;
        }
        inner.phase = match outcome {
            Ok(regions) => {
                info!(objects = regions.len(), "Stack plate solved; catalog objects protected from the background model");
                inner.protection = Some(Arc::new(FieldProtection {
                    generation: inner.epoch,
                    width,
                    height,
                    regions,
                }));
                Phase::Done
            }
            Err(PushToError::Cancelled) => {
                debug!(depth, "Stack plate solve cancelled; retrying on the next sub");
                Phase::Waiting { next_depth: depth + 1, attempts: attempts - 1 }
            }
            Err(e) if e.is_frame_quality() && attempts < MAX_ATTEMPTS => {
                debug!(error = %e, depth, "Stack not solvable yet; retrying deeper");
                Phase::Waiting { next_depth: depth + RETRY_STEP, attempts }
            }
            Err(e) => {
                info!(error = %e, "Stack plate solve gave up; background removal runs without catalog protection");
                Phase::GaveUp
            }
        };
    }
}

/// Offer the stack at `depth` to the field solve if it is due one: Pro, background
/// subtraction on and no Push-To solve running. Runs on its own thread off the stacking
/// task; returns at once.
pub fn offer(state: &Arc<AppState>, rt: &tokio::runtime::Handle, stack: &Arc<Frame>, depth: usize) {
    if state.plugins.field_objects().is_none() || state.push_to.as_ref().is_some_and(|pt| pt.is_solving()) {
        return;
    }
    let Some(ticket) = state.field_solve.claim(depth) else {
        return;
    };
    let (job_state, rt, stack) = (Arc::clone(state), rt.clone(), Arc::clone(stack));
    let spawned = std::thread::Builder::new().name("field-solve".into()).spawn(move || {
        let state = job_state;
        let (width, height) = (stack.width(), stack.height());
        // One plane is all the solver reads; the colour stack (108 MB at 3008² x3) is let
        // go here rather than held for the whole solve.
        let stack = match mono(stack) {
            Ok(mono) => mono,
            Err(e) => {
                warn!(error = %e, "Could not project the stack for its plate solve");
                state.field_solve.release(ticket, depth);
                return;
            }
        };
        let Some(plugin) = state.plugins.field_objects() else {
            state.field_solve.release(ticket, depth);
            return;
        };
        let camera = state.camera_in_role(CameraRole::Main).map(|c| c.info.name);
        let telescope = state.settings.snapshot().solver_telescope(camera.as_deref());
        let detector = super::solving::solve_detector();
        let outcome = rt.block_on(plugin.objects_in_field(&stack, camera.as_deref(), &telescope, detector, BUDGET));
        state.field_solve.finish(ticket, outcome, (width, height), depth);
    });
    if let Err(e) = spawned {
        warn!(error = %e, "Could not start the stack plate solve");
        state.field_solve.release(ticket, depth);
    }
}

/// Abandon the stack's plate solve in flight, as the capture stops. It retries once the
/// stack grows again (a resumed capture keeps it).
pub fn cancel(state: &AppState) {
    if state.plugins.field_objects().is_some_and(|plugin| plugin.cancel()) {
        info!("Capture ended; abandoned the stack plate solve in flight");
    }
}

/// `stack` as one plane, the projection the solver and its star detection use.
fn mono(stack: Arc<Frame>) -> night_amplifier_core::error::Result<Arc<Frame>> {
    if stack.channels() == 1 {
        return Ok(stack);
    }
    luminance_frame(&stack).map(Arc::new)
}

#[cfg(test)]
#[path = "field_solve_tests.rs"]
mod tests;
