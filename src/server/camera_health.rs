//! One fault detector for the whole server.
//!
//! Three places can discover that a camera has stopped answering: the capture
//! loop's frame watchdog, its status-poll watchdog, and the camera-session
//! monitor's cooler poll. They see the same hardware through different code
//! paths, so a fault that alternates between them is still one fault.
//!
//! Everything that decides "this camera is persistently unresponsive" lives
//! here: the threshold, the per-camera streak in
//! `AppState.consecutive_watchdog_timeouts`, and the escalation event. The
//! alternative — a counter per call site — needs each site to independently
//! reach the threshold before any of them escalates, which is how a camera
//! failing every other poll can stay "healthy" indefinitely.

use std::sync::Arc;
use std::time::Duration;

use tracing::{error, warn};

use crate::server::events::ServerEvent;
use crate::server::state::AppState;

/// Consecutive faults against one camera before escalating from an ordinary
/// disconnect to a distinct "persistently unresponsive" signal
/// (`ServerEvent::CameraPersistentlyUnresponsive`).
pub(crate) const PERSISTENT_FAULT_THRESHOLD: u32 = 3;

/// How long a streak survives without new evidence.
///
/// A plain "any success resets to zero" rule loses a fault that alternates
/// between a dead-handle error and an ordinary transient one — each transient
/// wipes the evidence and the threshold is never reached. Ageing the streak out
/// instead means only a genuinely quiet interval clears it.
pub(crate) const FAULT_STREAK_TTL: Duration = Duration::from_secs(60);

/// What kind of evidence a fault report carries. Both kinds count toward the
/// same streak — a hung call and a handle the SDK has invalidated are two
/// symptoms of one dead camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultKind {
    /// A bounded SDK call did not return within its budget. The handle was
    /// abandoned to a detached thread and must not be used again.
    Timeout,
    /// The SDK answered, but said the device is gone — see
    /// `CameraError::is_sdk_disconnected`.
    DeviceLost,
}

impl FaultKind {
    fn describe(self) -> &'static str {
        match self {
            FaultKind::Timeout => "stopped responding",
            FaultKind::DeviceLost => "reported its device is gone",
        }
    }
}

/// Clear a camera's fault streak. Called whenever any SDK call returns within
/// its budget and without a device-lost error, since that proves the camera is
/// currently responding regardless of which call site observed it.
pub(crate) fn clear_fault_streak(state: &Arc<AppState>, camera_name: &str) {
    let mut counts = state
        .consecutive_watchdog_timeouts
        .lock()
        .expect("consecutive_watchdog_timeouts mutex poisoned");
    counts.remove(camera_name);
}

/// Record one fault against `camera_name` and report the resulting streak.
///
/// Escalates with `ServerEvent::CameraPersistentlyUnresponsive` on reaching
/// `PERSISTENT_FAULT_THRESHOLD`, on top of whatever per-incident error the
/// caller already sends.
pub(crate) fn record_fault(state: &Arc<AppState>, camera_name: &str, kind: FaultKind) -> u32 {
    let consecutive = state.bump_fault_streak(camera_name, FAULT_STREAK_TTL);

    warn!(
        camera_name = %camera_name,
        ?kind,
        consecutive,
        "Camera fault recorded"
    );

    if consecutive >= PERSISTENT_FAULT_THRESHOLD {
        error!(
            camera_name = %camera_name,
            consecutive,
            "Camera appears persistently unresponsive"
        );
        let _ = state
            .events
            .send(ServerEvent::camera_persistently_unresponsive(
                camera_name.to_string(),
                consecutive,
            ));
    }

    consecutive
}

/// Whether a streak has reached the point where the handle should be given up
/// on rather than retried.
pub(crate) fn is_persistent(consecutive: u32) -> bool {
    consecutive >= PERSISTENT_FAULT_THRESHOLD
}

/// The user-facing sentence for one fault incident. Written for an observer at
/// a telescope, not for a log reader: it names the camera and says what will
/// happen next.
pub(crate) fn incident_message(camera_name: &str, kind: FaultKind) -> String {
    format!(
        "Camera '{}' {} — disconnecting",
        camera_name,
        kind.describe()
    )
}
