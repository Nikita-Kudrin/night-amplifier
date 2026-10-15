//! A slot's reopen held back while the other camera on its SDK reopens alone
//! (`camera::recovery::isolate_from_sibling`). Released by that camera's first frame or
//! the end of its recovery; the deadline bounds the hold, so a release that never comes
//! delays the reopen and never prevents it.

use std::time::{Duration, Instant};
use tokio::sync::watch;

/// How often a waiter re-checks whether it is still wanted, between wake-ups.
const RECHECK: Duration = Duration::from_millis(250);

pub struct ReopenHold {
    until: watch::Sender<Option<Instant>>,
}

impl Default for ReopenHold {
    fn default() -> Self {
        Self { until: watch::channel(None).0 }
    }
}

impl ReopenHold {
    pub fn hold_until(&self, deadline: Instant) {
        self.until.send_replace(Some(deadline));
    }

    pub fn release(&self) {
        self.until.send_if_modified(|until| until.take().is_some());
    }

    pub fn is_held(&self) -> bool {
        self.until.borrow().is_some_and(|deadline| deadline > Instant::now())
    }

    /// Return once the hold is released or past its deadline, or `still_wanted` says the
    /// reopen it holds back is no longer coming.
    pub async fn wait(&self, still_wanted: impl Fn() -> bool) {
        let mut until = self.until.subscribe();
        loop {
            let Some(deadline) = *until.borrow_and_update() else {
                return;
            };
            let now = Instant::now();
            if now >= deadline || !still_wanted() {
                return;
            }
            let _ = tokio::time::timeout((deadline - now).min(RECHECK), until.changed()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn an_unheld_slot_does_not_wait() {
        let hold = ReopenHold::default();
        assert!(!hold.is_held());
        tokio::time::timeout(Duration::from_millis(50), hold.wait(|| true))
            .await
            .expect("waited on nothing");
    }

    #[tokio::test]
    async fn a_release_wakes_the_waiter_before_the_deadline() {
        let hold = Arc::new(ReopenHold::default());
        hold.hold_until(Instant::now() + Duration::from_secs(60));
        assert!(hold.is_held());
        let waiter = {
            let hold = Arc::clone(&hold);
            tokio::spawn(async move { hold.wait(|| true).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        hold.release();

        tokio::time::timeout(Duration::from_millis(200), waiter)
            .await
            .expect("the release woke nobody")
            .unwrap();
        assert!(!hold.is_held());
    }

    /// A camera that never frames and never ends its recovery delays the sibling's
    /// reopen by the deadline, not for good.
    #[tokio::test]
    async fn the_deadline_ends_a_hold_nobody_releases() {
        let hold = ReopenHold::default();
        hold.hold_until(Instant::now() + Duration::from_millis(60));
        tokio::time::timeout(Duration::from_millis(500), hold.wait(|| true))
            .await
            .expect("the hold outlived its deadline");
        assert!(!hold.is_held());
    }

    /// The held reopen was called off (the observer disconnected the sibling): stop
    /// waiting, so the supervisor notices and ends.
    #[tokio::test]
    async fn a_reopen_no_longer_wanted_stops_waiting() {
        let hold = ReopenHold::default();
        hold.hold_until(Instant::now() + Duration::from_secs(60));
        tokio::time::timeout(Duration::from_millis(500), hold.wait(|| false))
            .await
            .expect("waited for a reopen nobody wants");
    }
}
