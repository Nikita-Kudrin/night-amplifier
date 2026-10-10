//! The guide camera's sensor readout and cooler, which the monitor cannot run for it: the
//! loop owns the handle for as long as it runs.

use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

use crate::camera::ramp::RampState;
use crate::state::{AppState, CameraCaptureProfile, ConnectedCameraInfo};
use night_amplifier_core::camera::{Camera, CameraStatus};

/// How often the loop reads the sensor and broadcasts a status sample.
///
/// Matched to the monitor's `PHASE_POLL_INTERVAL` so the two cameras' readouts update
/// at the same rate, rather than the guide camera's tracking its exposure length.
const STATUS_INTERVAL: Duration = Duration::from_secs(2);

/// The guide camera's status sampling, which the monitor cannot do for it.
///
/// `status()` is called straight rather than through `monitor::FfiWorker`: that worker
/// exists to keep a stalled vendor call off a *shared* handle and off the tokio
/// runtime, and this loop is neither — it is a dedicated OS thread that already blocks
/// on `capture()` against the same device. A stall here wedges the guide loop exactly
/// as a stalled exposure does, and `guide_task::stop` already gives up on a wedged loop.
#[derive(Default)]
pub(super) struct SensorReadout {
    last_sampled_at: Option<std::time::Instant>,
    /// Last temperature read, used to start the cooler ramp from where the sensor
    /// actually is rather than from its target.
    pub(super) temperature_c: Option<f64>,
}

impl SensorReadout {
    pub(super) fn sample(
        &mut self,
        camera: &dyn Camera,
        state: &Arc<AppState>,
        camera_info: &ConnectedCameraInfo,
        profile: &CameraCaptureProfile,
    ) {
        let now = std::time::Instant::now();
        let due = self
            .last_sampled_at
            .is_none_or(|last| now.duration_since(last) >= STATUS_INTERVAL);
        if !due {
            return;
        }
        self.last_sampled_at = Some(now);

        let status: CameraStatus = match camera.status() {
            Ok(status) => status,
            Err(e) => {
                debug!(error = %e, "Guide camera status read failed");
                return;
            }
        };
        if status.has_plausible_temperature() {
            self.temperature_c = Some(status.temperature_c);
        }
        state.update_camera_status(&camera_info.info.name, status, profile.target_temp_c);
    }
}

/// The guide camera's TEC setpoint, ramped by the loop that owns its handle.
///
/// Without this the per-frame config pushed the user's final target straight at the
/// sensor, so `RAMP_RATE_C_PER_MIN` — the 5 °C/min limit that exists to keep a cover
/// glass from condensing and a die from being thermally shocked — applied to the
/// imaging camera and not to this one.
#[derive(Default)]
pub(super) struct GuideCooler {
    ramp: Option<RampState>,
    /// The cooler settings the current ramp was built for. A settings edit changes this
    /// and restarts the ramp from wherever the sensor has got to.
    goal: Option<(bool, Option<f64>, bool)>,
}

impl GuideCooler {
    /// The setpoint this frame's config should carry, or `None` to leave it alone.
    ///
    /// `None` covers the three cases with nothing to ramp: the cooler is off, no target
    /// is set, or fast mode is on — which is documented to snap to the setpoint and is
    /// the switch a user flips when they accept that.
    pub(super) fn setpoint(
        &mut self,
        profile: &CameraCaptureProfile,
        sensor_temp_c: Option<f64>,
        now: std::time::Instant,
    ) -> Option<f64> {
        let goal = (
            profile.cooler_enabled,
            profile.target_temp_c,
            profile.cooler_fast_mode,
        );
        if self.goal != Some(goal) {
            self.goal = Some(goal);
            self.ramp = None;
        }

        let target = profile.target_temp_c?;
        if !profile.cooler_enabled || profile.cooler_fast_mode {
            self.ramp = None;
            return None;
        }

        let ramp = self.ramp.get_or_insert_with(|| {
            // Starting from the target rather than an unknown sensor temperature would
            // be a ramp that is already finished, i.e. no ramp at all.
            let start = sensor_temp_c.unwrap_or(target);
            RampState::new_from_current(start, target, now)
        });
        ramp.step(now);
        // The rounded value, not the logical one: a fractional setpoint that moves every
        // frame would make the backend re-issue `set_target_temperature` every frame,
        // which is exactly what `commanded_i64` exists to avoid.
        Some(ramp.commanded_i64() as f64)
    }
}
