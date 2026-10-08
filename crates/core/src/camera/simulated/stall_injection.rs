//! Debug-build stall injection for the simulated camera, so the recovery ladder can be
//! watched end to end in a running server rather than only in unit tests.
//!
//! `NIGHT_AMPLIFIER_SIM_STALL_EVERY=N` stalls the last `NIGHT_AMPLIFIER_SIM_STALL_RUN`
//! (default 1) captures of every N. A run of 1 exercises the in-place stream restart; a
//! run of `STALL_ESCALATION` or more escalates to a quiet reconnect. The count is
//! process-wide, so it carries on across the reopen. Compiled out of release builds.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Whether capture number `index` (from 1) falls in the stalled tail of its cycle.
fn stall_due(index: u64, every: u64, run: u64) -> bool {
    if every == 0 || run == 0 {
        return false;
    }
    let position = (index - 1) % every + 1;
    position > every.saturating_sub(run)
}

fn configured() -> Option<(u64, u64)> {
    static CONFIG: OnceLock<Option<(u64, u64)>> = OnceLock::new();
    *CONFIG.get_or_init(|| {
        let every = std::env::var("NIGHT_AMPLIFIER_SIM_STALL_EVERY").ok()?.parse().ok()?;
        let run = std::env::var("NIGHT_AMPLIFIER_SIM_STALL_RUN")
            .ok()
            .and_then(|run| run.parse().ok())
            .unwrap_or(1);
        tracing::warn!(every, run, "Simulated camera stall injection is on");
        Some((every, run))
    })
}

/// Count one capture and report whether it should stall.
pub(super) fn stall_now() -> bool {
    static CAPTURES: AtomicU64 = AtomicU64::new(0);
    let Some((every, run)) = configured() else {
        return false;
    };
    stall_due(CAPTURES.fetch_add(1, Ordering::Relaxed) + 1, every, run)
}

#[cfg(test)]
mod tests {
    use super::stall_due;

    #[test]
    fn a_run_of_one_stalls_every_nth_capture() {
        let stalled: Vec<u64> = (1..=25).filter(|&i| stall_due(i, 10, 1)).collect();
        assert_eq!(stalled, vec![10, 20]);
    }

    #[test]
    fn a_longer_run_stalls_the_tail_of_each_cycle() {
        let stalled: Vec<u64> = (1..=20).filter(|&i| stall_due(i, 10, 3)).collect();
        assert_eq!(stalled, vec![8, 9, 10, 18, 19, 20]);
    }

    #[test]
    fn nothing_stalls_when_unset_or_degenerate() {
        assert!(!(1..=50).any(|i| stall_due(i, 0, 1)));
        assert!(!(1..=50).any(|i| stall_due(i, 10, 0)));
        assert!((1..=5).all(|i| stall_due(i, 5, 9)), "a run longer than the cycle stalls all");
    }
}
