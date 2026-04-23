//! Background camera status monitor — runs on a dedicated OS thread.
//!
//! Responsibilities:
//! - Poll `camera.status()` every `PHASE_POLL_INTERVAL` while the handle
//!   is in the pool (not taken by the capture thread).
//! - Broadcast `CameraStatusUpdated` so the UI has a live temperature feed.
//! - Drive `Precooling → Idle` transition when the sensor settles near
//!   the target for `STABILITY_SAMPLE_COUNT` consecutive samples.
//! - Drive the warmup sequence: on `StartWarmup`, keep the cooler ON and
//!   ramp the setpoint up to `WARMUP_RAMP_TARGET_C` at `RAMP_RATE_C_PER_MIN`.
//!   Once the sensor reaches `WARMUP_THRESHOLD_C` and duty is ≤ 5 %, disable
//!   the cooler, close the handle, and broadcast `CameraDisconnected`.
//!
//! Both cooldown and warmup are rate-limited to `RAMP_RATE_C_PER_MIN`
//! (5 °C/min in production). The commanded setpoint is nudged toward its
//! final value every tick; the SDK call is only issued when the rounded
//! integer value changes (PlayerOne takes `i64`), which at 5 °C/min means
//! one SDK call per ~12 s. Starting capture mid-ramp aborts the ramp: the
//! capture thread's per-frame `apply_cooler_config` pushes the final target.
//!
//! Uses an OS thread (not a tokio task) so a blocking FFI call — e.g., a
//! USB stall inside `camera.status()` — cannot poison a runtime worker.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use super::{
    lifecycle, PHASE_POLL_INTERVAL, PRECOOL_TOLERANCE_C, RAMP_RATE_C_PER_MIN,
    STABILITY_SAMPLE_COUNT, WARMUP_RAMP_TARGET_C, WARMUP_THRESHOLD_C, WARMUP_TIMEOUT,
};
use crate::camera::CameraStatus;
use crate::server::state::{AppState, CameraPhase, MonitorCmd};

/// Direction of a ramp — determines the sign of the per-step delta and the
/// `is_at_final_target` clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RampDirection {
    /// Cooldown: setpoint decreases toward `final_target_c`.
    Cooling,
    /// Warmup: setpoint increases toward `final_target_c`.
    Warming,
}

/// Rate-limited TEC setpoint ramp. Used for both cooldown and warmup.
///
/// The logical setpoint is stored as `f64` and advanced by wall-clock delta
/// each tick. SDK calls (which take integer °C on some providers) are gated
/// on the rounded value changing — avoiding the case where sub-degree per-tick
/// steps would otherwise truncate to 0 and stall the ramp.
#[derive(Debug, Clone)]
pub(super) struct RampState {
    final_target_c: f64,
    current_setpoint_c: f64,
    last_commanded_i64: Option<i64>,
    last_tick_at: Instant,
    direction: RampDirection,
}

impl RampState {
    fn new_from_current(start_c: f64, final_target_c: f64, now: Instant) -> Self {
        let direction = if final_target_c < start_c {
            RampDirection::Cooling
        } else {
            RampDirection::Warming
        };
        Self {
            final_target_c,
            current_setpoint_c: start_c,
            last_commanded_i64: None,
            last_tick_at: now,
            direction,
        }
    }

    /// Advance the commanded setpoint by `dt_sec * RAMP_RATE_C_PER_MIN / 60`,
    /// clamped to not overshoot `final_target_c`. Returns the new setpoint.
    fn step(&mut self, now: Instant) -> f64 {
        let dt_sec = now
            .saturating_duration_since(self.last_tick_at)
            .as_secs_f64();
        self.last_tick_at = now;
        if dt_sec <= 0.0 {
            return self.current_setpoint_c;
        }
        let step_c = dt_sec * RAMP_RATE_C_PER_MIN / 60.0;
        self.current_setpoint_c = match self.direction {
            RampDirection::Cooling => (self.current_setpoint_c - step_c).max(self.final_target_c),
            RampDirection::Warming => (self.current_setpoint_c + step_c).min(self.final_target_c),
        };
        self.current_setpoint_c
    }

