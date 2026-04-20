//! Background camera status monitor — runs on a dedicated OS thread.
//!
//! Responsibilities:
//! - Poll `camera.status()` every `PHASE_POLL_INTERVAL` while the handle
//!   is in the pool (not taken by the capture thread).
//! - Broadcast `CameraStatusUpdated` so the UI has a live temperature feed.
//! - Drive `Precooling → Idle` transition when the sensor settles near
//!   the target for `STABILITY_SAMPLE_COUNT` consecutive samples.
//! - Drive the warmup sequence: on `StartWarmup`, disable the cooler and
//!   wait for the sensor to rise to `WARMUP_THRESHOLD_C`, then close the
//!   handle and broadcast `CameraDisconnected`.
//!
//! Uses an OS thread (not a tokio task) so a blocking FFI call — e.g., a
//! USB stall inside `camera.status()` — cannot poison a runtime worker.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use super::{
    lifecycle, PHASE_POLL_INTERVAL, PRECOOL_TOLERANCE_C, STABILITY_SAMPLE_COUNT, WARMUP_THRESHOLD_C,
    WARMUP_TIMEOUT,
};
use crate::camera::CameraStatus;
use crate::server::state::{AppState, CameraPhase, MonitorCmd};

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
            Ok(MonitorCmd::StartWarmup) => {
                start_warmup(&mut ctx);
                continue;
            }
            Ok(MonitorCmd::CancelWarmup) => {
                cancel_warmup(&mut ctx);
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

fn start_warmup(ctx: &mut MonitorCtx) {
    if ctx.warming_up {
        return;
    }
    ctx.warming_up = true;
    ctx.warmup_started_at = Some(Instant::now());
    ctx.warm_samples = 0;

    // Disable the cooler. Best-effort: if the SDK call fails we still
    // proceed with the warmup watcher using natural thermal rise.
    let mut guard = ctx
        .state
        .active_camera
        .lock()
        .expect("active_camera mutex poisoned");
    if let Some(cam) = guard.as_mut() {
        if let Err(e) = cam.set_cooler(false) {
            warn!(error = %e, "Failed to disable cooler at warmup start");
        }
    }
    info!(camera_name = %ctx.camera_name, "Warmup started");
}

fn cancel_warmup(ctx: &mut MonitorCtx) {
    if !ctx.warming_up {
        return;
    }
    ctx.warming_up = false;
    ctx.warmup_started_at = None;
    ctx.warm_samples = 0;
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
    let target = ctx
        .rt
        .block_on(ctx.state.settings.read())
        .target_temp_c;
    ctx.rt
        .block_on(ctx.state.update_camera_status(&ctx.camera_name, status.clone(), target));

    let phase = ctx.rt.block_on(ctx.state.camera_phase(&ctx.camera_name));

    match phase {
        CameraPhase::Precooling => {
            if let Some(target) = target {
                if (status.temperature_c - target).abs() <= PRECOOL_TOLERANCE_C {
                    ctx.settle_samples = ctx.settle_samples.saturating_add(1);
                    if ctx.settle_samples >= STABILITY_SAMPLE_COUNT {
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
                // StartWarmup. Kick off the warmup now.
                start_warmup(ctx);
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
