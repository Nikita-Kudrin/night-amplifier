//! Camera lifecycle management (connect → precool → capture → warmup → disconnect)
//!
//! Owns the long-lived camera handle stored in `AppState.active_camera`,
//! and runs a background `std::thread` monitor that polls sensor status
//! every `PHASE_POLL_INTERVAL` while broadcasting `CameraStatusUpdated`
//! and `CameraPhaseChanged` events.
//!
//! The monitor runs on an OS thread (not a tokio task) because vendor
//! FFI calls (`camera.status()`, `camera.set_cooler(...)`) can block under
//! USB stall — we must never hold the runtime worker across those calls.

use std::time::Duration;

pub mod lifecycle;
pub mod monitor;

#[cfg(test)]
mod tests;

/// Cadence for polling cooled-camera status from the monitor thread.
pub const PHASE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Temperature window (°C) within which the sensor is considered "settled"
/// at the target — used for `Precooling → Idle` transition.
pub const PRECOOL_TOLERANCE_C: f64 = 1.5;

/// Sensor temperature (°C) at which warmup is considered complete and the
/// USB handle can be closed safely (minimum — to avoid dew condensation).
pub const WARMUP_THRESHOLD_C: f64 = 10.0;

/// Hard cap on warmup duration. If the TEC refuses to ramp, we force-close
/// after this timeout and log a warning.
pub const WARMUP_TIMEOUT: Duration = Duration::from_secs(300);

/// Number of consecutive samples required for a transition (debounce).
pub const STABILITY_SAMPLE_COUNT: u32 = 2;

/// Maximum rate at which the commanded TEC setpoint may change (°C per minute).
/// Both cooldown and warmup are rate-limited to this value to avoid mechanical
/// stress on the sensor die and to prevent condensation on a rapidly-cooling
/// cover glass. Industry practice for cooled astronomy cameras.
#[cfg(not(test))]
pub const RAMP_RATE_C_PER_MIN: f64 = 5.0;

/// Test-time shadow of `RAMP_RATE_C_PER_MIN`. Set effectively-unbounded so the
/// state machine is still exercised but integration tests don't take minutes.
/// A unit test in `tests.rs` asserts the production constant is 5.0.
#[cfg(test)]
pub const RAMP_RATE_C_PER_MIN: f64 = 1200.0;

/// Warmup endpoint (°C). The setpoint ramps up to this value during warmup;
/// setting it slightly above typical room temperature drives the internal PID
/// duty cycle toward 0 % once it crosses ambient.
pub const WARMUP_RAMP_TARGET_C: f64 = 20.0;
