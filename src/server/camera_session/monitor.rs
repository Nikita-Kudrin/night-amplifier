//! Background camera status monitor, on a dedicated OS thread (not a tokio task, so
//! a blocking FFI call like a USB stall inside `camera.status()` can't poison a
//! runtime worker). Polls `camera.status()` every `PHASE_POLL_INTERVAL` while the
//! handle is in the pool, broadcasts `CameraStatusUpdated`, drives
//! `Precooling -> Idle` once the sensor settles for `STABILITY_SAMPLE_COUNT`
//! samples, and drives warmup: on `StartWarmup`, ramps the setpoint to
//! `WARMUP_RAMP_TARGET_C` at `RAMP_RATE_C_PER_MIN`, then once the sensor hits
//! `WARMUP_THRESHOLD_C` at ≤5% duty, disables the cooler, closes the handle, and
//! broadcasts `CameraDisconnected`.
//!
//! Both ramps are rate-limited to `RAMP_RATE_C_PER_MIN` (5°C/min in production): the
//! commanded setpoint nudges toward its final value each tick, but the SDK call
//! only fires when the rounded integer changes — one call per ~12s at that rate.
//! Starting capture mid-ramp aborts it; the capture thread's per-frame
//! `apply_cooler_config` pushes the final target instead.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::lifecycle::{self, DisconnectCause};
use super::{
    PHASE_POLL_INTERVAL, PRECOOL_TOLERANCE_C, RAMP_RATE_C_PER_MIN, STABILITY_SAMPLE_COUNT,
    WARMUP_RAMP_TARGET_C, WARMUP_THRESHOLD_C, WARMUP_TIMEOUT,
};

/// Budget for a single camera call made from the monitor. Matches the capture
/// path's `STATUS_POLL_TIMEOUT`: both bound the same class of vendor call, and
/// a camera that needs longer than this to answer a status read is stalled.
const FFI_CALL_TIMEOUT: Duration = Duration::from_secs(3);
use crate::camera::CameraStatus;
use crate::server::camera_health::{self, FaultKind};
use super::ramp::RampState;
use crate::server::state::{AppState, CameraPhase, CameraRole, MonitorCmd};

/// Spawn the monitor thread. Returns a sender the caller (lifecycle) uses
/// to issue commands.
pub fn spawn(
    state: Arc<AppState>,
    role: CameraRole,
    camera_name: String,
    rt: tokio::runtime::Handle,
) -> mpsc::Sender<MonitorCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("camera-monitor-{}", camera_name))
        .spawn(move || run(state, role, camera_name, rt, rx))
        .expect("failed to spawn camera monitor thread");
    tx
}

struct MonitorCtx {
    state: Arc<AppState>,
    /// The slot this monitor polls. Bound at spawn, never looked up: the handle it
    /// checks out must be the one belonging to the camera it reports about, and with
    /// two slots live "whatever is connected" is no longer an answer.
    role: CameraRole,
    camera_name: String,
    rt: tokio::runtime::Handle,
    /// True while the capture thread owns the handle.
    paused_for_capture: bool,
    /// True while driving the warmup sequence.
    warming_up: bool,
    warmup_started_at: Option<Instant>,
    /// Consecutive samples within target tolerance (for precool → idle).
    settle_samples: u32,
    /// Consecutive samples at or above warmup threshold with low cooler power.
    warm_samples: u32,
    /// Active cooldown ramp, if any. Installed when `UpdateCoolerTarget` is
    /// received with `enabled = true` and a target.
    cooldown_ramp: Option<RampState>,
    /// Active warmup ramp, if any. Installed in `start_warmup`.
    warmup_ramp: Option<RampState>,
    /// One reusable thread for this monitor's camera calls.
    ffi: FfiWorker,
    /// Set by `with_camera_bounded` when the shared fault detector says this
    /// camera has failed often enough to stop retrying. Read by the callers so
    /// that one incident is counted once, no matter how many of them see it.
    fault_is_persistent: bool,
}

