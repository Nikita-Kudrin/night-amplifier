//! Background camera status monitor, on a dedicated OS thread (not tokio) so a
//! blocking FFI call like a USB stall in `camera.status()` can't poison a runtime
//! worker. Polls every `PHASE_POLL_INTERVAL`, broadcasts `CameraStatusUpdated`,
//! drives `Precooling -> Idle` after `STABILITY_SAMPLE_COUNT` stable samples, and
//! drives warmup: ramps to `WARMUP_RAMP_TARGET_C` at `RAMP_RATE_C_PER_MIN`, then at
//! `WARMUP_THRESHOLD_C` and ≤5% duty disables the cooler and closes the handle.
//! Rate-limited to `RAMP_RATE_C_PER_MIN` (5°C/min): the SDK call fires only when
//! the rounded setpoint changes (~one call per 12s); mid-ramp capture aborts it.
//!
//! The cooler and warm-up steps live in `cooler`, the bounded vendor calls in `ffi_worker`.

mod cooler;
mod ffi_worker;

use cooler::{
    cancel_warmup, finish_warmup, handle_update_cooler_target, push_setpoint, start_warmup,
    warmup_overran,
};
use ffi_worker::{with_camera_bounded, CallError, FfiWorker, FFI_CALL_TIMEOUT};

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use super::lifecycle::{self, DisconnectCause};
use super::{
    PHASE_POLL_INTERVAL, PRECOOL_TOLERANCE_C, STABILITY_SAMPLE_COUNT, WARMUP_THRESHOLD_C,
};

/// Polls in a row that find the slot empty, with nobody else entitled to the handle,
/// before it is declared lost. One is a race with a capture taking it while its hand-off
/// command is still queued; three, ~6 s, is not.
const MISSING_HANDLE_TICKS: u32 = 3;

use night_amplifier_core::camera::{CameraError, CameraStatus};
use crate::camera::health::{self as camera_health, FaultKind};
use super::ramp::RampState;
use crate::state::{AppState, CameraPhase, CameraRole, MonitorCmd};

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
    /// Polls in a row that found the slot empty while unpaused. See
    /// [`MISSING_HANDLE_TICKS`].
    missing_handle_ticks: u32,
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
        missing_handle_ticks: 0,
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
                ctx.missing_handle_ticks = 0;
                continue;
            }
            Ok(MonitorCmd::ResumeAfterCapture) => {
                ctx.paused_for_capture = false;
                ctx.settle_samples = 0;
                ctx.missing_handle_ticks = 0;
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

/// Run one polling iteration. Returns `false` when the monitor should stop
/// (handle closed during warmup).
fn tick(ctx: &mut MonitorCtx) -> bool {
    let warming_up = ctx.state.camera_phase(ctx.role) == CameraPhase::WarmingUp;
    // Before the status read, which a camera that stopped answering never gets past.
    if warming_up && warmup_overran(ctx) {
        warn!(camera_name = %ctx.camera_name, "Warmup timed out; forcing disconnect");
        finish_warmup(ctx);
        return false;
    }

    // `with_camera_bounded` has already told the shared detector what happened
    // — recording it again here would count one incident twice and reach the
    // give-up threshold in two faults instead of three.
    let status = match read_status(ctx) {
        Ok(s) => s,
        Err(CallError::NoHandle) => return note_missing_handle(ctx),
        Err(e) if e.is_device_lost() => {
            // A camera being disconnected needs no proof it is gone: there is nothing
            // left to warm, and the observer is waiting.
            if warming_up || ctx.fault_is_persistent {
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
    ctx.missing_handle_ticks = 0;

    // Broadcast the sample for the UI.
    let target = ctx.state.settings.snapshot().profile_for(ctx.role).target_temp_c;
    ctx.state.update_camera_status(&ctx.camera_name, status.clone(), target);

    let phase = ctx.state.camera_phase(ctx.role);

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
                        // A capture that took the handle since this tick read the phase
                        // owns the cooler now, and must not read as settled.
                        let settled = ctx.state.transition_camera_phase(
                            ctx.role,
                            &ctx.camera_name,
                            CameraPhase::Precooling,
                            CameraPhase::Idle,
                        );
                        if settled {
                            info!(
                                camera_name = %ctx.camera_name,
                                temp = status.temperature_c,
                                target,
                                "Precool complete"
                            );
                        }
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

            if ctx.warm_samples >= STABILITY_SAMPLE_COUNT {
                info!(
                    camera_name = %ctx.camera_name,
                    temp = status.temperature_c,
                    "Warmup complete"
                );
                finish_warmup(ctx);
                return false;
            }
        }
        CameraPhase::Idle
        | CameraPhase::Capturing
        | CameraPhase::Guiding
        | CameraPhase::Recovering
        | CameraPhase::Disconnected => {
            // Nothing to do — status was broadcast above.
            ctx.settle_samples = 0;
            ctx.warm_samples = 0;
        }
    }

    true
}

/// Count a poll that found the slot empty. The monitor is not paused, so nothing but the
/// monitor itself is entitled to the handle: past `MISSING_HANDLE_TICKS` it is lost, and
/// only a reopen gives the camera one again. On 2026-09-20 an Ares-C PRO sat four
/// minutes like this, refusing every Start, until the board was power-cycled.
fn note_missing_handle(ctx: &mut MonitorCtx) -> bool {
    ctx.missing_handle_ticks += 1;
    if ctx.missing_handle_ticks < MISSING_HANDLE_TICKS {
        return true;
    }
    error!(
        camera_name = %ctx.camera_name,
        role = ctx.role.label(),
        polls = ctx.missing_handle_ticks,
        "The camera handle is gone and nothing holds it; reopening the camera"
    );
    give_up_on_camera(ctx, FaultKind::HandleLost);
    false
}

fn read_status(ctx: &mut MonitorCtx) -> Result<CameraStatus, CallError> {
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
    let status = result?;
    // A reading no sensor can produce must not seed a ramp or settle a phase.
    if !status.has_plausible_temperature() {
        return Err(CallError::Camera(CameraError::TemperatureReadFailed(format!(
            "implausible sensor temperature {} °C",
            status.temperature_c
        ))));
    }
    Ok(status)
}

impl MonitorCtx {
    /// Report one camera fault to the shared detector and remember its verdict.
    fn record(&mut self, kind: FaultKind) {
        let consecutive = camera_health::record_fault(&self.state, self.role, &self.camera_name, kind);
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
    let phase = ctx.state.camera_phase(ctx.role);
    let cause = if phase == CameraPhase::WarmingUp {
        DisconnectCause::Requested
    } else {
        DisconnectCause::DeviceFault
    };

    warn!(camera_name = %name, role = ctx.role.label(), ?kind, ?phase, "Giving up on camera handle");

    let role = ctx.role;
    // No message to the user here: a fault that recovery fixes is not worth one, and one
    // it cannot fix is reported when it gives up.
    ctx.rt.block_on(async move {
        lifecycle::finalize_disconnect(&state, role, &name, cause).await;
    });
}
