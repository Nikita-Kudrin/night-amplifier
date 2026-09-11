//! One camera position and everything that is singular about it.
//!
//! Before roles existed these five fields sat directly on `AppState`, which is why a
//! second `connect` could only be implemented by *displacing* the first camera's handle
//! and closing it. Grouping them makes the invariant structural: a slot owns exactly one
//! device, and every consumer addresses a slot rather than "the camera".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Notify, RwLock};

use super::{CameraPhase, MonitorCmd};
use crate::camera::Camera;

/// A hardware call queued for whoever currently owns a slot's handle.
///
/// Only the dew heater needs this. Everything else a settings edit can change travels
/// to the camera inside `CaptureConfig`, which the owner rebuilds every frame — but
/// `CaptureConfig` carries no dew-heater field, so with the handle checked out there
/// was no path to the device at all and the switch simply did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraOp {
    SetDewHeater { enabled: bool, power: i32 },
}

/// The per-role half of what used to be `AppState`'s camera state.
pub struct CameraSlot {
    /// Long-lived camera handle. `Some` while connected and not checked out.
    ///
    /// `None` is ambiguous — "nothing connected" *or* "the monitor is mid-poll" — which
    /// is why readers go through `lifecycle::with_camera` and wait on
    /// [`Self::handle_returned`] instead of testing it directly.
    pub handle: StdMutex<Option<Box<dyn Camera>>>,
    /// Woken every time a handle is put back into `handle` — or fails to be, after a
    /// stall.
    pub handle_returned: Arc<Notify>,
    /// Sender for the monitor thread polling this slot's camera. `None` when no monitor
    /// is running.
    pub monitor_tx: StdMutex<Option<std::sync::mpsc::Sender<MonitorCmd>>>,
    /// Cancel token of the camera currently occupying the slot, so an in-flight exposure
    /// can be cut short when settings change.
    pub cancel_token: RwLock<Option<Arc<AtomicBool>>>,
    /// True while a reconnect supervisor is recovering this slot.
    ///
    /// Per-slot rather than global: a guide dropout during a main-camera recovery used to
    /// be refused outright by the single flag, leaving the guide camera down for the rest
    /// of the night. Two supervisors can never race for one device because a device
    /// belongs to exactly one slot.
    pub reconnect_in_flight: Arc<AtomicBool>,
    /// Where this slot is in quiet recovery. Read through [`Self::recovery`]; only
    /// `camera_session` moves it.
    recovery: StdMutex<Recovery>,
    /// Lifecycle phase of the camera in this slot. Per slot, not per model name: two
    /// bodies of one model otherwise shared one phase, and the imaging camera starting a
    /// capture read as the guide camera's recovery having ended.
    pub phase: RwLock<CameraPhase>,
    /// Bounded SDK calls against this slot's device that have not returned yet —
    /// including ones a watchdog gave up on. A reconnect waits for this to drain before
    /// reopening, so it does not open the device while an abandoned call is still inside
    /// the vendor SDK.
    pub sdk_calls: Arc<InFlightCalls>,
    /// Recovery opens the supervisor stopped waiting for. Unlike an abandoned capture, a
    /// late *open* takes the device lease when it returns and would supersede any handle
    /// opened in between, so nothing reopens while one is pending.
    pub pending_opens: Arc<InFlightCalls>,
    /// The raw-frame directory this slot's session was writing into, parked at
    /// disconnect so a reconnect rejoins it instead of scattering one observation across
    /// timestamped folders, together with the frame number to carry on from. Only the
    /// guide loop uses this; the main camera carries the equivalent on
    /// `SessionResumePlan`.
    pub raw_session: RwLock<Option<RawSessionResume>>,
    /// Hardware calls waiting for the handle's owner to run them. See [`CameraOp`].
    pending_ops: StdMutex<Vec<CameraOp>>,
}

/// Where a slot is in quiet recovery (`camera_session::recovery`).
///
/// One state rather than a flag, because a fault can arrive while a reopened handle is
/// still being installed: that fault belongs to the *new* handle, and a flag that still
/// read "recovering" swallowed it as a duplicate of the old one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    None,
    /// The handle was lost; the camera stays registered and a supervisor reopens it.
    Suspended,
    /// The supervisor is installing a reopened handle. `refaulted` records a fault on
    /// that handle, so the supervisor suspends again once the install has finished.
    Installing { refaulted: bool },
}

