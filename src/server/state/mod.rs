//! Application state management for the web server
//!
//! This module contains the shared state that is accessed by all request handlers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::warn;

use super::events::ServerEvent;
use super::services::PushToState;
use super::settings_persistence::SettingsPersistence;
use crate::camera::CameraStatus;
use crate::disk_writer::{DiskWriter, DiskWriterConfig, DiskWriterHandle};
use crate::telemetry::metrics as telemetry_metrics;

mod camera_slot;
mod capture_mode;
mod frame_stream;
mod jpeg_tiers;
mod session;
mod settings;
mod types;

pub use crate::stacking::{StackingType, StackingTypeInfo, WeightingPreset};
pub use camera_slot::{CameraOp, CameraSlot, RawSessionResume};
pub use capture_mode::{CaptureMode, RawFrameSaving};
pub use frame_stream::FrameStream;
pub use jpeg_tiers::{JpegTier, JpegTierCache, StreamKind, TierClientGuard};
pub use session::{
    CaptureSession, ConnectedCameraInfo, SessionResumePlan, REJECTION_RATE_THRESHOLD,
    REJECTION_RATE_WINDOW,
};
pub use settings::{
    CameraCaptureProfile, CaptureSettings, DenoiseSettings, EyepieceSettings, PreviewResolution,
    SensorCorrectionSettings, TelescopeSettings,
};
pub use types::{CameraPhase, CameraRole, CaptureState, RenderReadyFrame, StretchResult};

