//! Which guide loop is the current one.
//!
//! A loop is registered by `guide_task::start` and unregistered by `stop` — or by the loop
//! itself when it ends. Every write names the loop it is about, because a loop that `stop`
//! gave up waiting for can end *after* its successor started: with one bare flag and one
//! bare token, its exit cleared the new loop's "running" and its stop switch, and the new
//! loop could no longer be stopped.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

/// The registry. `running` is an atomic so the stacking thread can read it per frame.
#[derive(Default)]
pub struct GuideLoops {
    current: StdMutex<Option<Registered>>,
    running: AtomicBool,
    next_id: AtomicU64,
}

struct Registered {
    id: u64,
    cancel: Arc<AtomicBool>,
}

/// What a loop gets at registration: who it is, and the switch that stops it.
#[derive(Debug, Clone)]
pub struct GuideLoopTicket {
    pub id: u64,
    pub cancel: Arc<AtomicBool>,
}

impl GuideLoops {
    /// Register a new loop, or `None` while one is already registered — starting, running
    /// or stopping on its own. A second loop would compete with it for the one handle.
    pub fn register(&self) -> Option<GuideLoopTicket> {
        let mut current = self.lock();
        if current.is_some() {
            return None;
        }
        let ticket = GuideLoopTicket {
            id: self.next_id.fetch_add(1, Ordering::SeqCst) + 1,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        *current = Some(Registered {
            id: ticket.id,
            cancel: Arc::clone(&ticket.cancel),
        });
        Some(ticket)
    }

    /// The loop is exposing. Ignored for a loop that is no longer the registered one.
    pub fn mark_running(&self, id: u64) {
        let current = self.lock();
        if current.as_ref().is_some_and(|loop_| loop_.id == id) {
            self.running.store(true, Ordering::SeqCst);
        }
    }

    /// The loop is over. Unregisters it only if it is still the current one.
    pub fn finish(&self, id: u64) {
        let mut current = self.lock();
        if current.as_ref().is_some_and(|loop_| loop_.id == id) {
            *current = None;
            self.running.store(false, Ordering::SeqCst);
        }
    }

    /// Unregister the current loop and hand back its stop switch. `None` when no loop is
    /// registered, which is what makes stopping idempotent.
    pub fn take(&self) -> Option<Arc<AtomicBool>> {
        let mut current = self.lock();
        self.running.store(false, Ordering::SeqCst);
        current.take().map(|loop_| loop_.cancel)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Whether a loop is registered, whether or not it has started exposing yet.
    pub fn is_registered(&self) -> bool {
        self.lock().is_some()
    }

    /// Set the running flag without a loop behind it — for tests that simulate one.
    pub fn force_running(&self, running: bool) {
        self.running.store(running, Ordering::SeqCst);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Registered>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_runs_once_marked_and_stops_when_taken() {
        let loops = GuideLoops::default();
        let ticket = loops.register().expect("nothing registered yet");
        assert!(!loops.is_running(), "registered is not yet exposing");
        loops.mark_running(ticket.id);
        assert!(loops.is_running());

        let cancel = loops.take().expect("the loop's switch");
        assert!(Arc::ptr_eq(&cancel, &ticket.cancel));
        assert!(!loops.is_running());
        assert!(!loops.is_registered());
        assert!(loops.take().is_none(), "a second stop finds nothing");
    }

    #[test]
    fn a_second_loop_is_refused_while_one_is_registered() {
        let loops = GuideLoops::default();
        let _first = loops.register().unwrap();
        assert!(loops.register().is_none());
    }

    /// The race this module exists for: the old loop outlived `stop`'s wait and ended after
    /// its successor had started.
    #[test]
    fn a_stopped_loop_ending_late_leaves_its_successor_alone() {
        let loops = GuideLoops::default();
        let old = loops.register().unwrap();
        loops.mark_running(old.id);
        loops.take();

        let new = loops.register().expect("stop unregistered the old loop");
        loops.mark_running(new.id);

        loops.mark_running(old.id);
        loops.finish(old.id);

        assert!(loops.is_running(), "the old loop's exit cleared the new loop's flag");
        let cancel = loops.take().expect("the old loop's exit dropped the new loop's switch");
        assert!(Arc::ptr_eq(&cancel, &new.cancel));
    }

    #[test]
    fn a_loop_ending_on_its_own_unregisters_itself() {
        let loops = GuideLoops::default();
        let ticket = loops.register().unwrap();
        loops.mark_running(ticket.id);
        loops.finish(ticket.id);
        assert!(!loops.is_running());
        assert!(loops.register().is_some(), "a new loop may start");
    }
}
