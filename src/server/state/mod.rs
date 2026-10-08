//! Application state management for the web server
//!
//! This module contains the shared state that is accessed by all request handlers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{debug, warn};

use super::events::ServerEvent;
use super::services::PushToState;
use super::settings_persistence::SettingsPersistence;
use crate::camera::CameraStatus;
use crate::disk_writer::{DiskWriter, DiskWriterConfig, DiskWriterHandle};
use crate::telemetry::metrics as telemetry_metrics;

mod camera_slot;
mod capture_mode;
pub mod focus_mode;
mod frame_stream;
mod guide_loop;
mod stream_viewers;
mod session;
mod settings;
mod settings_store;
mod types;

pub use crate::stacking::{StackingType, StackingTypeInfo, WeightingPreset};
pub use camera_slot::{
    BoundedCallError, CameraOp, CameraSlot, InstallOutcome, RawSessionResume, Recovery,
    SuspendVerdict,
};
pub use capture_mode::{CaptureMode, RawFrameSaving};
pub use focus_mode::FocusModeSnapshot;
pub use frame_stream::FrameStream;
pub use guide_loop::{GuideLoopTicket, GuideLoops};
pub use session::{
    ConnectedCameraInfo, FrameCounts, SessionResumePlan, SessionStats, REJECTION_RATE_THRESHOLD,
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

/// The main application state shared across all handlers
pub struct AppState {
    /// Currently connected cameras info (camera_id -> info)
    pub cameras: RwLock<HashMap<String, ConnectedCameraInfo>>,
    /// Currently selected camera ID
    pub selected_camera: RwLock<Option<String>>,
    /// Where the capture session stands. Transitions are compare-and-set under this lock.
    pub capture: RwLock<CaptureState>,
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
    /// Cancellation flag for capture loop
    pub cancel_flag: AtomicBool,
    /// Event broadcast channel
    pub events: broadcast::Sender<ServerEvent>,
    /// Disk writer handle for saving frames
    pub disk_writer: DiskWriterHandle,
    /// Push-To navigation state
    pub push_to: RwLock<Option<PushToState>>,
    /// Push-To's solve and watch consumer threads, started by the first frame offered.
    pub(crate) push_to_tasks: std::sync::OnceLock<crate::server::capture::push_to_tasks::PushToTasks>,
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
    pub settings_persistence: SettingsPersistence,
    /// Latest reported camera status keyed by camera name (for cooled cameras)
    pub latest_camera_status: RwLock<HashMap<String, CameraStatus>>,
    /// One slot per [`CameraRole`], each owning that position's handle, monitor,
    /// cancel token and reconnect guard. Address it through [`AppState::slot`].
    pub camera_slots: [CameraSlot; CameraRole::COUNT],
    /// Serializes `camera_session::lifecycle::connect`. Its idempotency check
    /// reads `cameras`, which `finalize_disconnect` clears first, so two
    /// concurrent connects for one id would both pass it, both open the
    /// device, and the second would displace — and so close — the first.
    pub camera_connect_lock: Mutex<()>,
    /// Where cameras are discovered and opened. A trait object so tests can script
    /// the USB bus — including one that reorders itself between two enumerations.
    pub device_catalog: Arc<dyn crate::camera::DeviceCatalog>,
    /// The Pro plugins this server runs: the process's installed set, or a test's own.
    /// Everything the server drives — stacking, rendering, Push-To — takes it from here.
    pub plugins: crate::plugins::Plugins,
    /// One counter per provider for `CameraService`'s bounded enumerations: a refresh waits on
    /// a provider still inside its SDK instead of starting a second call behind it.
    pub discovery_calls: StdMutex<HashMap<String, Arc<camera_slot::InFlightCalls>>>,
    /// What an interrupted capture needs in order to pick up where it left
    /// off. Recorded when a capture starts, consumed by the reconnect
    /// supervisor, cleared on a clean stop. Main camera only — for the guide
    /// camera, reconnecting *is* resuming, since its loop is started by `connect`.
    pub session_resume_plan: RwLock<Option<SessionResumePlan>>,
    /// Stacking state parked by a capture that ended unexpectedly, so a
    /// resumed capture continues the same integration instead of restarting
    /// it. Cleared whenever a capture starts fresh or stops cleanly — holding
    /// full-resolution accumulators between sessions would be pure waste.
    pub stacking_carryover: StdMutex<Option<crate::server::capture::StackingCarryover>>,
    /// Consecutive camera faults keyed by role and camera name, with the instant the
    /// streak was last extended. The role keeps two bodies of one model apart: a guide
    /// twin's stall must not skip the imaging twin's warm-up. Every fault detector — the capture watchdog,
    /// the status-poll watchdog and the monitor's cooler poll — feeds this one
    /// counter, so evidence from any of them counts toward the same
    /// escalation. Cleared by a call that succeeds, and aged out after
    /// `camera_health::FAULT_STREAK_TTL` so an alternating fault cannot hide
    /// behind the occasional success. See `camera_health`.
    pub consecutive_watchdog_timeouts: StdMutex<HashMap<(CameraRole, String), (u32, Instant)>>,
    /// Whether restarting a stalled stream in place has lately worked, per role and camera
    /// name. Outlives the capture loop, whose next reopen it decides on. See
    /// `camera_health::RestartHistory`.
    pub(crate) restart_histories:
        StdMutex<HashMap<(CameraRole, String), crate::server::camera_health::RestartHistory>>,
}

/// Commands accepted by the camera monitor thread. Defined here (not in
/// `camera_session`) so `AppState` can hold the sender without a cyclic
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
            cameras: RwLock::new(HashMap::new()),
            selected_camera: RwLock::new(None),
            capture: RwLock::new(CaptureState::Idle),
            stats: SessionStats::default(),
            settings: SettingsStore::new(settings),
            main_stream: Arc::new(FrameStream::default()),
            guide_stream: Arc::new(FrameStream::default()),
            cancel_flag: AtomicBool::new(false),
            events: events_tx,
            disk_writer: disk_writer_handle,
            push_to: RwLock::new(push_to),
            push_to_tasks: std::sync::OnceLock::new(),
            guide_loops: guide_loop::GuideLoops::default(),
            settings_persistence,
            latest_camera_status: RwLock::new(HashMap::new()),
            camera_slots: std::array::from_fn(|_| CameraSlot::default()),
            camera_connect_lock: Mutex::new(()),
            device_catalog: Arc::new(crate::camera::RegistryCatalog::new()),
            plugins: crate::plugins::Plugins::installed(),
            discovery_calls: StdMutex::new(HashMap::new()),
            session_resume_plan: RwLock::new(None),
            stacking_carryover: StdMutex::new(None),
            consecutive_watchdog_timeouts: StdMutex::new(HashMap::new()),
            restart_histories: StdMutex::new(HashMap::new()),
        };

        (state, disk_writer)
    }

    /// The slot owning `role`'s handle, monitor and reconnect guard.
    pub fn slot(&self, role: CameraRole) -> &CameraSlot {
        &self.camera_slots[role as usize]
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
    pub async fn camera_in_role(&self, role: CameraRole) -> Option<ConnectedCameraInfo> {
        self.cameras
            .read()
            .await
            .values()
            .find(|info| info.role == role)
            .cloned()
    }

    /// Which role a connected camera holds, by id.
    pub async fn role_of(&self, camera_id: &str) -> Option<CameraRole> {
        self.cameras.read().await.get(camera_id).map(|info| info.role)
    }

    /// The display name of a connected camera, by id.
    ///
    /// Exists so error messages can name the camera the way the user does — "Ares-C
    /// Pro", or the fixture directory a simulator was pointed at — rather than the
    /// wire id. `simulator_0` is not something anyone chose or can recognise, and an
    /// id in a message is a message the reader has to translate before it helps.
    pub async fn connected_camera_name(&self, camera_id: &str) -> Option<String> {
        self.cameras
            .read()
            .await
            .get(camera_id)
            .map(|info| info.info.name.clone())
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
        use std::sync::atomic::AtomicU64;

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
        state.device_catalog = Arc::new(crate::camera::RegistryCatalog::simulator_only());
        (state, disk_writer)
    }

    /// Get the current capture state
    pub async fn capture_state(&self) -> CaptureState {
        *self.capture.read().await
    }

    /// Update capture state and broadcast event
    pub async fn set_capture_state(&self, state: CaptureState) {
        *self.capture.write().await = state;
        let _ = self.events.send(ServerEvent::state_changed(state));
    }

    /// End a capture paused for recovery, dropping what its resume would have needed.
    /// Returns whether one was paused.
    ///
    /// One compare-and-set on the session state, the same one `resume_capture` makes
    /// the other way: whichever lands first wins, so a resume and a Disconnect or give-up
    /// can no longer both act on one pause.
    pub async fn end_paused_capture(&self) -> bool {
        {
            let mut capture = self.capture.write().await;
            if *capture != CaptureState::Recovering {
                return false;
            }
            *capture = CaptureState::Idle;
        }
        *self.session_resume_plan.write().await = None;
        self.clear_stacking_carryover();
        let _ = self.events.send(ServerEvent::state_changed(CaptureState::Idle));
        crate::render::denoise::ai::start_benchmark(&self.plugins);
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
    pub async fn end_capture_state(&self) {
        let recovering = self.slot(CameraRole::Main).is_recovering()
            && self.session_resume_plan.read().await.is_some();
        {
            let mut capture = self.capture.write().await;
            if *capture == CaptureState::Recovering {
                return;
            }
            // `Stopping` is the observer's Stop, which a recovery must not undo.
            if recovering && *capture != CaptureState::Stopping {
                *capture = CaptureState::Recovering;
                drop(capture);
                let _ = self
                    .events
                    .send(ServerEvent::state_changed(CaptureState::Recovering));
                return;
            }
            *capture = CaptureState::Idle;
        }
        *self.session_resume_plan.write().await = None;
        self.clear_stacking_carryover();
        let _ = self.events.send(ServerEvent::state_changed(CaptureState::Idle));
        // A licence activated mid-session left the AI compute benchmark to here, when every
        // capture thread has been joined. Idempotent, so any other end does nothing.
        crate::render::denoise::ai::start_benchmark(&self.plugins);
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

    /// Discard any stacking accumulators parked for a resume.
    pub fn clear_stacking_carryover(&self) {
        *self
            .stacking_carryover
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Extend a camera's fault streak and return its new length. A streak
    /// older than `ttl` has expired and restarts at 1.
    pub fn bump_fault_streak(&self, role: CameraRole, camera_name: &str, ttl: Duration) -> u32 {
        let now = Instant::now();
        let mut counts = self
            .consecutive_watchdog_timeouts
            .lock()
            .expect("consecutive_watchdog_timeouts mutex poisoned");
        let entry = counts.entry((role, camera_name.to_string())).or_insert((0, now));
        if now.duration_since(entry.1) > ttl {
            entry.0 = 0;
        }
        entry.0 += 1;
        entry.1 = now;
        entry.0
    }

    /// Send an error event
    pub fn send_error(&self, message: String) {
        let _ = self.events.send(ServerEvent::error(message));
    }

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }

    /// Request cancellation
    pub fn request_cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    /// Reset cancellation flag
    pub fn reset_cancel(&self) {
        self.cancel_flag.store(false, Ordering::SeqCst);
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
    pub async fn update_camera_status(
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
        {
            let mut map = self.latest_camera_status.write().await;
            map.insert(camera_name.to_string(), status.clone());
        }
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
    pub async fn get_camera_status(&self, camera_name: &str) -> Option<CameraStatus> {
        self.latest_camera_status
            .read()
            .await
            .get(camera_name)
            .cloned()
    }

    /// Set the lifecycle phase of `role`'s camera and broadcast `CameraPhaseChanged`,
    /// which names the camera for the UI.
    pub async fn set_camera_phase(&self, role: CameraRole, camera_name: &str, phase: CameraPhase) {
        *self.slot(role).phase.write().await = phase;
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
    pub async fn camera_phase(&self, role: CameraRole) -> CameraPhase {
        *self.slot(role).phase.read().await
    }

    /// Every connected camera's phase, as the event that replaces a client's copy.
    pub async fn camera_phases_event(&self) -> ServerEvent {
        let connected: Vec<ConnectedCameraInfo> =
            self.cameras.read().await.values().cloned().collect();
        let mut cameras = Vec::with_capacity(connected.len());
        for camera in connected {
            let slot = self.slot(camera.role);
            cameras.push(super::events::CameraPhaseEntry {
                name: camera.info.name,
                role: camera.role,
                phase: (*slot.phase.read().await).into(),
                warmup_remaining_s: slot.warmup_remaining().map(|left| left.as_secs()),
            });
        }
        ServerEvent::CameraPhases { cameras }
    }

    /// Update the cached "plugin holds a target" flag. No-op without Push-To.
    ///
    /// A cache, not the source of truth — see [`PushToState`]. Written by the
    /// target mutations in `PushToService` and re-synced from `solve_frame`,
    /// so that the stacking thread can gate plate solving synchronously.
    pub async fn set_push_to_has_target(&self, has_target: bool) {
        if let Some(ref mut pt) = *self.push_to.write().await {
            pt.has_target = has_target;
        }
    }

    /// Record that the target changed: update the cached flag and drop the
    /// de-duplication record for the push direction.
    ///
    /// The direction is only re-broadcast when its numbers change, so without this a
    /// new target whose arrow happens to point the same way would leave the client
    /// showing a distance and heading computed for the *old* target.
    pub async fn push_to_target_changed(&self, has_target: bool) {
        if let Some(ref mut pt) = *self.push_to.write().await {
            pt.has_target = has_target;
            pt.forget_direction();
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
mod tests {
    use super::*;

    #[test]
    fn test_capture_state_default() {
        assert_eq!(CaptureState::default(), CaptureState::Idle);
    }

    #[test]
    fn test_capture_settings_default() {
        let settings = CaptureSettings::default();
        assert_eq!(settings.exposure_us, 1_000_000);
        assert_eq!(settings.gain, 0);
        assert!(settings.auto_stretch);
        assert!(settings.stacking);
    }

    #[test]
    fn test_capture_settings_to_config() {
        let settings = CaptureSettings {
            exposure_us: 2_000_000,
            gain: 100,
            offset: 20,
            bin: 2,
            planetary_roi: None,
            ..Default::default()
        };

        let config = settings.to_capture_config();
        assert_eq!(config.exposure_us, 2_000_000);
        assert_eq!(config.gain, 100);
        assert_eq!(config.offset, 20);
        assert_eq!(config.bin, 2);
    }

    #[tokio::test]
    async fn test_app_state_creation() {
        let (state, _disk_writer) = AppState::new_for_testing();
        assert_eq!(state.capture_state().await, CaptureState::Idle);
        assert!(!state.is_cancelled());
    }

    #[tokio::test]
    async fn test_app_state_capture_state() {
        let (state, _disk_writer) = AppState::new_for_testing();

        state.set_capture_state(CaptureState::Capturing).await;
        assert_eq!(state.capture_state().await, CaptureState::Capturing);

        state.set_capture_state(CaptureState::Idle).await;
        assert_eq!(state.capture_state().await, CaptureState::Idle);
    }

    #[test]
    fn test_app_state_frame_tracking() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.reset_session();

        state.frame_captured(true, true, None);
        state.frame_captured(true, true, None);
        state.frame_captured(false, true, None);

        let counts = state.stats.counts();
        assert_eq!(counts.frames, 3);
        assert_eq!(counts.stacked, 2);
        assert!(state.stats.started_at().is_some());
    }

    #[tokio::test]
    async fn test_app_state_cancellation() {
        let (state, _disk_writer) = AppState::new_for_testing();

        assert!(!state.is_cancelled());
        state.request_cancel();
        assert!(state.is_cancelled());
        state.reset_cancel();
        assert!(!state.is_cancelled());
    }

    /// The two streams are independent down to the counter. Sharing one would make each
    /// camera's frames invalidate the other's payloads on every exposure.
    #[tokio::test]
    async fn guide_and_main_streams_do_not_share_a_counter_or_payloads() {
        let (state, _disk_writer) = AppState::new_for_testing();

        let main_counter = state.main_stream.begin_frame();
        state.main_stream.set_payload(StreamKind::Jpeg, main_counter, vec![1]);

        // Three guide frames must leave the main stream's payload readable.
        for _ in 0..3 {
            let guide_counter = state.guide_stream.begin_frame();
            state.guide_stream.set_payload(StreamKind::Jpeg, guide_counter, vec![2]);
        }

        assert_eq!(state.main_stream.frame_counter(), 1);
        assert_eq!(state.guide_stream.frame_counter(), 3);
        assert_eq!(
            state.main_stream.payload(StreamKind::Jpeg, main_counter).unwrap().as_ref(),
            &[1]
        );
    }

    #[tokio::test]
    async fn test_capture_settings_to_config_forwards_cooling() {
        let settings = CaptureSettings {
            cooler_enabled: true,
            target_temp_c: Some(-10.0),
            ..Default::default()
        };

        let config = settings.to_capture_config();
        assert!(config.cooler_enabled);
        assert_eq!(config.target_temp_c, Some(-10.0));
    }

    #[tokio::test]
    async fn test_capture_settings_to_config_cooler_off_keeps_target_none() {
        let settings = CaptureSettings {
            cooler_enabled: false,
            target_temp_c: None,
            ..Default::default()
        };

        let config = settings.to_capture_config();
        assert!(!config.cooler_enabled);
        assert_eq!(config.target_temp_c, None);
    }

    #[tokio::test]
    async fn test_update_camera_status_caches_and_broadcasts() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let mut subscriber = state.subscribe_events();

        let status = CameraStatus {
            temperature_c: -5.0,
            cooler_power: Some(60.0),
            cooler_on: true,
            is_exposing: false,
            current_gain: 100,
            current_offset: 10,
            current_exposure_us: 1_000_000,
            dew_heater_on: false,
        };

        state
            .update_camera_status("Test Cam", status.clone(), Some(-10.0))
            .await;

        let cached = state.get_camera_status("Test Cam").await.unwrap();
        assert_eq!(cached.temperature_c, -5.0);
        assert_eq!(cached.cooler_power, Some(60.0));
        assert!(cached.cooler_on);

        // The broadcast should have produced a CameraStatusUpdated event
        let event = subscriber.recv().await.unwrap();
        match event {
            ServerEvent::CameraStatusUpdated {
                name,
                temperature_c,
                cooler_power,
                cooler_on,
                dew_heater_on,
                target_temp_c,
            } => {
                assert_eq!(name, "Test Cam");
                assert_eq!(temperature_c, -5.0);
                assert_eq!(cooler_power, Some(60.0));
                assert!(cooler_on);
                assert!(!dew_heater_on);
                assert_eq!(target_temp_c, Some(-10.0));
            }
            other => panic!("Unexpected event: {:?}", other),
        }
    }

    /// Every client counts a warm-up down, not only the one whose Disconnect started it and
    /// read the time left from the answer.
    #[tokio::test]
    async fn a_warming_up_phase_event_carries_the_time_left() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let mut subscriber = state.subscribe_events();
        state
            .slot(CameraRole::Main)
            .begin_warmup(Instant::now() + Duration::from_secs(120));

        state.set_camera_phase(CameraRole::Main, "Ares", CameraPhase::WarmingUp).await;
        state.set_camera_phase(CameraRole::Main, "Ares", CameraPhase::Idle).await;

        let left = |event| match event {
            ServerEvent::CameraPhaseChanged { warmup_remaining_s, .. } => warmup_remaining_s,
            other => panic!("expected a phase change, got {other:?}"),
        };
        let warming = left(subscriber.try_recv().unwrap()).expect("time left");
        assert!((110..=120).contains(&warming), "{warming}");
        assert_eq!(left(subscriber.try_recv().unwrap()), None, "only a warm-up counts down");
    }

    /// -300 °C from a just-reopened Ares-C PRO (2026-09-20) is a glitch, not a reading:
    /// cached, it is what every ramp seed and the UI's temperature would have used.
    #[tokio::test]
    async fn an_implausible_temperature_is_neither_cached_nor_broadcast() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let mut subscriber = state.subscribe_events();
        let glitch = CameraStatus {
            temperature_c: -300.0,
            ..Default::default()
        };

        state.update_camera_status("Ares", glitch, Some(0.0)).await;

        assert!(state.get_camera_status("Ares").await.is_none());
        assert!(subscriber.try_recv().is_err(), "the glitch was broadcast");
    }

    #[tokio::test]
    async fn test_get_camera_status_returns_none_for_unknown() {
        let (state, _disk_writer) = AppState::new_for_testing();
        assert!(state.get_camera_status("Unknown").await.is_none());
    }

    #[tokio::test]
    async fn test_app_state_active_camera_cancellation() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let token = Arc::new(AtomicBool::new(false));

        state
            .set_camera_token(CameraRole::Main, Arc::clone(&token))
            .await;
        assert!(!token.load(Ordering::SeqCst));

        state.cancel_active_exposure(CameraRole::Main).await;
        assert!(token.load(Ordering::SeqCst));

        state.clear_camera_token(CameraRole::Main).await;
        // The token itself remains true, but the slot no longer holds it
        assert!(state
            .slot(CameraRole::Main)
            .cancel_token
            .read()
            .await
            .is_none());
    }

    /// An edit aimed at one camera must not cut short the other's exposure. Cancelling
    /// both would throw away a running imaging sub because the guide camera's gain moved.
    #[tokio::test]
    async fn cancelling_one_slots_exposure_leaves_the_other_running() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let main_token = Arc::new(AtomicBool::new(false));
        let guide_token = Arc::new(AtomicBool::new(false));

        state
            .set_camera_token(CameraRole::Main, Arc::clone(&main_token))
            .await;
        state
            .set_camera_token(CameraRole::Guide, Arc::clone(&guide_token))
            .await;

        state.cancel_active_exposure(CameraRole::Guide).await;

        assert!(guide_token.load(Ordering::SeqCst));
        assert!(
            !main_token.load(Ordering::SeqCst),
            "a guide-camera edit cancelled the imaging camera's exposure"
        );
    }
}
