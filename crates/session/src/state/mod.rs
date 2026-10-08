//! The state every session use case and request handler shares — see [`AppState`].

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, warn};

use super::events::ServerEvent;
use super::services::PushToState;
use super::settings_persistence::SettingsPersistence;
use night_amplifier_core::camera::CameraStatus;
use night_amplifier_core::disk_writer::{DiskWriter, DiskWriterConfig, DiskWriterHandle};
use night_amplifier_core::telemetry::metrics as telemetry_metrics;

mod camera_slot;
mod capture_mode;
pub mod focus_mode;
mod frame_stream;
mod guide_loop;
mod roster;
mod stream_viewers;
mod session;
mod settings;
mod settings_store;
mod types;

pub use night_amplifier_core::stacking::{StackingType, StackingTypeInfo, WeightingPreset};
pub use camera_slot::{
    BoundedCallError, CameraOp, CameraSlot, InstallOutcome, RawSessionResume, Recovery,
    SuspendVerdict,
};
pub use capture_mode::{CaptureMode, RawFrameSaving};
pub use focus_mode::FocusModeSnapshot;
pub use frame_stream::FrameStream;
pub use guide_loop::{GuideLoopTicket, GuideLoops};
pub use roster::{CameraRoster, ConnectedCameraInfo};
pub use session::{
    CaptureControl, CaptureResume, FrameCounts, SessionResumePlan, SessionStats, REJECTION_RATE_THRESHOLD,
    REJECTION_RATE_WINDOW,
};
pub use settings::{
    default_preview_resolution, default_streaming_resolution, CameraCaptureProfile,
    CaptureSettings, EyepieceSettings, EyepieceStreamResolution, Resolution,
    DEFAULT_PREVIEW_RESOLUTION, DEFAULT_STREAMING_RESOLUTION,
};
pub use settings_store::SettingsStore;
pub use stream_viewers::{StreamKind, ViewerGuard};
pub use types::{CameraPhase, CameraRole, CaptureState};

/// The application state shared across all handlers.
///
/// Every public field is an aggregate that keeps its locks to itself, or an injected port
/// (`plugins`, `device_catalog`, `events`): no caller can hold one of its locks across an
/// `await`, or change half of what belongs together. Bare locks stay private.
pub struct AppState {
    /// The connected cameras by role, their phases, statuses and fault records, and each
    /// role's slot. Address a slot through [`AppState::slot`].
    pub roster: CameraRoster,
    /// Where the capture session stands, and the cancel switch its loops watch.
    pub capture: CaptureControl,
    /// The session's frame counters, lock-free for the capture threads.
    pub stats: SessionStats,
    /// Capture settings, read as snapshots.
    pub settings: SettingsStore,
    /// The main camera's rendered image stream — what `/ws/stream` serves by default.
    pub main_stream: Arc<FrameStream>,
    /// The guide camera's rendered image stream — `/ws/stream?source=guide`.
    ///
    /// Separate from `main_stream` down to the frame counter: sharing one would make
    /// each camera's frames invalidate the other's payloads. Its client census is
    /// also the guide loop's render gate — see [`FrameStream::has_viewers`].
    pub guide_stream: Arc<FrameStream>,
    /// Event broadcast channel
    pub events: broadcast::Sender<ServerEvent>,
    /// Disk writer handle for saving frames
    pub disk_writer: DiskWriterHandle,
    /// Push-To's solve bookkeeping; `None` for a server without it. Synchronized inside,
    /// so the capture threads read it without a lock.
    pub push_to: Option<PushToState>,
    /// Push-To's solve and watch consumer threads, started by the first frame offered.
    pub(crate) push_to_tasks: std::sync::OnceLock<crate::capture::push_to_tasks::PushToTasks>,
    /// The guide camera's free-running loop: whether one is registered, whether it is
    /// exposing, and its stop switch. "Exposing" decides the plate-solve source with an
    /// atomic load on the stacking thread — see `capture::solving::SolveSource`.
    ///
    /// Deliberately not "a guide camera is connected": a cooled guide camera stays
    /// registered through its whole warm-up with its loop already stopped, and
    /// `connect` can return before a loop that then fails to start — either way,
    /// presence once answered "solving" when nothing was. Only `guide_task` (un)registers loops.
    pub guide_loops: guide_loop::GuideLoops,
    /// Settings persistence manager
    settings_persistence: SettingsPersistence,
    /// Serializes `camera::lifecycle::connect`. Its idempotency check
    /// reads the roster, which `finalize_disconnect` clears first, so two
    /// concurrent connects for one id would both pass it, both open the
    /// device, and the second would displace — and so close — the first.
    pub(crate) camera_connect_lock: Mutex<()>,
    /// Where cameras are discovered and opened. A trait object so tests can script
    /// the USB bus — including one that reorders itself between two enumerations.
    pub device_catalog: Arc<dyn night_amplifier_core::camera::DeviceCatalog>,
    /// The Pro plugins this server runs: the process's installed set, or a test's own.
    /// Everything the server drives — stacking, rendering, Push-To — takes it from here.
    pub plugins: night_amplifier_core::plugins::Plugins,
    /// One counter per provider for `CameraService`'s bounded enumerations: a refresh waits on
    /// a provider still inside its SDK instead of starting a second call behind it.
    discovery_calls: StdMutex<HashMap<String, Arc<camera_slot::InFlightCalls>>>,
    /// What an interrupted capture needs in order to pick up where it left off: the
    /// plan recorded when it started, and the stack it parked. Consumed by the reconnect
    /// supervisor, cleared on a clean stop or a fresh start.
    pub resume: CaptureResume,
}