fn run(
    state: Arc<AppState>,
    role: CameraRole,
    camera_name: String,
    rt: tokio::runtime::Handle,
    rx: mpsc::Receiver<MonitorCmd>,
) {
    debug!(camera_name, role = role.label(), "Camera monitor thread started");

    let mut ctx = MonitorCtx {
        state,
        role,
        camera_name,
        rt,
        paused_for_capture: false,
        warming_up: false,
        warmup_started_at: None,
        settle_samples: 0,
        warm_samples: 0,
        cooldown_ramp: None,
        warmup_ramp: None,
        ffi: FfiWorker::new(),
        fault_is_persistent: false,
    };

    loop {
        // Wait for the next tick or an incoming command (whichever comes first).
        // `recv_timeout` handles both: a command pre-empts the tick; a timeout
        // means it's time to poll status.
        match rx.recv_timeout(PHASE_POLL_INTERVAL) {
            Ok(MonitorCmd::Shutdown) => {
                debug!(camera_name = %ctx.camera_name, "Monitor: Shutdown");
                break;
            }
            Ok(MonitorCmd::HandOffToCapture) => {
                ctx.paused_for_capture = true;
                continue;
            }
            Ok(MonitorCmd::ResumeAfterCapture) => {
                ctx.paused_for_capture = false;
                ctx.settle_samples = 0;
                continue;
            }
            Ok(MonitorCmd::StartWarmup { fast }) => {
                start_warmup(&mut ctx, fast);
                continue;
            }
            Ok(MonitorCmd::CancelWarmup) => {
                cancel_warmup(&mut ctx);
                continue;
            }
            Ok(MonitorCmd::UpdateCoolerTarget {
                enabled,
                target,
                fast,
            }) => {
                handle_update_cooler_target(&mut ctx, enabled, target, fast);
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Tick.
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                warn!(
                    camera_name = %ctx.camera_name,
                    "Monitor: command channel disconnected — exiting"
                );
                break;
            }
        }

        if ctx.paused_for_capture {
            continue;
        }

        if !tick(&mut ctx) {
            // tick() returned false → handle is gone (warmup finalized).
            break;
        }
    }

    debug!(camera_name = %ctx.camera_name, "Camera monitor thread exited");
}

fn handle_update_cooler_target(
    ctx: &mut MonitorCtx,
    enabled: bool,
    target: Option<f64>,
    fast: bool,
) {
    if !enabled {
        ctx.cooldown_ramp = None;
        ctx.settle_samples = 0;
        return;
    }
    let Some(final_target) = target else {
        ctx.cooldown_ramp = None;
        return;
    };

    if fast {
        // Fast mode: snap the hardware setpoint to the final target and
        // leave no ramp installed. The monitor's Precooling tick treats
        // "no cooldown_ramp" as "ramp already done" and will transition to
        // Idle once the sensor settles within tolerance.
        if !push_raw_setpoint(ctx, final_target) {
            return;
        }
        ctx.cooldown_ramp = None;
        ctx.settle_samples = 0;
        debug!(
            camera_name = %ctx.camera_name,
            final_target_c = final_target,
            "Installed fast-mode cooldown (no ramp)"
        );
        return;
    }

    // Seed the ramp start from the freshest sensor reading we can get. If
    // everything fails we fall back to the final target (ramp becomes a
    // no-op, which is the old behavior).
    let start = current_sensor_temp(ctx).unwrap_or(final_target);
    let ramp = RampState::new_from_current(start, final_target, Instant::now());
    debug!(
        camera_name = %ctx.camera_name,
        start_c = start,
        final_target_c = final_target,
        "Installed cooldown ramp"
    );
    ctx.cooldown_ramp = Some(ramp);
    ctx.settle_samples = 0;
}

fn start_warmup(ctx: &mut MonitorCtx, fast: bool) {
    if ctx.warming_up {
        return;
    }
    ctx.warming_up = true;
    ctx.warmup_started_at = Some(Instant::now());
    ctx.warm_samples = 0;
    // Cooldown ramp is no longer relevant while warming up.
    ctx.cooldown_ramp = None;

    if fast {
        // Fast mode: disable the TEC immediately and let the sensor rise
        // naturally. The WarmingUp tick branch still watches for the
        // warm-enough predicate before closing the handle.
        ctx.warmup_ramp = None;
        let result = with_camera_bounded(ctx, FFI_CALL_TIMEOUT, |cam| cam.set_cooler(false));
        if let Err(e) = result {
            warn!(error = %e, "Failed to disable cooler at fast-warmup start");
        }
        info!(camera_name = %ctx.camera_name, "Warmup started (fast — cooler disabled)");
        return;
    }

    // Seed the warmup ramp from the current sensor temperature so the first
    // commanded setpoint matches the PID's current operating point and we
    // avoid a jump up to ambient.
    let start = current_sensor_temp(ctx).unwrap_or(WARMUP_RAMP_TARGET_C);
    let ramp = RampState::new_from_current(start, WARMUP_RAMP_TARGET_C, Instant::now());

    // Push the initial integer setpoint so the TEC starts coasting up.
    // Keep the cooler ON — the user requirement is that duty falls naturally
    // as setpoint rises past ambient.
    if !push_setpoint(ctx, &ramp) {
        return;
    }
    ctx.warmup_ramp = Some(ramp);
    info!(
        camera_name = %ctx.camera_name,
        start_c = start,
        final_target_c = WARMUP_RAMP_TARGET_C,
        "Warmup started (ramped)"
    );
}

