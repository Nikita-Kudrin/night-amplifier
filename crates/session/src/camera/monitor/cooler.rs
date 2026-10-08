//! The monitor's cooler work: ramping toward a target, and the warm-up that ends a
//! session by switching the cooler off and closing the camera.

use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

use super::ffi_worker::{with_camera_bounded, FFI_CALL_TIMEOUT};
use super::{give_up_on_camera, read_status, MonitorCtx};
use crate::camera::health::FaultKind;
use crate::camera::lifecycle::{self, DisconnectCause};
use crate::camera::ramp::RampState;
use crate::camera::{WARMUP_RAMP_TARGET_C, WARMUP_TIMEOUT};

pub(super) fn handle_update_cooler_target(
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

pub(super) fn start_warmup(ctx: &mut MonitorCtx, fast: bool) {
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

pub(super) fn cancel_warmup(ctx: &mut MonitorCtx) {
    if !ctx.warming_up {
        return;
    }
    ctx.warming_up = false;
    ctx.warmup_started_at = None;
    ctx.warm_samples = 0;
    ctx.warmup_ramp = None;
    info!(camera_name = %ctx.camera_name, "Warmup cancelled");
}

/// Whether the warm-up has run past `WARMUP_TIMEOUT`.
pub(super) fn warmup_overran(ctx: &MonitorCtx) -> bool {
    ctx.warmup_started_at
        .is_some_and(|started| started.elapsed() >= WARMUP_TIMEOUT)
}

/// End a warm-up — complete or out of time — and the session with it.
pub(super) fn finish_warmup(ctx: &mut MonitorCtx) {
    // Disable the cooler here (moved from start_warmup). By this point the setpoint is at
    // or past ambient so duty is already near 0 % — this just latches it off before we
    // close.
    let result = with_camera_bounded(ctx, FFI_CALL_TIMEOUT, |cam| cam.set_cooler(false));
    if let Err(e) = result {
        warn!(error = %e, "Failed to disable cooler at warmup finalize");
    }
    ctx.warmup_ramp = None;

    // Finalize disconnect from the monitor thread. `finalize_disconnect` will clear this
    // slot's monitor sender (ours) and close the handle.
    let state = Arc::clone(&ctx.state);
    let name = ctx.camera_name.clone();
    let role = ctx.role;
    ctx.rt.block_on(async move {
        lifecycle::finalize_disconnect(&state, role, &name, DisconnectCause::Requested).await;
    });
}

/// Read the current sensor temperature for ramp seeding. Prefers a fresh
/// hardware sample, falling back to the last cached status if the handle is
/// unavailable (e.g. momentarily held by another path).
fn current_sensor_temp(ctx: &mut MonitorCtx) -> Option<f64> {
    if let Ok(status) = read_status(ctx) {
        return Some(status.temperature_c);
    }
    ctx.state.get_camera_status(&ctx.camera_name).map(|s| s.temperature_c)
}

/// Push the ramp's current integer setpoint to the camera. Best-effort: a
/// failed SDK call is logged but does not abort the ramp — the next tick will
/// retry.
pub(super) fn push_setpoint(ctx: &mut MonitorCtx, ramp: &RampState) -> bool {
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

    // A warm-up ends on the first sign the device is gone; see `tick`.
    if !e.is_device_lost() || !(ctx.fault_is_persistent || ctx.warming_up) {
        return true; // Transient, or not yet conclusive; the next tick retries.
    }
    give_up_on_camera(ctx, FaultKind::DeviceLost);
    false
}
