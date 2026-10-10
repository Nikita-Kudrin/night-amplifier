//! Camera calls from the monitor, bounded: a vendor call that hangs is abandoned with its
//! thread rather than stopping the phase machine.

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tracing::{error, warn};

use super::MonitorCtx;
use crate::camera::health::{self as camera_health, FaultKind};
use crate::state::CameraPhase;
use night_amplifier_core::camera::CameraError;

/// Budget for a single camera call made from the monitor. Matches the capture
/// path's `STATUS_POLL_TIMEOUT`: both bound the same class of vendor call, and
/// a camera that needs longer than this to answer a status read is stalled.
pub(super) const FFI_CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// Why a monitor call produced no result.
#[derive(Debug)]
pub(super) enum CallError {
    /// The slot was empty: somebody else has the handle, or nobody does.
    NoHandle,
    /// The call ran and failed, or overran its budget and was abandoned.
    Camera(CameraError),
}

impl CallError {
    /// The device said it is gone, or stopped answering and its handle was abandoned.
    pub(super) fn is_device_lost(&self) -> bool {
        matches!(self, CallError::Camera(e) if e.is_sdk_disconnected())
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NoHandle => f.write_str("no camera handle in the slot"),
            CallError::Camera(e) => e.fmt(f),
        }
    }
}

/// A reusable thread for the monitor's camera FFI calls. Must not block
/// indefinitely inside a vendor call — the phase machine would stop, a warmup could
/// never finalize, and `take_for_capture` couldn't get the handle — but it polls
/// every `PHASE_POLL_INTERVAL`, so a thread per call meant ~1,800 spawns/hour while
/// connected (real cost on a Pi 5 for a near-instant call). One thread serves every
/// call instead: a call overrunning its budget is abandoned with its thread (no way
/// to cancel a stuck synchronous FFI call), and the next call spawns a replacement —
/// one thread for the monitor's lifetime, plus one per actual stall.
pub(super) struct FfiWorker {
    /// `None` until the first call, and again after a stall abandons a worker.
    jobs: Option<mpsc::Sender<Job>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl FfiWorker {
    pub(super) fn new() -> Self {
        Self { jobs: None }
    }

    /// Run `f` on the worker thread, waiting at most `timeout`.
    ///
    /// `None` means it did not return in time. Whatever `f` owns — including a
    /// camera handle — stays with the abandoned thread and is dropped there
    /// when the SDK finally returns; `DeviceLease` is what keeps that late drop
    /// from closing a device a reconnect has since opened.
    fn run<T: Send + 'static>(
        &mut self,
        timeout: Duration,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (done_tx, done_rx) = mpsc::channel();
        let job: Job = Box::new(move || {
            let _ = done_tx.send(f());
        });

        if !self.dispatch(job) {
            return None;
        }

        match done_rx.recv_timeout(timeout) {
            Ok(value) => Some(value),
            Err(_) => {
                // The worker is still inside the SDK. Drop our end of its job
                // channel so it exits once it unwinds, and start fresh.
                self.jobs = None;
                None
            }
        }
    }

    /// Send `job` to the worker, spawning or replacing it as needed.
    fn dispatch(&mut self, job: Job) -> bool {
        if self.jobs.is_none() {
            self.jobs = Self::spawn();
        }
        let Some(tx) = self.jobs.as_ref() else {
            return false;
        };
        let Err(returned) = tx.send(job) else {
            return true;
        };

        // The worker exited between calls. One retry with a fresh thread.
        self.jobs = Self::spawn();
        let Some(tx) = self.jobs.as_ref() else {
            return false;
        };
        tx.send(returned.0).is_ok()
    }

    fn spawn() -> Option<mpsc::Sender<Job>> {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("camera-monitor-ffi".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            });
        match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                error!(error = %e, "Failed to spawn the camera monitor FFI worker");
                None
            }
        }
    }
}

/// Run one camera operation with the handle checked out of this monitor's slot.
///
/// The handle has to leave the mutex for the call's duration: a `Box<dyn
/// Camera>` can only be used by one caller at a time, and holding the
/// `std::sync::Mutex` across a vendor call that might hang would block async
/// readers on a runtime worker. While it is out, the slot's handle reads
/// as `None` — every other reader waits on `handle_returned` rather than
/// treating that as "no camera" (see `lifecycle::with_camera`).
pub(super) fn with_camera_bounded<T, F>(
    ctx: &mut MonitorCtx,
    timeout: Duration,
    f: F,
) -> Result<T, CallError>
where
    F: FnOnce(&mut Box<dyn night_amplifier_core::camera::Camera>) -> Result<T, night_amplifier_core::camera::CameraError>
        + Send
        + 'static,
    T: Send + 'static,
{
    let state = Arc::clone(&ctx.state);
    let slot = state.slot(ctx.role);
    // Marked before the handle leaves the slot and cleared after it is back, so at no
    // instant does a reader see neither — which is what "lost" means to them.
    slot.set_monitor_call(Some(Instant::now()));
    let camera_opt = {
        let mut guard = slot.handle.lock().expect("camera handle mutex poisoned");
        guard.take()
    };
    let Some(camera) = camera_opt else {
        slot.set_monitor_call(None);
        return Err(CallError::NoHandle);
    };

    let outcome = ctx.ffi.run(timeout, move || {
        let mut camera = camera;
        let result = f(&mut camera);
        (camera, result)
    });

    let Some((mut camera, result)) = outcome else {
        error!(
            camera_name = %ctx.camera_name,
            ?timeout,
            "Camera call did not return in time — abandoning handle (suspected USB stall)"
        );
        slot.set_monitor_call(None);
        ctx.record(FaultKind::Timeout);
        slot.notify_handle_returned();
        return Err(CallError::Camera(CameraError::Disconnected));
    };

    let phase = ctx.state.camera_phase(ctx.role);
    // A slot suspended while ours was out has no camera to give it back to, and the
    // reopen expects it empty.
    let recovering = slot.is_recovering();
    if phase == CameraPhase::Disconnected || recovering {
        warn!(
            camera_name = %ctx.camera_name,
            ?phase,
            recovering,
            "Closing the handle a status call brought back: its slot was torn down or suspended meanwhile"
        );
        let _ = camera.close();
    } else {
        let mut guard = slot.handle.lock().expect("camera handle mutex poisoned");
        match guard.as_ref() {
            // A reconnect installed a new handle while ours was out. Ours is
            // the stale one; `DeviceLease` makes closing it a no-op against the
            // live device.
            Some(_) => {
                warn!(camera_name = %ctx.camera_name, "Camera replaced during poll; dropping the superseded handle");
                let _ = camera.close();
            }
            None => *guard = Some(camera),
        }
    }
    // Cleared once the handle is back, never before: a failed hand-off reading "no call,
    // no handle" in between would declare a healthy handle lost.
    slot.set_monitor_call(None);
    slot.notify_handle_returned();

    match &result {
        Err(e) if e.is_sdk_disconnected() => ctx.record(FaultKind::DeviceLost),
        _ => {
            camera_health::clear_fault_streak(&ctx.state, ctx.role, &ctx.camera_name);
            ctx.fault_is_persistent = false;
        }
    }

    result.map_err(CallError::Camera)
}