/// What [`CameraSlot::begin_suspend`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendVerdict {
    /// Newly suspended: the caller detaches the handle and starts a supervisor.
    Suspended,
    /// Already suspended — the same loss reported by a second call site.
    AlreadySuspended,
    /// The reopened handle failed mid-install; the supervisor handles it afterwards.
    Deferred,
}

/// How an install ended, from [`CameraSlot::finish_install`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Healthy,
    /// The new handle faulted during the install; the slot is suspended again.
    Refaulted,
    /// Recovery was ended (the camera was torn down) while installing.
    Ended,
}

/// A raw-frame directory to rejoin, and where its numbering had got to.
///
/// The number travels with the directory because the writer names files
/// `frame_{:06}.fits`: a resumed run that restarted at 1 wrote straight over the frames
/// the interrupted one had already saved.
#[derive(Debug, Clone)]
pub struct RawSessionResume {
    pub dir: PathBuf,
    /// The number the next frame written into `dir` should take.
    pub next_frame: u64,
}

impl Default for CameraSlot {
    fn default() -> Self {
        Self {
            handle: StdMutex::new(None),
            handle_returned: Arc::new(Notify::new()),
            monitor_tx: StdMutex::new(None),
            cancel_token: RwLock::new(None),
            reconnect_in_flight: Arc::new(AtomicBool::new(false)),
            recovery: StdMutex::new(Recovery::None),
            phase: RwLock::new(CameraPhase::Disconnected),
            sdk_calls: Arc::new(InFlightCalls::default()),
            pending_opens: Arc::new(InFlightCalls::default()),
            raw_session: RwLock::new(None),
            pending_ops: StdMutex::new(Vec::new()),
        }
    }
}

impl CameraSlot {
    /// Wake everyone waiting for the handle to come back.
    pub fn notify_handle_returned(&self) {
        self.handle_returned.notify_waiters();
    }

    /// Whether a handle is currently parked here. Only meaningful as a hint — a
    /// checked-out handle reads as absent; see [`Self::handle`].
    pub fn holds_handle(&self) -> bool {
        self.handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Send a command to this slot's monitor thread, if one is running.
    pub fn send_monitor_cmd(&self, cmd: MonitorCmd) {
        let guard = self.monitor_tx.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(cmd);
        }
    }

    /// Install the monitor sender, returning the one it replaces so the caller can
    /// shut down an orphan rather than dropping it silently.
    pub fn set_monitor_tx(
        &self,
        tx: Option<std::sync::mpsc::Sender<MonitorCmd>>,
    ) -> Option<std::sync::mpsc::Sender<MonitorCmd>> {
        std::mem::replace(
            &mut *self.monitor_tx.lock().unwrap_or_else(|e| e.into_inner()),
            tx,
        )
    }

    /// Queue a hardware call for whoever owns the handle, replacing any earlier call of
    /// the same kind — only the latest position of a slider is worth applying.
    pub fn queue_op(&self, op: CameraOp) {
        let mut ops = self.pending_ops.lock().unwrap_or_else(|e| e.into_inner());
        ops.retain(|queued| std::mem::discriminant(queued) != std::mem::discriminant(&op));
        ops.push(op);
    }