fn cancel_warmup(ctx: &mut MonitorCtx) {
    if !ctx.warming_up {
        return;
    }
    ctx.warming_up = false;
    ctx.warmup_started_at = None;
    ctx.warm_samples = 0;
    ctx.warmup_ramp = None;
    info!(camera_name = %ctx.camera_name, "Warmup cancelled");
}

/// Run one polling iteration. Returns `false` when the monitor should stop
/// (handle closed during warmup).
fn tick(ctx: &mut MonitorCtx) -> bool {
    // `with_camera_bounded` has already told the shared detector what happened
    // — recording it again here would count one incident twice and reach the
    // give-up threshold in two faults instead of three.
    let status = match read_status(ctx) {
        Ok(s) => s,
        Err(e) if e.is_sdk_disconnected() => {
            if ctx.fault_is_persistent {
                give_up_on_camera(ctx, FaultKind::DeviceLost);
                return false;
            }
            return true; // Not yet conclusive; poll again.
        }
        Err(e) => {
            debug!(camera_name = %ctx.camera_name, error = %e, "Transient error reading camera status");
            return true; // transient error; keep running
        }
    };

    // Broadcast the sample for the UI.
    let target = ctx
        .rt
        .block_on(ctx.state.settings.read())
        .profile_for(ctx.role)
        .target_temp_c;
    ctx.rt.block_on(
        ctx.state
            .update_camera_status(&ctx.camera_name, status.clone(), target),
    );

    let phase = ctx.rt.block_on(ctx.state.camera_phase(&ctx.camera_name));

    match phase {
        CameraPhase::Precooling => {
            // Advance the ramp (if any) and push new setpoint when it crosses
            // an integer boundary.
            if let Some(ramp) = ctx.cooldown_ramp.as_mut() {
                ramp.step(Instant::now());
                let commanded = ramp.commanded_i64();
                if ramp.last_commanded_i64 != Some(commanded) {
                    let snapshot = ramp.clone();
                    if !push_setpoint(ctx, &snapshot) {
                        return false;
                    }
                    if let Some(ramp) = ctx.cooldown_ramp.as_mut() {
                        ramp.last_commanded_i64 = Some(commanded);
                    }
                }
            }

            // Settle to Idle: the commanded setpoint must have reached the
            // user's target AND the sensor must be within tolerance for
            // STABILITY_SAMPLE_COUNT consecutive samples.
            let ramp_done = ctx
                .cooldown_ramp
                .as_ref()
                .map(|r| r.is_at_final_target())
                .unwrap_or(true);
            if let Some(target) = target {
                let within = (status.temperature_c - target).abs() <= PRECOOL_TOLERANCE_C;
                if ramp_done && within {
                    ctx.settle_samples = ctx.settle_samples.saturating_add(1);
                    if ctx.settle_samples >= STABILITY_SAMPLE_COUNT {
                        ctx.cooldown_ramp = None;
                        ctx.rt.block_on(
                            ctx.state
                                .set_camera_phase(&ctx.camera_name, CameraPhase::Idle),
                        );
                        info!(
                            camera_name = %ctx.camera_name,
                            temp = status.temperature_c,
                            target,
                            "Precool complete"
                        );
                    }
                } else {
                    ctx.settle_samples = 0;
                }
            }
        }
        CameraPhase::WarmingUp => {
            if !ctx.warming_up {
                // External (lifecycle) set phase to WarmingUp without sending
                // StartWarmup. Default to the safe ramped path — the normal
                // disconnect flow will have already sent StartWarmup with the
                // user's actual fast-mode preference.
                start_warmup(ctx, false);
            }

            // Advance the warmup ramp and push new setpoint when the rounded
            // integer commanded value changes.
            if let Some(ramp) = ctx.warmup_ramp.as_mut() {
                ramp.step(Instant::now());
                let commanded = ramp.commanded_i64();
                if ramp.last_commanded_i64 != Some(commanded) {
                    let snapshot = ramp.clone();
                    if !push_setpoint(ctx, &snapshot) {
                        return false;
                    }
                    if let Some(ramp) = ctx.warmup_ramp.as_mut() {
                        ramp.last_commanded_i64 = Some(commanded);
                    }
                }
            }

            let warm_enough = status.temperature_c >= WARMUP_THRESHOLD_C
                && status.cooler_power.unwrap_or(0.0) <= 5.0;
            if warm_enough {
                ctx.warm_samples = ctx.warm_samples.saturating_add(1);
            } else {
                ctx.warm_samples = 0;
            }

            let timed_out = ctx
                .warmup_started_at
                .map(|t| t.elapsed() >= WARMUP_TIMEOUT)
                .unwrap_or(false);

            if ctx.warm_samples >= STABILITY_SAMPLE_COUNT || timed_out {
                if timed_out {
                    warn!(
                        camera_name = %ctx.camera_name,
                        temp = status.temperature_c,
                        "Warmup timed out; forcing disconnect"
                    );
                } else {
                    info!(
                        camera_name = %ctx.camera_name,
                        temp = status.temperature_c,
                        "Warmup complete"
                    );
                }

                // Disable the cooler here (moved from start_warmup). By this
                // point the setpoint is at or past ambient so duty is already
                // near 0 % — this just latches it off before we close.
                let result =
                    with_camera_bounded(ctx, FFI_CALL_TIMEOUT, |cam| cam.set_cooler(false));
                if let Err(e) = result {
                    warn!(error = %e, "Failed to disable cooler at warmup finalize");
                }
                ctx.warmup_ramp = None;

                // Finalize disconnect from the monitor thread. `finalize_disconnect`
                // will clear this slot's monitor sender (ours) and close the handle.
                let state = Arc::clone(&ctx.state);
                let name = ctx.camera_name.clone();
                let role = ctx.role;
                ctx.rt.block_on(async move {
                    lifecycle::finalize_disconnect(&state, role, &name, DisconnectCause::Requested)
                        .await;
                });
                return false;
            }
        }
        CameraPhase::Idle
        | CameraPhase::Capturing
        | CameraPhase::Guiding
        | CameraPhase::Disconnected => {
            // Nothing to do — status was broadcast above.
            ctx.settle_samples = 0;
            ctx.warm_samples = 0;
        }
    }

    true
}