    fn is_at_final_target(&self) -> bool {
        (self.current_setpoint_c - self.final_target_c).abs() < 1e-6
    }

    /// The value we'd pass to `set_target_temperature` given the current
    /// logical setpoint — integer, rounded. SDK call should only be issued
    /// when this differs from `last_commanded_i64`.
    fn commanded_i64(&self) -> i64 {
        self.current_setpoint_c.round() as i64
    }
}

/// Spawn the monitor thread. Returns a sender the caller (lifecycle) uses
/// to issue commands.
pub fn spawn(
    state: Arc<AppState>,
    camera_name: String,
    rt: tokio::runtime::Handle,
) -> mpsc::Sender<MonitorCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("camera-monitor-{}", camera_name))
        .spawn(move || run(state, camera_name, rt, rx))
        .expect("failed to spawn camera monitor thread");
    tx
}

struct MonitorCtx {
    state: Arc<AppState>,
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
}

fn run(
    state: Arc<AppState>,
    camera_name: String,
    rt: tokio::runtime::Handle,
    rx: mpsc::Receiver<MonitorCmd>,
) {
    debug!(camera_name, "Camera monitor thread started");

    let mut ctx = MonitorCtx {
        state,
        camera_name,
        rt,
        paused_for_capture: false,
        warming_up: false,
        warmup_started_at: None,
        settle_samples: 0,
        warm_samples: 0,
        cooldown_ramp: None,
        warmup_ramp: None,
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
        push_raw_setpoint(ctx, final_target);
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
        let mut guard = ctx
            .state
            .active_camera
            .lock()
            .expect("active_camera mutex poisoned");
        if let Some(cam) = guard.as_mut() {
            if let Err(e) = cam.set_cooler(false) {
                warn!(error = %e, "Failed to disable cooler at fast-warmup start");
            }
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
    push_setpoint(ctx, &ramp);
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
    let status = match read_status(ctx) {
        Some(s) => s,
        None => return true, // transient error; keep running
    };

    // Broadcast the sample for the UI.
    let target = ctx.rt.block_on(ctx.state.settings.read()).target_temp_c;
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
                    push_setpoint(ctx, &snapshot);
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
                    push_setpoint(ctx, &snapshot);
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
                if let Some(cam) = ctx
                    .state
                    .active_camera
                    .lock()
                    .expect("active_camera mutex poisoned")
                    .as_mut()
                {
                    if let Err(e) = cam.set_cooler(false) {
                        warn!(error = %e, "Failed to disable cooler at warmup finalize");
                    }
                }
                ctx.warmup_ramp = None;

                // Finalize disconnect from the monitor thread. `finalize_disconnect`
                // will clear `camera_monitor_tx` (our sender) and close the handle.
                let state = Arc::clone(&ctx.state);
                let name = ctx.camera_name.clone();
                ctx.rt.block_on(async move {
                    lifecycle::finalize_disconnect(&state, &name).await;
                });
                return false;
            }
        }
        CameraPhase::Idle | CameraPhase::Capturing | CameraPhase::Disconnected => {
            // Nothing to do — status was broadcast above.
            ctx.settle_samples = 0;
            ctx.warm_samples = 0;
        }
    }

    true
}

fn read_status(ctx: &MonitorCtx) -> Option<CameraStatus> {
    let start = Instant::now();
    let mut guard = ctx
        .state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned");
    let camera = guard.as_mut()?;
    let result = camera.status();
    let elapsed = start.elapsed();
    if elapsed > Duration::from_millis(500) {
        warn!(
            camera_name = %ctx.camera_name,
            elapsed_ms = elapsed.as_millis(),
            "camera.status() was slow"
        );
    }
    match result {
        Ok(status) => Some(status),
        Err(e) => {
            debug!(camera_name = %ctx.camera_name, error = %e, "Failed to read camera status");
            None
        }
    }
}