    /// Take everything queued. Called by the handle's owner between exposures.
    pub fn drain_ops(&self) -> Vec<CameraOp> {
        std::mem::take(&mut *self.pending_ops.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn recovery(&self) -> Recovery {
        *self.recovery.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Suspended or installing: registered, but not yet a camera anything may use.
    pub fn is_recovering(&self) -> bool {
        self.recovery() != Recovery::None
    }

    pub fn begin_suspend(&self) -> SuspendVerdict {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        match *recovery {
            Recovery::None => {
                *recovery = Recovery::Suspended;
                SuspendVerdict::Suspended
            }
            Recovery::Suspended => SuspendVerdict::AlreadySuspended,
            Recovery::Installing { .. } => {
                *recovery = Recovery::Installing { refaulted: true };
                SuspendVerdict::Deferred
            }
        }
    }

    /// Claim a suspended slot for installing a reopened handle. `false` when the slot is
    /// no longer suspended — recovery was ended while the device was being opened.
    pub fn begin_install(&self) -> bool {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        if *recovery != Recovery::Suspended {
            return false;
        }
        *recovery = Recovery::Installing { refaulted: false };
        true
    }

    pub fn finish_install(&self) -> InstallOutcome {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        match *recovery {
            Recovery::Installing { refaulted: false } => {
                *recovery = Recovery::None;
                InstallOutcome::Healthy
            }
            Recovery::Installing { refaulted: true } => {
                *recovery = Recovery::Suspended;
                InstallOutcome::Refaulted
            }
            Recovery::None | Recovery::Suspended => InstallOutcome::Ended,
        }
    }

    /// Give an install up before anything was registered — its hardware setup did not
    /// return — and suspend the slot again. `false` when recovery was ended meanwhile.
    pub fn abandon_install(&self) -> bool {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        if !matches!(*recovery, Recovery::Installing { .. }) {
            return false;
        }
        *recovery = Recovery::Suspended;
        true
    }

    /// Leave recovery for good. Returns whether the slot was recovering.
    pub fn end_recovery(&self) -> bool {
        let mut recovery = self.recovery.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::replace(&mut *recovery, Recovery::None) != Recovery::None
    }

    /// Cancel the exposure in flight on this slot, if any.
    pub async fn cancel_exposure(&self) {
        if let Some(token) = self.cancel_token.read().await.as_ref() {
            token.store(true, Ordering::SeqCst);
        }
    }
}

/// A count of calls in progress that can be waited on until it drains.
#[derive(Debug, Default)]
pub struct InFlightCalls {
    count: AtomicUsize,
    drained: Notify,
}

/// Why [`InFlightCalls::run_bounded`] came back without a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundedCallError {
    /// Still inside the vendor SDK at the deadline. It stays counted until it returns.
    TimedOut,
    Panicked,
}

/// Held for the length of one call; dropping it — on whatever thread the call finally
/// returns on — takes the call off the count.
#[must_use = "the call is counted only while the guard is alive"]
pub struct InFlightCall(Arc<InFlightCalls>);

impl InFlightCalls {
    pub fn begin(self: &Arc<Self>) -> InFlightCall {
        self.count.fetch_add(1, Ordering::SeqCst);
        InFlightCall(Arc::clone(self))
    }

    pub fn in_flight(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Run `call` on a blocking thread, counted, and wait for it up to `timeout`.
    ///
    /// A call past the deadline keeps running. What it eventually returns — an opened
    /// handle, typically — is dropped on its own thread *before* the call leaves the
    /// count, so a caller that waits for the count to drain never reopens a device whose
    /// late handle has not been closed yet.
    pub async fn run_bounded<T: Send + 'static>(
        self: &Arc<Self>,
        timeout: Duration,
        call: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, BoundedCallError> {
        let counted = self.begin();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            // An unclaimed result comes back as the send's error, dropped right here.
            let _ = tx.send(call());
            drop(counted);
        });
        match tokio::time::timeout(timeout, &mut rx).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(_)) => Err(BoundedCallError::Panicked),
            Err(_) => {
                rx.close();
                // Returned between the deadline and the close: take it rather than drop
                // it on this thread.
                rx.try_recv().map_err(|_| BoundedCallError::TimedOut)
            }
        }
    }

    /// Wait until no call is in flight, or `max` has passed. Returns whether it drained.
    pub async fn wait_drained(&self, max: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + max;
        loop {
            let drained = self.drained.notified();
            tokio::pin!(drained);
            drained.as_mut().enable();
            if self.in_flight() == 0 {
                return true;
            }
            tokio::select! {
                _ = &mut drained => {}
                _ = tokio::time::sleep_until(deadline) => return self.in_flight() == 0,
            }
        }
    }
}