/// Commands accepted by the camera monitor thread. Defined here (not in
/// `camera`) so `AppState` can hold the sender without a cyclic
/// module dependency.
#[derive(Debug, Clone)]
pub enum MonitorCmd {
    /// Camera is about to be handed off to the capture thread. Monitor
    /// should pause its polling loop.
    HandOffToCapture,
    /// Camera handle has been returned. Monitor should resume polling.
    ResumeAfterCapture,
    /// Begin the warmup sequence. When `fast` is true the cooler is
    /// disabled immediately and the sensor rises naturally (old behavior).
    /// Otherwise the monitor keeps the cooler on and raises the commanded
    /// setpoint toward `WARMUP_RAMP_TARGET_C` at `RAMP_RATE_C_PER_MIN`. In
    /// both cases the handle closes once the sensor reaches
    /// `WARMUP_THRESHOLD_C` and duty is ≤ 5 %.
    StartWarmup { fast: bool },
    /// Cancel an in-progress warmup (user started capture during warmup).
    CancelWarmup,
    /// Install or update the cooldown target. When `fast` is true the final
    /// target is pushed to hardware immediately and no ramp is installed.
    /// Otherwise the monitor re-seeds its cooldown ramp from the latest
    /// sensor temperature and advances toward `target` at
    /// `RAMP_RATE_C_PER_MIN`. `enabled = false` clears any active ramp.
    UpdateCoolerTarget {
        enabled: bool,
        target: Option<f64>,
        fast: bool,
    },
    /// Stop polling and close the handle immediately.
    Shutdown,
}

impl AppState {
    /// Create new application state
    pub fn new() -> (Self, DiskWriter) {
        Self::with_disk_writer_config(DiskWriterConfig::default())
    }

    /// Create new application state with custom disk writer configuration
    pub fn with_disk_writer_config(disk_config: DiskWriterConfig) -> (Self, DiskWriter) {
        let settings_persistence = SettingsPersistence::default();
        let settings = settings_persistence.load().unwrap_or_default();
        Self::build(
            disk_config,
            settings_persistence,
            settings,
            Some(PushToState::default()),
        )
    }

