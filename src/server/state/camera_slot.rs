//! One camera position and everything that is singular about it.
//!
//! Before roles existed these five fields sat directly on `AppState`, which is why a
//! second `connect` could only be implemented by *displacing* the first camera's handle
//! and closing it. Grouping them makes the invariant structural: a slot owns exactly one
//! device, and every consumer addresses a slot rather than "the camera".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Notify, RwLock};

use super::MonitorCmd;
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
    /// The raw-frame directory this slot's session was writing into, parked at
    /// disconnect so a reconnect rejoins it instead of scattering one observation across
    /// timestamped folders, together with the frame number to carry on from. Only the
    /// guide loop uses this; the main camera carries the equivalent on
    /// `SessionResumePlan`.
    pub raw_session: RwLock<Option<RawSessionResume>>,
    /// Hardware calls waiting for the handle's owner to run them. See [`CameraOp`].
    pending_ops: StdMutex<Vec<CameraOp>>,
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

    /// Cancel the exposure in flight on this slot, if any.
    pub async fn cancel_exposure(&self) {
        if let Some(token) = self.cancel_token.read().await.as_ref() {
            token.store(true, Ordering::SeqCst);
        }
    }
}
