//! Stop's bound on the stacker: frames still queued stack for [`STOP_DRAIN`] after Stop,
//! the rest are taken off unstacked, and the stack is saved.
//!
//! On 2026-10-24 each Moon frame took 134 s to align and Stop waited for the 19 queued
//! behind it: 42 minutes in *Stopping*, the camera held, until the power was cut and no
//! stack was saved. A Stop means stop. The raw frames and the SER are not capped (they
//! are the observer's data and the storage task keeps them); only stacking them is, and
//! the frame being stacked when the time runs out still finishes. A capture that ends on
//! its own (a camera fault) is not a Stop, and drains whole as before.

use std::time::Duration;

use tracing::info;

/// How long after Stop queued frames still join the stack.
pub const STOP_DRAIN: Duration = Duration::from_secs(5);

/// Decides frame by frame whether the stacker still stacks, and counts what it skips.
pub(super) struct StopDrain {
    budget: Duration,
    skipped: usize,
}

impl StopDrain {
    pub(super) fn new(budget: Duration) -> Self {
        Self { budget, skipped: 0 }
    }

    /// Whether to stack a frame taken `cancelled_for` after Stop (`None`: no Stop).
    pub(super) fn admits(&mut self, cancelled_for: Option<Duration>) -> bool {
        if cancelled_for.is_some_and(|since| since >= self.budget) {
            self.skipped += 1;
            return false;
        }
        true
    }

    /// One line for the whole drain, not one per skipped frame.
    pub(super) fn finish(&self) {
        if self.skipped > 0 {
            info!(
                skipped = self.skipped,
                budget_s = self.budget.as_secs_f64(),
                "Stop: frames still queued past the drain were not stacked (their raw files are saved)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_stack_until_the_drain_runs_out_after_stop() {
        let mut drain = StopDrain::new(STOP_DRAIN);
        assert!(drain.admits(None), "no Stop");
        assert!(drain.admits(Some(Duration::ZERO)));
        assert!(drain.admits(Some(Duration::from_millis(4_999))));
        assert!(!drain.admits(Some(STOP_DRAIN)));
        assert!(!drain.admits(Some(Duration::from_secs(60))));
        assert_eq!(drain.skipped, 2);
    }
}
