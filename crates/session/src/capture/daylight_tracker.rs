//! When the render task reads the camera's daylight balance off the Moon, and whether what
//! it read is worth saving. Every [`DaylightTracker::PERIOD`] in Planetary, the frame on screen
//! is measured (`render::planetary::TargetMeasure`); the balance it implies replaces the
//! camera's when it differs past [`DaylightTracker::DEAD_BAND`], since the Moon's colour holds
//! to 0.1 % frame to frame and a save a frame would thrash `settings.json` for nothing.

use std::time::{Duration, Instant};

use night_amplifier_core::render::planetary::DaylightBalance;

/// The clock behind the periodic measure. First call due at once, then every `PERIOD`.
#[derive(Debug, Default)]
pub struct DaylightTracker {
    last: Option<Instant>,
}

impl DaylightTracker {
    pub const PERIOD: Duration = Duration::from_secs(2);

    /// A gain moving by less than this share of itself is noise, not a change.
    pub const DEAD_BAND: f32 = 0.005;

    /// Whether a measure is due at `now`, and if so, starts the next period from it.
    pub fn due(&mut self, now: Instant) -> bool {
        let due = self.last.is_none_or(|last| now.duration_since(last) >= Self::PERIOD);
        if due {
            self.last = Some(now);
        }
        due
    }

    /// `implied`, if it differs from the balance `in_force` by more than the dead band.
    pub fn proposal(in_force: &DaylightBalance, implied: DaylightBalance) -> Option<DaylightBalance> {
        implied.differs_from(in_force, Self::DEAD_BAND).then_some(implied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_measure_is_due_at_once_and_the_next_a_period_later() {
        let mut tracker = DaylightTracker::default();
        let start = Instant::now();
        assert!(tracker.due(start));
        assert!(!tracker.due(start + Duration::from_millis(1999)));
        assert!(tracker.due(start + Duration::from_millis(2000)));
        assert!(!tracker.due(start + Duration::from_millis(3999)));
        assert!(tracker.due(start + Duration::from_millis(4000)), "periods count from the last measure");
    }

    #[test]
    fn a_change_within_the_dead_band_is_not_proposed() {
        let in_force = DaylightBalance { red: 1.17, green: 1.0, blue: 1.89 };
        assert_eq!(DaylightTracker::proposal(&in_force, DaylightBalance { red: 1.17, green: 1.0, blue: 1.895 }), None);
        let moved = DaylightBalance { red: 1.17, green: 1.0, blue: 1.92 };
        assert_eq!(DaylightTracker::proposal(&in_force, moved), Some(moved));
        assert_eq!(DaylightTracker::proposal(&DaylightBalance::PRIOR, in_force), Some(in_force), "the prior gives way");
    }
}