fn read_status(ctx: &mut MonitorCtx) -> Result<CameraStatus, crate::camera::CameraError> {
    let start = Instant::now();
    let result = with_camera_bounded(ctx, FFI_CALL_TIMEOUT, |cam| cam.status());
    let elapsed = start.elapsed();
    if elapsed > Duration::from_millis(500) {
        warn!(
            camera_name = %ctx.camera_name,
            elapsed_ms = elapsed.as_millis(),
            "camera.status() was slow"
        );
    }
    result
}

/// Read the current sensor temperature for ramp seeding. Prefers a fresh
/// hardware sample, falling back to the last cached status if the handle is
/// unavailable (e.g. momentarily held by another path).
fn current_sensor_temp(ctx: &mut MonitorCtx) -> Option<f64> {
    if let Ok(status) = read_status(ctx) {
        return Some(status.temperature_c);
    }
    ctx.rt
        .block_on(ctx.state.get_camera_status(&ctx.camera_name))
        .map(|s| s.temperature_c)
}

/// Push the ramp's current integer setpoint to the camera. Best-effort: a
/// failed SDK call is logged but does not abort the ramp — the next tick will
/// retry.
fn push_setpoint(ctx: &mut MonitorCtx, ramp: &RampState) -> bool {
    push_raw_setpoint(ctx, ramp.current_setpoint_c)
}