/// Read the current sensor temperature for ramp seeding. Prefers a fresh
/// hardware sample, falling back to the last cached status if the handle is
/// unavailable (e.g. momentarily held by another path).
fn current_sensor_temp(ctx: &MonitorCtx) -> Option<f64> {
    if let Some(status) = read_status(ctx) {
        return Some(status.temperature_c);
    }
    ctx.rt
        .block_on(ctx.state.get_camera_status(&ctx.camera_name))
        .map(|s| s.temperature_c)
}

/// Push the ramp's current integer setpoint to the camera. Best-effort: a
/// failed SDK call is logged but does not abort the ramp — the next tick will
/// retry.
fn push_setpoint(ctx: &MonitorCtx, ramp: &RampState) {
    push_raw_setpoint(ctx, ramp.current_setpoint_c);
}

/// Push an arbitrary setpoint (°C) to the camera, bypassing any ramp. Used
/// by the fast-mode path and by `push_setpoint` above.
fn push_raw_setpoint(ctx: &MonitorCtx, temp_c: f64) {
    let mut guard = ctx
        .state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned");
    let Some(cam) = guard.as_mut() else {
        return;
    };
    if let Err(e) = cam.set_target_temperature(temp_c) {
        warn!(
            camera_name = %ctx.camera_name,
            setpoint = temp_c,
            error = %e,
            "Failed to push setpoint"
        );
    }
}

#[cfg(test)]
mod ramp_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn step_respects_rate_cooling() {
        let now = Instant::now();
        let mut ramp = RampState::new_from_current(20.0, -15.0, now);
        let later = now + Duration::from_secs(12);
        let sp = ramp.step(later);
        // At 1200 °C/min (test rate) over 12 s, a huge delta — clamped to target.
        // So instead use a direct rate calc in the assertion:
        let expected_step = 12.0 * RAMP_RATE_C_PER_MIN / 60.0;
        let expected = (20.0 - expected_step).max(-15.0);
        assert!(
            (sp - expected).abs() < 1e-6,
            "expected {}, got {}",
            expected,
            sp
        );
    }

    #[test]
    fn step_clamps_to_target_cooling() {
        let now = Instant::now();
        let mut ramp = RampState::new_from_current(-14.9, -15.0, now);
        let later = now + Duration::from_secs(600); // very long dt
        let sp = ramp.step(later);
        assert!((sp - -15.0).abs() < 1e-6);
        assert!(ramp.is_at_final_target());
    }

    #[test]
    fn step_clamps_to_target_warming() {
        let now = Instant::now();
        let mut ramp = RampState::new_from_current(19.9, 20.0, now);
        let later = now + Duration::from_secs(600);
        let sp = ramp.step(later);
        assert!((sp - 20.0).abs() < 1e-6);
        assert!(ramp.is_at_final_target());
    }

    #[test]
    fn step_wall_clock_catches_up() {
        // Missed ticks: a 3-second gap should produce a 3-second worth step
        // (not one tick worth). Target chosen far away so the clamp doesn't
        // fire — the point is to verify wall-clock accounting, not clamping.
        let now = Instant::now();
        let mut ramp = RampState::new_from_current(20.0, -1000.0, now);
        let later = now + Duration::from_secs(3);
        let sp = ramp.step(later);
        let expected = 20.0 - (3.0 * RAMP_RATE_C_PER_MIN / 60.0);
        assert!(
            (sp - expected).abs() < 1e-6,
            "expected {}, got {}",
            expected,
            sp
        );
    }

    #[test]
    fn direction_inferred_from_endpoints() {
        let now = Instant::now();
        let cooling = RampState::new_from_current(20.0, -15.0, now);
        assert_eq!(cooling.direction, RampDirection::Cooling);
        let warming = RampState::new_from_current(-15.0, 20.0, now);
        assert_eq!(warming.direction, RampDirection::Warming);
    }

    #[test]
    fn commanded_i64_rounds_to_nearest() {
        let now = Instant::now();
        let mut ramp = RampState::new_from_current(0.4, -10.0, now);
        assert_eq!(ramp.commanded_i64(), 0);
        ramp.current_setpoint_c = -0.6;
        assert_eq!(ramp.commanded_i64(), -1);
        ramp.current_setpoint_c = -0.4;
        assert_eq!(ramp.commanded_i64(), 0);
    }
}
