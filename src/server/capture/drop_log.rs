//! Rate limiting for the per-frame "we dropped it" warnings.

use std::time::{Duration, Instant};

/// How often a stream of drops may put a line in the log.
const REPORT_INTERVAL: Duration = Duration::from_secs(2);

/// Rate limiter for a warning that would otherwise fire once per dropped frame.
///
/// One line per drop is proportionate at 30-second subs, where a drop is a rare event
/// worth a log entry of its own. Raw saving in Live view makes 100 ms subs reachable,
/// and a disk that cannot keep up there drops most of them — ten lines a second for the
/// length of the session, which buries everything else in the log and costs the
/// capturing thread the formatting.
///
/// Reports the first drop immediately (that one is news), then at most one line per
/// interval carrying the number suppressed since. The caller writes its own message:
/// the counts are what needs limiting, not what to say about them.
#[derive(Debug, Default)]
pub struct DropLog {
    dropped_since_report: u64,
    last_report: Option<Instant>,
}

impl DropLog {
    /// Count a dropped frame, returning how many to report if a line is due now.
    #[must_use]
    pub fn record(&mut self) -> Option<u64> {
        self.dropped_since_report += 1;

        let due = match self.last_report {
            None => true,
            Some(at) => at.elapsed() >= REPORT_INTERVAL,
        };
        if !due {
            return None;
        }
        self.last_report = Some(Instant::now());
        Some(std::mem::take(&mut self.dropped_since_report))
    }

    /// Take anything the interval swallowed, so the tail of a struggling session is not
    /// silently lost. Returns `None` when there is nothing outstanding.
    #[must_use]
    pub fn flush(&mut self) -> Option<u64> {
        if self.dropped_since_report == 0 {
            return None;
        }
        Some(std::mem::take(&mut self.dropped_since_report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first drop is news and goes out on its own; the ones behind it are counted
    /// rather than logged, which is the whole point.
    #[test]
    fn the_first_drop_reports_and_the_burst_behind_it_does_not() {
        let mut log = DropLog::default();

        assert_eq!(log.record(), Some(1));
        for _ in 0..100 {
            assert_eq!(log.record(), None);
        }
    }

    /// Every suppressed drop has to turn up in a later count, or the log understates a
    /// session that was losing frames the whole time.
    #[test]
    fn suppressed_drops_are_carried_into_the_flush() {
        let mut log = DropLog::default();

        assert_eq!(log.record(), Some(1));
        for _ in 0..9 {
            let _ = log.record();
        }

        assert_eq!(log.flush(), Some(9));
        assert_eq!(log.flush(), None, "a flush must not report the same drops twice");
    }

    #[test]
    fn a_session_that_dropped_nothing_flushes_nothing() {
        assert_eq!(DropLog::default().flush(), None);
    }
}