    /// Assemble the state. Every constructor funnels through here so a new
    /// field only has to be initialized once.
    fn build(
        disk_config: DiskWriterConfig,
        settings_persistence: SettingsPersistence,
        settings: CaptureSettings,
        push_to: Option<PushToState>,
    ) -> (Self, DiskWriter) {
        let (events_tx, _) = broadcast::channel(256);
        let (disk_writer, disk_writer_handle) = DiskWriter::new(disk_config);

        let state = Self {
            roster: CameraRoster::default(),
            capture: CaptureControl::default(),
            stats: SessionStats::default(),
            settings: SettingsStore::new(settings),
            main_stream: Arc::new(FrameStream::default()),
            guide_stream: Arc::new(FrameStream::default()),
            events: events_tx,
            disk_writer: disk_writer_handle,
            push_to,
            push_to_tasks: std::sync::OnceLock::new(),
            guide_loops: guide_loop::GuideLoops::default(),
            settings_persistence,
            camera_connect_lock: Mutex::new(()),
            device_catalog: Arc::new(night_amplifier_core::camera::RegistryCatalog::new()),
            plugins: night_amplifier_core::plugins::Plugins::installed(),
            discovery_calls: StdMutex::new(HashMap::new()),
            resume: CaptureResume::default(),
        };

        (state, disk_writer)
    }

    /// The slot owning `role`'s handle, monitor and reconnect guard.
    pub fn slot(&self, role: CameraRole) -> &CameraSlot {
        self.roster.slot(role)
    }

    /// The in-flight counter for `provider`'s camera discovery, created on first use.
    pub fn discovery_calls_for(&self, provider: &str) -> Arc<camera_slot::InFlightCalls> {
        let mut calls = self.discovery_calls.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(calls.entry(provider.to_string()).or_default())
    }

    /// The rendered image stream `role`'s camera produces.
    pub fn stream(&self, role: CameraRole) -> &Arc<FrameStream> {
        match role {
            CameraRole::Main => &self.main_stream,
            CameraRole::Guide => &self.guide_stream,
        }
    }

    /// The camera currently occupying `role`, if any.
    pub fn camera_in_role(&self, role: CameraRole) -> Option<ConnectedCameraInfo> {
        self.roster.in_role(role)
    }

    /// Which role a connected camera holds, by id.
    pub fn role_of(&self, camera_id: &str) -> Option<CameraRole> {
        self.roster.get(camera_id).map(|info| info.role)
    }

    /// The display name of a connected camera, by id.
    ///
    /// Exists so error messages can name the camera the way the user does — "Ares-C
    /// Pro", or the fixture directory a simulator was pointed at — rather than the
    /// wire id. `simulator_0` is not something anyone chose or can recognise, and an
    /// id in a message is a message the reader has to translate before it helps.
    pub fn connected_camera_name(&self, camera_id: &str) -> Option<String> {
        self.roster.get(camera_id).map(|info| info.info.name)
    }

    /// Whether the guide loop is exposing. An atomic load, so the stacking thread can
    /// ask it per frame.
    pub fn guide_loop_running(&self) -> bool {
        self.guide_loops.is_running()
    }

    /// Whether the guide camera owns plate solving: its loop is running, or it is being
    /// reopened after a fault. Recovery keeps the solver pointed at the guide scope's
    /// optics, so the imaging camera offering frames meanwhile would be judged against
    /// the wrong focal length.
    pub fn guide_holds_solving(&self) -> bool {
        self.guide_loop_running() || self.slot(CameraRole::Guide).is_recovering()
    }

    /// Mark a guide loop running or not without one behind it — for tests that simulate
    /// the loop. `guide_task` goes through [`Self::guide_loops`].
    pub fn set_guide_loop_running(&self, running: bool) {
        self.guide_loops.force_running(running);
    }

    /// Save current settings to disk
    pub fn save_settings(&self) {
        if let Err(e) = self.settings_persistence.save(&self.settings.snapshot()) {
            warn!("Failed to save settings: {}", e);
        }
    }