/// Push an arbitrary setpoint (°C) to the camera, bypassing any ramp. Used
/// by the fast-mode path and by `push_setpoint` above.
fn push_raw_setpoint(ctx: &mut MonitorCtx, temp_c: f64) -> bool {
    let result = with_camera_bounded(ctx, FFI_CALL_TIMEOUT, move |cam| {
        cam.set_target_temperature(temp_c)
    });

    let Err(e) = result else {
        return true;
    };

    warn!(
        camera_name = %ctx.camera_name,
        setpoint = temp_c,
        error = %e,
        "Failed to push setpoint"
    );

    if !e.is_sdk_disconnected() || !ctx.fault_is_persistent {
        return true; // Transient, or not yet conclusive; the next tick retries.
    }
    give_up_on_camera(ctx, FaultKind::DeviceLost);
    false
}

impl MonitorCtx {
    /// Report one camera fault to the shared detector and remember its verdict.
    fn record(&mut self, kind: FaultKind) {
        let consecutive = camera_health::record_fault(&self.state, &self.camera_name, kind);
        self.fault_is_persistent = camera_health::is_persistent(consecutive);
    }
}

/// Tear the session down after the fault detector has concluded the camera is
/// not coming back on this handle.
///
/// Whether this counts as an unexpected loss — and so whether the reconnect
/// supervisor should try to recover it — depends on what the session was doing.
/// A camera that stops answering while warming up is on its way out anyway;
/// reconnecting it would fight the user's own disconnect.
fn give_up_on_camera(ctx: &mut MonitorCtx, kind: FaultKind) {
    let state = Arc::clone(&ctx.state);
    let name = ctx.camera_name.clone();
    let phase = ctx.rt.block_on(ctx.state.camera_phase(&ctx.camera_name));
    let cause = if phase == CameraPhase::WarmingUp {
        DisconnectCause::Requested
    } else {
        DisconnectCause::DeviceFault
    };

    warn!(camera_name = %name, role = ctx.role.label(), ?kind, ?phase, "Giving up on camera handle");

    let role = ctx.role;
    ctx.rt.block_on(async move {
        state.send_error(camera_health::incident_message(&name, kind));
        lifecycle::finalize_disconnect(&state, role, &name, cause).await;
    });
}

/// A reusable thread for the monitor's camera FFI calls. Must not block
/// indefinitely inside a vendor call — the phase machine would stop, a warmup could
/// never finalize, and `take_for_capture` couldn't get the handle — but it polls
/// every `PHASE_POLL_INTERVAL`, so a thread per call meant ~1,800 spawns/hour while
/// connected (real cost on a Pi 5 for a near-instant call). One thread serves every
/// call instead: a call overrunning its budget is abandoned with its thread (no way
/// to cancel a stuck synchronous FFI call), and the next call spawns a replacement —
/// one thread for the monitor's lifetime, plus one per actual stall.
struct FfiWorker {
    /// `None` until the first call, and again after a stall abandons a worker.
    jobs: Option<mpsc::Sender<Job>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl FfiWorker {
    fn new() -> Self {
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
fn with_camera_bounded<T, F>(
    ctx: &mut MonitorCtx,
    timeout: Duration,
    f: F,
) -> Result<T, crate::camera::CameraError>
where
    F: FnOnce(&mut Box<dyn crate::camera::Camera>) -> Result<T, crate::camera::CameraError>
        + Send
        + 'static,
    T: Send + 'static,
{
    let slot = ctx.state.slot(ctx.role);
    let camera_opt = {
        let mut guard = slot.handle.lock().expect("camera handle mutex poisoned");
        guard.take()
    };
    let camera = camera_opt.ok_or(crate::camera::CameraError::Disconnected)?;

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
        ctx.record(FaultKind::Timeout);
        ctx.state.slot(ctx.role).notify_handle_returned();
        return Err(crate::camera::CameraError::Disconnected);
    };

    let phase = ctx.rt.block_on(ctx.state.camera_phase(&ctx.camera_name));
    if phase == CameraPhase::Disconnected {
        let _ = camera.close();
    } else {
        let mut guard = ctx
            .state
            .slot(ctx.role)
            .handle
            .lock()
            .expect("camera handle mutex poisoned");
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
    ctx.state.slot(ctx.role).notify_handle_returned();

    match &result {
        Err(e) if e.is_sdk_disconnected() => ctx.record(FaultKind::DeviceLost),
        _ => {
            camera_health::clear_fault_streak(&ctx.state, &ctx.camera_name);
            ctx.fault_is_persistent = false;
        }
    }

    result
}

#[cfg(test)]
mod ramp_tests {
    use super::*;
    use std::time::Duration;

}