/// The main application state shared across all handlers
pub struct AppState {
    /// Currently connected cameras info (camera_id -> info)
    pub cameras: RwLock<HashMap<String, ConnectedCameraInfo>>,
    /// Currently selected camera ID
    pub selected_camera: RwLock<Option<String>>,
    /// Current capture session info
    pub session: RwLock<CaptureSession>,
    /// Capture settings
    pub settings: RwLock<CaptureSettings>,
    /// The main camera's rendered image stream — what `/ws/stream` serves by default.
    pub main_stream: Arc<FrameStream>,
    /// The guide camera's rendered image stream — `/ws/stream?source=guide`.
    ///
    /// Separate from `main_stream` down to the frame counter: sharing one would make
    /// each camera's frames invalidate the other's cached tiers. Its client census is
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
    /// True while the guide loop is actually exposing, so the plate-solve source can be
    /// decided with an atomic load on the stacking thread rather than a lock — see
    /// `capture::solving::SolveSource`.
    ///
    /// Deliberately not "a guide camera is connected". The two diverge in both
    /// directions: a cooled guide camera stays registered for the whole of its warm-up
    /// with its loop already stopped, and `connect` can return before a loop that then
    /// fails to start. Either way presence answered "the guide camera is solving" when
    /// nothing was, and the imaging camera stood down for nothing. `guide_task` owns
    /// this flag: it is set when the loop starts running and cleared when it stops.
    pub guide_loop_running: AtomicBool,
    /// Settings persistence manager
    pub settings_persistence: SettingsPersistence,
    /// Counter for frames dropped due to pipeline back-pressure
    pub dropped_frames: AtomicU64,
    /// Frames the camera actually handed the pipeline this session.
    ///
    /// The denominator [`Self::dropped_frames`] needs. A drop *count* grows all night
    /// and says nothing on its own — 40 drops is a bad evening at 30 s subs and a
    /// rounding error at 100 ms. The rate is what tells an observer they are integrating
    /// at 65 % of the cadence their settings imply.
    pub delivered_frames: AtomicU64,
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
    /// Stop switch for the guide camera's free-running loop. Separate from
    /// `cancel_flag`, which belongs to the imaging capture — stopping a capture must not
    /// stop plate solving, and disconnecting the guide camera must not stop the capture.
    ///
    /// A `std` mutex, not a tokio one, so `guide_task::start` can publish the token
    /// *before* spawning: a disconnect landing in the gap would otherwise find no token,
    /// return immediately, and close the handle underneath a loop that was still
    /// starting up.
    pub guide_cancel: StdMutex<Option<Arc<AtomicBool>>>,
    /// Current lifecycle phase per connected camera (keyed by camera name).
    pub camera_phase: RwLock<HashMap<String, CameraPhase>>,
    /// Consecutive camera faults keyed by camera name, with the instant the
    /// streak was last extended. Every fault detector — the capture watchdog,
    /// the status-poll watchdog and the monitor's cooler poll — feeds this one
    /// counter, so evidence from any of them counts toward the same
    /// escalation. Cleared by a call that succeeds, and aged out after
    /// `camera_health::FAULT_STREAK_TTL` so an alternating fault cannot hide
    /// behind the occasional success. See `camera_health`.
    pub consecutive_watchdog_timeouts: StdMutex<HashMap<String, (u32, Instant)>>,
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
            session: RwLock::new(CaptureSession::default()),
            settings: RwLock::new(settings),
            main_stream: Arc::new(FrameStream::default()),
            guide_stream: Arc::new(FrameStream::default()),
            cancel_flag: AtomicBool::new(false),
            events: events_tx,
            disk_writer: disk_writer_handle,
            push_to: RwLock::new(push_to),
            guide_loop_running: AtomicBool::new(false),
            settings_persistence,
            dropped_frames: AtomicU64::new(0),
            delivered_frames: AtomicU64::new(0),
            latest_camera_status: RwLock::new(HashMap::new()),
            camera_slots: std::array::from_fn(|_| CameraSlot::default()),
            guide_cancel: StdMutex::new(None),
            camera_phase: RwLock::new(HashMap::new()),
            camera_connect_lock: Mutex::new(()),
            session_resume_plan: RwLock::new(None),
            stacking_carryover: StdMutex::new(None),
            consecutive_watchdog_timeouts: StdMutex::new(HashMap::new()),
        };

        (state, disk_writer)
    }

    /// The slot owning `role`'s handle, monitor and reconnect guard.
    pub fn slot(&self, role: CameraRole) -> &CameraSlot {
        &self.camera_slots[role as usize]
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

    /// Whether the guide loop is running. An atomic load, so the stacking thread can
    /// ask it per frame.
    pub fn guide_loop_running(&self) -> bool {
        self.guide_loop_running.load(Ordering::SeqCst)
    }

    pub fn set_guide_loop_running(&self, running: bool) {
        self.guide_loop_running.store(running, Ordering::SeqCst);
    }

    /// Take the guide loop's stop switch, leaving the slot empty. `None` means no loop
    /// is running, which is what makes `guide_task::stop` idempotent.
    pub fn take_guide_cancel(&self) -> Option<Arc<AtomicBool>> {
        self.guide_cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Drop the stop switch without signalling it — for a loop that never started.
    pub fn clear_guide_cancel(&self) {
        *self.guide_cancel.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Save current settings to disk
    pub async fn save_settings(&self) {
        let settings = self.settings.read().await;
        if let Err(e) = self.settings_persistence.save(&settings) {
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
    #[cfg(test)]
    pub fn new_for_testing() -> (Self, DiskWriter) {
        use std::sync::atomic::AtomicU64;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let captures_dir = std::env::temp_dir().join(format!(
            "night_amplifier_test_captures/{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        Self::build(
            DiskWriterConfig::new(captures_dir),
            SettingsPersistence::new("/nonexistent/test/settings.json"),
            CaptureSettings::default(),
            None,
        )
    }

    /// Get the current capture state
    pub async fn capture_state(&self) -> CaptureState {
        self.session.read().await.state
    }

    /// Update capture state and broadcast event
    pub async fn set_capture_state(&self, state: CaptureState) {
        {
            let mut session = self.session.write().await;
            session.state = state;
        }
        let _ = self.events.send(ServerEvent::state_changed(state));
    }

    /// Increment frame count and broadcast event.
    ///
    /// `rejection_reason` says why the frame did not join the stack and is only
    /// meaningful when `stacked` is false. It rides on the `frame_captured`
    /// event rather than going through [`AppState::frame_rejected`], which is
    /// for a camera that failed to deliver a frame and feeds the capture-abort
    /// burst detector — a frame that arrived fine and merely aligned badly must
    /// never reach that.
    pub async fn frame_captured(&self, stacked: bool, rejection_reason: Option<&str>) {
        debug_assert!(
            !(stacked && rejection_reason.is_some()),
            "a stacked frame has no rejection reason"
        );
        let (frame_number, stacked_count, rejected_count) = {
            let mut session = self.session.write().await;
            session.frame_count += 1;
            if stacked {
                session.stacked_count += 1;
            } else {
                let settings = self.settings.read().await;
                if settings.stacking {
                    session.rejected_count += 1;
                }
            }
            (
                session.frame_count,
                session.stacked_count,
                session.rejected_count,
            )
        };
        let _ = self.events.send(ServerEvent::frame_captured(
            frame_number,
            stacked_count,
            rejected_count,
            rejection_reason,
        ));
    }

    /// Record a rejected frame (a camera-capture failure — see
    /// [`CaptureSession::record_rejection`] for how this feeds the
    /// current-failure-burst detection used by `should_stop_on_errors`).
    pub async fn frame_rejected(&self, reason: String) {
        let (frame_number, stacked_count, rejected_count) = {
            let mut session = self.session.write().await;
            session.frame_count += 1;

            let settings = self.settings.read().await;
            if settings.stacking {
                session.rejected_count += 1;
            }
            drop(settings);

            // Tracked regardless of `settings.stacking` — this is about
            // whether the camera itself is responding, not about stacking.
            session.record_rejection(std::time::Instant::now());

            (
                session.frame_count,
                session.stacked_count,
                session.rejected_count,
            )
        };
        let _ = self.events.send(ServerEvent::frame_rejected(
            frame_number,
            stacked_count,
            rejected_count,
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
    pub fn bump_fault_streak(&self, camera_name: &str, ttl: Duration) -> u32 {
        let now = Instant::now();
        let mut counts = self
            .consecutive_watchdog_timeouts
            .lock()
            .expect("consecutive_watchdog_timeouts mutex poisoned");
        let entry = counts.entry(camera_name.to_string()).or_insert((0, now));
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

    /// Reset session for new capture
    pub async fn reset_session(&self) {
        let mut session = self.session.write().await;
        session.frame_count = 0;
        session.stacked_count = 0;
        session.rejected_count = 0;
        session.rejection_timestamps.clear();
        session.last_error = None;
        session.started_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        drop(session);
        self.dropped_frames.store(0, Ordering::SeqCst);
        self.delivered_frames.store(0, Ordering::SeqCst);
    }

    /// Reset frame counters without resetting session start time
    pub async fn reset_counters(&self) {
        let mut session = self.session.write().await;
        session.frame_count = 0;
        session.stacked_count = 0;
        session.rejected_count = 0;
        session.rejection_timestamps.clear();
        drop(session);
        self.dropped_frames.store(0, Ordering::SeqCst);
        self.delivered_frames.store(0, Ordering::SeqCst);
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

    /// Set the lifecycle phase for a camera and broadcast a `CameraPhaseChanged` event.
    pub async fn set_camera_phase(&self, camera_name: &str, phase: CameraPhase) {
        {
            let mut map = self.camera_phase.write().await;
            if phase == CameraPhase::Disconnected {
                map.remove(camera_name);
            } else {
                map.insert(camera_name.to_string(), phase);
            }
        }
        let _ = self
            .events
            .send(ServerEvent::camera_phase_changed(camera_name, phase));
    }

    /// Read the current lifecycle phase for a camera (defaults to Disconnected).
    pub async fn camera_phase(&self, camera_name: &str) -> CameraPhase {
        self.camera_phase
            .read()
            .await
            .get(camera_name)
            .copied()
            .unwrap_or(CameraPhase::Disconnected)
    }

    /// Update the cached "plugin holds a target" flag. No-op without Push-To.
    ///
    /// A cache, not the source of truth — see [`PushToState`]. Written by the
    /// target mutations in `PushToService` and re-synced from `try_plate_solve`,
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
        self.delivered_frames.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Record a dropped frame (pipeline back-pressure) and broadcast event
    pub fn frame_dropped(&self) -> u64 {
        telemetry_metrics::record_frame_dropped();
        let count = self.dropped_frames.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.events.send(ServerEvent::frame_dropped(
            count,
            self.delivered_frames.load(Ordering::SeqCst),
        ));
        count
    }

    /// Get the current dropped frames count
    pub fn dropped_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::SeqCst)
    }

    /// Share of delivered frames the pipeline could not take, `0.0..=1.0`.
    ///
    /// `0.0` before any frame has been delivered rather than a division by zero: no
    /// frames means no evidence, not a perfect session.
    pub fn drop_rate(&self) -> f64 {
        let delivered = self.delivered_frames.load(Ordering::SeqCst);
        if delivered == 0 {
            return 0.0;
        }
        self.dropped_frames.load(Ordering::SeqCst) as f64 / delivered as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count alone is not the number an observer needs.
    ///
    /// 40 drops is a ruined evening at 30 s subs and a rounding error at 100 ms, so the
    /// rate is what says "you are integrating at 65 % of the cadence you set".
    #[test]
    fn the_drop_rate_is_a_share_of_what_the_camera_delivered() {
        let (state, _disk_writer) = AppState::new_for_testing();

        assert_eq!(state.drop_rate(), 0.0, "no frames is no evidence");

        for _ in 0..100 {
            state.frame_delivered();
        }
        assert_eq!(state.drop_rate(), 0.0);

        for _ in 0..35 {
            state.frame_dropped();
        }
        assert!(
            (state.drop_rate() - 0.35).abs() < 1e-9,
            "35 of 100 delivered frames is {}",
            state.drop_rate()
        );

        // A drop with no delivery behind it must not divide by zero or exceed 1.
        let (fresh, _fresh_writer) = AppState::new_for_testing();
        fresh.frame_dropped();
        assert_eq!(fresh.drop_rate(), 0.0);
    }

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

    #[tokio::test]
    async fn test_app_state_frame_tracking() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.reset_session().await;

        state.frame_captured(true, None).await;
        state.frame_captured(true, None).await;
        state.frame_captured(false, None).await;

        let session = state.session.read().await;
        assert_eq!(session.frame_count, 3);
        assert_eq!(session.stacked_count, 2);
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

    #[tokio::test]
    async fn test_app_state_frame_storage() {
        let (state, _disk_writer) = AppState::new_for_testing();

        assert!(state.main_stream.get_latest_frame().await.is_none());

        state.main_stream.set_latest_frame(vec![1, 2, 3, 4]).await;
        let frame = state.main_stream.get_latest_frame().await.unwrap();
        assert_eq!(frame.as_ref(), &[1, 2, 3, 4]);
    }

    /// Storing payloads must not advance the counter — only `begin_frame` does,
    /// so clients cannot wake on a frame whose payloads are still being written.
    #[tokio::test]
    async fn test_begin_frame_owns_the_counter() {
        let (state, _disk_writer) = AppState::new_for_testing();

        state.main_stream.set_latest_frame(vec![1]).await;
        assert_eq!(state.main_stream.frame_counter(), 0);

        assert_eq!(state.main_stream.begin_frame(), 1);
        assert_eq!(state.main_stream.begin_frame(), 2);
        assert_eq!(state.main_stream.frame_counter(), 2);
    }

    #[tokio::test]
    async fn test_tier_jpeg_publish_and_lookup() {
        let (state, _disk_writer) = AppState::new_for_testing();

        assert!(state.main_stream.get_tier_jpeg(JpegTier::Hd1080, 1).is_none());

        state
            .main_stream
            .set_tier_jpeg(JpegTier::Hd1080, 1, vec![7, 8, 9]);
        assert_eq!(
            state
                .main_stream
                .get_tier_jpeg(JpegTier::Hd1080, 1)
                .unwrap()
                .as_ref(),
            &[7, 8, 9]
        );
        assert!(state.main_stream.get_tier_jpeg(JpegTier::Hd1080, 2).is_none());
    }

    /// The two streams are independent down to the counter. Sharing one would make each
    /// camera's frames invalidate the other's cached tiers on every exposure.
    #[tokio::test]
    async fn guide_and_main_streams_do_not_share_a_counter_or_cache() {
        let (state, _disk_writer) = AppState::new_for_testing();

        let main_counter = state.main_stream.begin_frame();
        state
            .main_stream
            .set_tier_jpeg(JpegTier::Hd1080, main_counter, vec![1]);

        // Three guide frames must leave the main stream's cached payload readable.
        for _ in 0..3 {
            let guide_counter = state.guide_stream.begin_frame();
            state
                .guide_stream
                .set_tier_jpeg(JpegTier::Hd1080, guide_counter, vec![2]);
        }

        assert_eq!(state.main_stream.frame_counter(), 1);
        assert_eq!(state.guide_stream.frame_counter(), 3);
        assert_eq!(
            state
                .main_stream
                .get_tier_jpeg(JpegTier::Hd1080, main_counter)
                .unwrap()
                .as_ref(),
            &[1]
        );
    }

    /// The guide loop's render gate. With nobody watching it must report no viewers, or
    /// it would post-process and encode a stream no one can see.
    #[tokio::test]
    async fn has_viewers_tracks_tier_client_guards() {
        let (state, _disk_writer) = AppState::new_for_testing();
        let stream = Arc::clone(&state.guide_stream);

        assert!(!stream.has_viewers());

        let guard = TierClientGuard::new(Arc::clone(&stream), StreamKind::Jpeg, JpegTier::Hd1080);
        assert!(stream.has_viewers());

        drop(guard);
        assert!(!stream.has_viewers());

        let lossless = TierClientGuard::new(
            Arc::clone(&stream),
            StreamKind::Lossless,
            JpegTier::LOSSLESS_DEFAULT,
        );
        assert!(stream.has_viewers());
        drop(lossless);
        assert!(!stream.has_viewers());
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