    /// Create new application state for testing
    ///
    /// Captures go to a directory of their own under the system temp dir, not to the
    /// `./captures` the default config points at — tests that open a capture session
    /// really do create the folder, and against the default they accumulated hundreds of
    /// dated directories in whatever tree the suite was run from. Cleanup is left to the
    /// OS: the paths outlive the call, and nothing here can say when a test is done with
    /// one.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new_for_testing() -> (Self, DiskWriter) {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let captures_dir = std::env::temp_dir().join(format!(
            "night_amplifier_test_captures/{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        let (mut state, disk_writer) = Self::build(
            DiskWriterConfig::new(captures_dir),
            SettingsPersistence::new("/nonexistent/test/settings.json"),
            CaptureSettings::default(),
            None,
        );
        // Discovery would otherwise call every vendor SDK installed on the machine, and open
        // its cameras, from tests running in parallel.
        state.device_catalog = Arc::new(night_amplifier_core::camera::RegistryCatalog::simulator_only());
        (state, disk_writer)
    }

    /// Get the current capture state
    pub fn capture_state(&self) -> CaptureState {
        self.capture.state()
    }

    /// Update capture state and broadcast event
    pub fn set_capture_state(&self, state: CaptureState) {
        self.capture.set(state);
        let _ = self.events.send(ServerEvent::state_changed(state));
    }

    /// [`CaptureControl::transition`], broadcasting the state it moved to.
    pub fn transition_capture_state(
        &self,
        next: impl FnOnce(CaptureState) -> Option<CaptureState>,
    ) -> Result<CaptureState, CaptureState> {
        let moved = self.capture.transition(next)?;
        let _ = self.events.send(ServerEvent::state_changed(moved));
        Ok(moved)
    }

    /// End a capture paused for recovery, dropping what its resume would have needed.
    /// Returns whether one was paused.
    ///
    /// One compare-and-set on the session state, the same one `resume_capture` makes
    /// the other way: whichever lands first wins, so a resume and a Disconnect or give-up
    /// can no longer both act on one pause.
    pub fn end_paused_capture(&self) -> bool {
        let paused = |current| (current == CaptureState::Recovering).then_some(CaptureState::Idle);
        if self.capture.transition(paused).is_err() {
            return false;
        }
        self.resume.clear();
        let _ = self.events.send(ServerEvent::state_changed(CaptureState::Idle));
        night_amplifier_core::render::denoise::ai::start_benchmark(&self.plugins);
        true
    }

    /// End a capture pipeline: `Idle`, unless it has been paused for recovery.
    ///
    /// A pipeline ended by a device fault is already suspended, and marking it `Idle`
    /// would tell the UI the session was over — same for one that never got going
    /// because its camera failed again between reopen and resume (slot recovering,
    /// plan still there); ending that would discard the plan recovery needs. Any other
    /// end is final, so the resume plan and parked stack go with it: left behind, the
    /// next quiet recovery of the idle camera restarted a capture the observer saw end.
    pub fn end_capture_state(&self) {
        let recovering = self.slot(CameraRole::Main).is_recovering() && self.resume.has_plan();
        let ended = self.capture.transition(|current| match current {
            CaptureState::Recovering => None,
            // `Stopping` is the observer's Stop, which a recovery must not undo.
            _ if recovering && current != CaptureState::Stopping => Some(CaptureState::Recovering),
            _ => Some(CaptureState::Idle),
        });
        match ended {
            Err(_) => return,
            Ok(CaptureState::Recovering) => {
                let _ = self
                    .events
                    .send(ServerEvent::state_changed(CaptureState::Recovering));
                return;
            }
            Ok(_) => {}
        }
        self.resume.clear();
        let _ = self.events.send(ServerEvent::state_changed(CaptureState::Idle));
        // A licence activated mid-session left the AI compute benchmark to here, when every
        // capture thread has been joined. Idempotent, so any other end does nothing.
        night_amplifier_core::render::denoise::ai::start_benchmark(&self.plugins);
    }

    /// Count a frame the stack decided on and broadcast it. `stacking` is the frame's
    /// own setting: an unstacked frame counts as rejected only while stacking.
    ///
    /// `rejection_reason` says why the frame did not join the stack and is only
    /// meaningful when `stacked` is false. It rides on the `frame_captured`
    /// event rather than going through [`AppState::frame_rejected`], which is
    /// for a camera that failed to deliver a frame and feeds the capture-abort
    /// burst detector — a frame that arrived fine and merely aligned badly must
    /// never reach that.
    pub fn frame_captured(&self, stacked: bool, stacking: bool, rejection_reason: Option<&str>) {
        debug_assert!(
            !(stacked && rejection_reason.is_some()),
            "a stacked frame has no rejection reason"
        );
        let counts = self.stats.frame_captured(stacked, stacking);
        let _ = self.events.send(ServerEvent::frame_captured(
            counts.frames,
            counts.stacked,
            counts.rejected,
            rejection_reason,
        ));
    }

    /// Count a frame the camera failed to deliver and broadcast it — see
    /// [`SessionStats::frame_failed`] for how this feeds the current-failure-burst
    /// detection `should_stop_on_errors` uses.
    pub fn frame_rejected(&self, stacking: bool, reason: String) {
        let counts = self.stats.frame_failed(stacking, std::time::Instant::now());
        let _ = self.events.send(ServerEvent::frame_rejected(
            counts.frames,
            counts.stacked,
            counts.rejected,
            reason,
        ));
    }

    /// Subscribe to events
    pub fn subscribe_events(&self) -> broadcast::Receiver<ServerEvent> {
        let receiver = self.events.subscribe();
        telemetry_metrics::record_event_subscribers(self.events.receiver_count() as u64);
        receiver
    }

    /// Send an error event
    pub fn send_error(&self, message: String) {
        let _ = self.events.send(ServerEvent::error(message));
    }

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool {
        self.capture.is_cancelled()
    }

    /// Request cancellation
    pub fn request_cancel(&self) {
        self.capture.request_cancel();
    }

    /// Reset cancellation flag
    pub fn reset_cancel(&self) {
        self.capture.reset_cancel();
    }

    /// Reset the counters for a new capture, and stamp its start.
    pub fn reset_session(&self) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        self.stats.start(now_ms);
    }

    /// Reset frame counters without resetting session start time
    pub fn reset_counters(&self) {
        self.stats.reset_counters();
    }

    /// Set a slot's camera cancel token
    pub async fn set_camera_token(&self, role: CameraRole, token: Arc<AtomicBool>) {
        *self.slot(role).cancel_token.write().await = Some(token);
    }

    /// Clear a slot's camera cancel token
    pub async fn clear_camera_token(&self, role: CameraRole) {
        *self.slot(role).cancel_token.write().await = None;
    }

    /// Cut short the exposure in flight on `role`'s camera.
    ///
    /// Used when that camera's settings change: the running exposure was configured
    /// with the old values, and finishing it only delays the new ones. Scoped to the
    /// one slot — cancelling both would throw away a 5-minute imaging sub because
    /// somebody nudged the guide camera's gain. A slot with no camera is a no-op.
    pub async fn cancel_active_exposure(&self, role: CameraRole) {
        self.slot(role).cancel_exposure().await;
    }

    /// Cache the latest camera status sample and broadcast a status event.
    pub fn update_camera_status(
        &self,
        camera_name: &str,
        status: CameraStatus,
        target_temp_c: Option<f64>,
    ) {
        // Dropped rather than shown: the UI's temperature, the precool check in
        // `return_from_capture` and every ramp seeded from this cache would believe it.
        if !status.has_plausible_temperature() {
            debug!(camera_name, temperature_c = status.temperature_c, "Ignoring an implausible sensor temperature");
            return;
        }
        self.roster.record_status(camera_name, status.clone());
        let _ = self.events.send(ServerEvent::camera_status_updated(
            camera_name,
            status.temperature_c,
            status.cooler_power,
            status.cooler_on,
            status.dew_heater_on,
            target_temp_c,
        ));
    }

    /// Get the latest cached camera status for the given camera name.
    pub fn get_camera_status(&self, camera_name: &str) -> Option<CameraStatus> {
        self.roster.status(camera_name)
    }

    /// Set the lifecycle phase of `role`'s camera and broadcast `CameraPhaseChanged`,
    /// which names the camera for the UI.
    pub fn set_camera_phase(&self, role: CameraRole, camera_name: &str, phase: CameraPhase) {
        self.roster.set_phase(role, phase);
        self.announce_camera_phase(role, camera_name, phase);
    }

    /// [`Self::set_camera_phase`] only if `role` is still in `from` — see
    /// [`CameraRoster::transition`]. Returns whether it moved.
    pub fn transition_camera_phase(
        &self,
        role: CameraRole,
        camera_name: &str,
        from: CameraPhase,
        to: CameraPhase,
    ) -> bool {
        let moved = self.roster.transition(role, from, to);
        if moved {
            self.announce_camera_phase(role, camera_name, to);
        }
        moved
    }

    /// Broadcast `CameraPhaseChanged` for a phase the roster already holds.
    pub(crate) fn announce_camera_phase(&self, role: CameraRole, camera_name: &str, phase: CameraPhase) {
        // Every client's countdown, not only the one whose Disconnect started the warm-up.
        let warmup_remaining = match phase {
            CameraPhase::WarmingUp => self.slot(role).warmup_remaining(),
            _ => None,
        };
        let _ = self.events.send(ServerEvent::camera_phase_changed(
            camera_name,
            role,
            phase,
            warmup_remaining,
        ));
    }

    /// The lifecycle phase of `role`'s camera; `Disconnected` when the slot is empty.
    pub fn camera_phase(&self, role: CameraRole) -> CameraPhase {
        self.roster.phase(role)
    }

    /// Every connected camera's phase, as the event that replaces a client's copy.
    pub fn camera_phases_event(&self) -> ServerEvent {
        let cameras = self
            .roster
            .connected_with_phases()
            .into_iter()
            .map(|(camera, phase)| super::events::CameraPhaseEntry {
                name: camera.info.name,
                role: camera.role,
                phase: phase.into(),
                warmup_remaining_s: self
                    .slot(camera.role)
                    .warmup_remaining()
                    .map(|left| left.as_secs()),
            })
            .collect();
        ServerEvent::CameraPhases { cameras }
    }

    /// Update the cached "plugin holds a target" flag. No-op without Push-To.
    ///
    /// A cache, not the source of truth — see [`PushToState`]. Written by the
    /// target mutations in `PushToService` and re-synced from `solve_frame`,
    /// so that the stacking thread can gate plate solving synchronously.
    pub fn set_push_to_has_target(&self, has_target: bool) {
        if let Some(pt) = &self.push_to {
            pt.set_has_target(has_target);
        }
    }

    /// Record that the target changed: update the cached flag and drop the
    /// de-duplication record for the push direction.
    ///
    /// The direction is only re-broadcast when its numbers change, so without this a
    /// new target whose arrow happens to point the same way would leave the client
    /// showing a distance and heading computed for the *old* target.
    pub fn push_to_target_changed(&self, has_target: bool) {
        if let Some(pt) = &self.push_to {
            pt.forget_direction();
            pt.set_has_target(has_target);
        }
    }

    /// Record a frame the camera handed the pipeline, dropped or not.
    ///
    /// Counted at the point of hand-off rather than in the stacking task, because a
    /// frame that never reached a channel is exactly the one the rate has to account
    /// for.
    pub fn frame_delivered(&self) -> u64 {
        self.stats.frame_delivered()
    }

    /// Record a dropped frame (pipeline back-pressure) and broadcast event
    pub fn frame_dropped(&self) -> u64 {
        telemetry_metrics::record_frame_dropped();
        let count = self.stats.frame_dropped();
        let _ = self
            .events
            .send(ServerEvent::frame_dropped(count, self.stats.delivered()));
        count
    }
}

#[cfg(test)]
mod tests;
