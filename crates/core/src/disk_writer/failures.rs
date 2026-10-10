//! What the worker says when frames stop reaching the disk.

use std::time::{Duration, Instant};
use tracing::{error, info};

use super::error::DiskWriterError;

/// Told when writes start failing; the session layer forwards it to the observer.
pub type FailureSink = Box<dyn Fn(&DiskWriterError) + Send>;

/// How often a run of failures may put a line in the log.
const LOG_INTERVAL: Duration = Duration::from_secs(10);
/// How often the observer may be told, so a disk that flaps between failing and
/// working does not raise a notice per frame.
const NOTIFY_INTERVAL: Duration = Duration::from_secs(60);

/// A failed write was only ever logged, a line per frame and never to the UI: a disk
/// that filled up or went read-only mid-session lost the rest of the night silently.
/// One notice per episode (failing until a write succeeds), logs rate-limited.
#[derive(Default)]
pub(crate) struct WriteFailures {
    sink: Option<FailureSink>,
    failing: bool,
    unlogged: u64,
    last_logged: Option<Instant>,
    last_notified: Option<Instant>,
}

impl WriteFailures {
    pub(crate) fn report_to(&mut self, sink: FailureSink) {
        self.sink = Some(sink);
    }

    pub(crate) fn failed(&mut self, error: &DiskWriterError, frame_number: u64, now: Instant) {
        self.unlogged += 1;
        if self.last_logged.is_none_or(|at| now.duration_since(at) >= LOG_INTERVAL) {
            error!(error = %error, frame_number, failed = self.unlogged, "Failed to write frames");
            self.unlogged = 0;
            self.last_logged = Some(now);
        }

        if std::mem::replace(&mut self.failing, true) {
            return;
        }
        if self.last_notified.is_some_and(|at| now.duration_since(at) < NOTIFY_INTERVAL) {
            return;
        }
        self.last_notified = Some(now);
        if let Some(sink) = &self.sink {
            sink(error);
        }
    }

    pub(crate) fn succeeded(&mut self) {
        if !std::mem::replace(&mut self.failing, false) {
            return;
        }
        if self.unlogged > 0 {
            error!(failed = self.unlogged, "Failed to write frames");
        }
        info!("Frames are reaching the disk again");
        self.unlogged = 0;
        self.last_logged = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn counting() -> (WriteFailures, Arc<AtomicUsize>) {
        let told = Arc::new(AtomicUsize::new(0));
        let mut failures = WriteFailures::default();
        let count = Arc::clone(&told);
        failures.report_to(Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        }));
        (failures, told)
    }

    fn error() -> DiskWriterError {
        DiskWriterError::WriteFailed("Read-only file system".to_string())
    }

    #[test]
    fn a_run_of_failures_is_reported_once() {
        let (mut failures, told) = counting();
        let start = Instant::now();
        for n in 0..500 {
            failures.failed(&error(), n, start + Duration::from_secs(n));
        }
        assert_eq!(told.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_new_episode_after_a_recovery_is_reported_again() {
        let (mut failures, told) = counting();
        let start = Instant::now();
        failures.failed(&error(), 1, start);
        failures.succeeded();
        failures.failed(&error(), 2, start + NOTIFY_INTERVAL);
        assert_eq!(told.load(Ordering::SeqCst), 2);
    }

    /// A disk alternating between failing and working must not notify per frame.
    #[test]
    fn a_flapping_disk_is_reported_at_most_once_per_interval() {
        let (mut failures, told) = counting();
        let start = Instant::now();
        for n in 0..50 {
            failures.failed(&error(), n, start + Duration::from_secs(n));
            failures.succeeded();
        }
        assert_eq!(told.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn successes_alone_report_nothing() {
        let (mut failures, told) = counting();
        failures.succeeded();
        failures.succeeded();
        assert_eq!(told.load(Ordering::SeqCst), 0);
    }
}