impl Drop for InFlightCall {
    fn drop(&mut self) {
        if self.0.count.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.0.drained.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_report_of_one_loss_is_a_duplicate() {
        let slot = CameraSlot::default();
        assert_eq!(slot.begin_suspend(), SuspendVerdict::Suspended);
        assert_eq!(slot.begin_suspend(), SuspendVerdict::AlreadySuspended);
        assert_eq!(slot.recovery(), Recovery::Suspended);
    }

    /// A fault during the install is the new handle's: kept, and handed back as a
    /// suspension once the install ends, never mistaken for the loss being recovered.
    #[test]
    fn a_fault_during_an_install_suspends_again_when_it_finishes() {
        let slot = CameraSlot::default();
        slot.begin_suspend();
        assert!(slot.begin_install());
        assert!(slot.is_recovering(), "an install in progress still holds the slot");
        assert_eq!(slot.begin_suspend(), SuspendVerdict::Deferred);
        assert_eq!(slot.finish_install(), InstallOutcome::Refaulted);
        assert_eq!(slot.recovery(), Recovery::Suspended);
        assert!(slot.begin_install(), "the next attempt may install again");
        assert_eq!(slot.finish_install(), InstallOutcome::Healthy);
        assert_eq!(slot.recovery(), Recovery::None);
    }

    #[test]
    fn an_install_cannot_claim_a_slot_whose_recovery_was_ended() {
        let slot = CameraSlot::default();
        assert!(!slot.begin_install(), "nothing to install into a healthy slot");
        slot.begin_suspend();
        assert!(slot.end_recovery());
        assert!(!slot.begin_install());

        slot.begin_suspend();
        slot.begin_install();
        assert!(slot.end_recovery());
        assert_eq!(slot.finish_install(), InstallOutcome::Ended);
        assert!(!slot.end_recovery(), "already ended");
    }

    #[tokio::test]
    async fn waiting_returns_as_soon_as_the_last_call_ends() {
        let calls = Arc::new(InFlightCalls::default());
        let call = calls.begin();
        let waiter = {
            let calls = Arc::clone(&calls);
            tokio::spawn(async move { calls.wait_drained(Duration::from_secs(10)).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let started = std::time::Instant::now();
        // Ended on another thread, the way an abandoned SDK call finally returns.
        std::thread::spawn(move || drop(call)).join().unwrap();
        assert!(waiter.await.unwrap(), "should report drained");
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(calls.in_flight(), 0);
    }

    /// Records how many calls were still counted at the moment it was dropped.
    struct LateHandle(Arc<InFlightCalls>, Arc<AtomicUsize>);

    impl Drop for LateHandle {
        fn drop(&mut self) {
            self.1.store(self.0.in_flight(), Ordering::SeqCst);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bounded_call_past_its_deadline_is_counted_until_its_result_is_dropped() {
        let calls = Arc::new(InFlightCalls::default());
        let counted_at_drop = Arc::new(AtomicUsize::new(usize::MAX));
        let (release, released) = std::sync::mpsc::channel::<()>();

        let outcome = calls
            .run_bounded(Duration::from_millis(50), {
                let (calls, counted_at_drop) = (Arc::clone(&calls), Arc::clone(&counted_at_drop));
                move || {
                    released.recv().ok();
                    LateHandle(calls, counted_at_drop)
                }
            })
            .await;
        assert!(matches!(outcome, Err(BoundedCallError::TimedOut)));
        assert_eq!(calls.in_flight(), 1, "a call still inside the SDK left the count");

        release.send(()).unwrap();
        assert!(calls.wait_drained(Duration::from_secs(2)).await);
        assert_eq!(
            counted_at_drop.load(Ordering::SeqCst),
            1,
            "the late result must be dropped while its call is still counted"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bounded_call_that_returns_in_time_hands_its_result_back() {
        let calls = Arc::new(InFlightCalls::default());
        let outcome = calls.run_bounded(Duration::from_secs(2), || 42).await;
        assert_eq!(outcome, Ok(42));
        assert!(calls.wait_drained(Duration::from_secs(1)).await);
    }

    #[tokio::test]
    async fn waiting_gives_up_at_the_cap_while_a_call_is_stuck() {
        let calls = Arc::new(InFlightCalls::default());
        let _stuck = calls.begin();
        let started = std::time::Instant::now();
        assert!(!calls.wait_drained(Duration::from_millis(50)).await);
        assert!(started.elapsed() >= Duration::from_millis(50));
    }
}
