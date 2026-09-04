//! The rate-limited TEC setpoint ramp, shared by everything that drives a cooler.
//!
//! Its own module because it has two drivers now: the monitor, for a camera parked in
//! its slot, and `capture::guide_task`, for a guide camera whose handle the monitor can
//! never check out. One ramp implementation is the only way `RAMP_RATE_C_PER_MIN` means
//! the same thing on both cameras.

use std::time::Instant;

use super::RAMP_RATE_C_PER_MIN;

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
pub(crate) struct RampState {
    final_target_c: f64,
    pub(crate) current_setpoint_c: f64,
    pub(crate) last_commanded_i64: Option<i64>,
    last_tick_at: Instant,
    direction: RampDirection,
}

impl RampState {
    pub(crate) fn new_from_current(start_c: f64, final_target_c: f64, now: Instant) -> Self {
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
    pub(crate) fn step(&mut self, now: Instant) -> f64 {
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

    pub(crate) fn is_at_final_target(&self) -> bool {
        (self.current_setpoint_c - self.final_target_c).abs() < 1e-6
    }

    /// The value we'd pass to `set_target_temperature` given the current
    /// logical setpoint — integer, rounded. SDK call should only be issued
    /// when this differs from `last_commanded_i64`.
    pub(crate) fn commanded_i64(&self) -> i64 {
        self.current_setpoint_c.round() as i64
    }
}

#[cfg(test)]
mod tests {
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
