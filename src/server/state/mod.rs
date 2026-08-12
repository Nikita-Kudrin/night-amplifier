//! Application state management for the web server
//!
//! This module contains the shared state that is accessed by all request handlers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::warn;

use super::events::ServerEvent;
use super::services::PushToState;
use super::settings_persistence::SettingsPersistence;
use crate::camera::{Camera, CameraStatus};
use crate::disk_writer::{DiskWriter, DiskWriterConfig, DiskWriterHandle};
use crate::telemetry::metrics as telemetry_metrics;

mod jpeg_tiers;
mod session;
mod settings;
mod types;

pub use crate::stacking::{StackingType, StackingTypeInfo, WeightingPreset};
pub use jpeg_tiers::{JpegTier, JpegTierCache, JpegTierClientGuard};
pub use session::{CaptureSession, ConnectedCameraInfo};
pub use settings::{CameraCaptureProfile, CaptureSettings, EyepieceSettings, TelescopeSettings};
pub use types::{CameraPhase, CaptureState};

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
    /// Latest rendered frame (LZ4 compressed for streaming)
    pub latest_frame: RwLock<Option<bytes::Bytes>>,
    /// Latest raw frame (uncompressed, for dynamic JPEG encoding)
    pub latest_raw_frame: RwLock<Option<Arc<crate::frame::Frame>>>,
    /// Frame counter (for change detection)
    pub frame_counter: AtomicU64,
    /// Cancellation flag for capture loop
    pub cancel_flag: AtomicBool,
    /// Event broadcast channel
    pub events: broadcast::Sender<ServerEvent>,
    /// Mutex for capture operations (ensures only one capture at a time)
    pub capture_lock: Mutex<()>,
    /// Notification for new frame ready
    pub frame_ready: Arc<tokio::sync::Notify>,
    /// Disk writer handle for saving frames
    pub disk_writer: DiskWriterHandle,
    /// Push-To navigation state
    pub push_to: RwLock<Option<PushToState>>,
    /// Settings persistence manager
    pub settings_persistence: SettingsPersistence,
    /// Cancel token for currently active camera
    pub active_camera_cancel_token: RwLock<Option<Arc<AtomicBool>>>,
    /// Counter for frames dropped due to pipeline back-pressure
    pub dropped_frames: AtomicU64,
    /// Latest reported camera status keyed by camera name (for cooled cameras)
    pub latest_camera_status: RwLock<HashMap<String, CameraStatus>>,
    /// Long-lived camera handle. `Some` while connected and not capturing.
    /// Taken out during a capture session and returned on exit.
    pub active_camera: StdMutex<Option<Box<dyn Camera>>>,
    /// Current lifecycle phase per connected camera (keyed by camera name).
    pub camera_phase: RwLock<HashMap<String, CameraPhase>>,
    /// Sender used by `lifecycle` to issue commands to the running monitor
    /// thread. `None` when no monitor is running.
    pub camera_monitor_tx: StdMutex<Option<std::sync::mpsc::Sender<MonitorCmd>>>,
    /// Number of active JPEG clients per resolution tier. The render task only
    /// encodes tiers somebody is watching.
    pub jpeg_tier_clients: [AtomicUsize; JpegTier::COUNT],
    /// JPEG payloads pre-encoded by the render task, one slot per tier.
    pub jpeg_tier_cache: StdRwLock<JpegTierCache>,
    /// Number of active LZ4 stream clients
    pub lz4_clients: AtomicUsize,
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
            latest_frame: RwLock::new(None),
            latest_raw_frame: RwLock::new(None),
            frame_counter: AtomicU64::new(0),
            cancel_flag: AtomicBool::new(false),
            events: events_tx,
            capture_lock: Mutex::new(()),
            frame_ready: Arc::new(tokio::sync::Notify::new()),
            disk_writer: disk_writer_handle,
            push_to: RwLock::new(push_to),
            settings_persistence,
            active_camera_cancel_token: RwLock::new(None),
            dropped_frames: AtomicU64::new(0),
            latest_camera_status: RwLock::new(HashMap::new()),
            active_camera: StdMutex::new(None),
            camera_phase: RwLock::new(HashMap::new()),
            camera_monitor_tx: StdMutex::new(None),
            jpeg_tier_clients: std::array::from_fn(|_| AtomicUsize::new(0)),
            jpeg_tier_cache: StdRwLock::new(JpegTierCache::default()),
            lz4_clients: AtomicUsize::new(0),
        };

        (state, disk_writer)
    }

    /// Save current settings to disk
    pub async fn save_settings(&self) {
        let settings = self.settings.read().await;
        if let Err(e) = self.settings_persistence.save(&settings) {
            warn!("Failed to save settings: {}", e);
        }
    }

    /// Create new application state for testing
    #[cfg(test)]
    pub fn new_for_testing() -> (Self, DiskWriter) {
        Self::build(
            DiskWriterConfig::default(),
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

    /// Increment frame count and broadcast event
    pub async fn frame_captured(&self, stacked: bool) {
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
            (session.frame_count, session.stacked_count, session.rejected_count)
        };
        let _ = self
            .events
            .send(ServerEvent::frame_captured(frame_number, stacked_count, rejected_count));
    }

    /// Record a rejected frame
    pub async fn frame_rejected(&self, reason: String) {
        let (frame_number, stacked_count, rejected_count) = {
            let mut session = self.session.write().await;
            session.frame_count += 1;
            
            let settings = self.settings.read().await;
            if settings.stacking {
                session.rejected_count += 1;
            }
            
            (session.frame_count, session.stacked_count, session.rejected_count)
        };
        let _ = self.events.send(ServerEvent::frame_rejected(
            frame_number,
            stacked_count,
            rejected_count,
            reason,
        ));
    }

    /// Claim the frame counter for the frame currently being rendered.
    ///
    /// The render task stores every payload for a frame (LZ4 blob, per-tier
    /// JPEGs) against the returned counter and then calls
    /// [`AppState::publish_frame`]. Splitting the two means a woken client never
    /// observes a counter whose payloads are still missing.
    pub fn begin_frame(&self) -> u64 {
        self.frame_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Wake every stream client waiting on a new frame.
    pub fn publish_frame(&self) {
        telemetry_metrics::record_frame_published();
        self.frame_ready.notify_waiters();
    }

    /// Store the LZ4-encoded payload for the lossless stream.
    ///
    /// Storing does not advance the frame counter — see [`AppState::begin_frame`].
    pub async fn set_latest_frame(&self, frame_data: Vec<u8>) {
        let frame_size = frame_data.len() as u64;
        *self.latest_frame.write().await = Some(bytes::Bytes::from(frame_data));
        telemetry_metrics::record_latest_frame_size(frame_size);
    }

    /// Set the latest raw frame for dynamic encoding
    pub async fn set_latest_raw_frame(&self, frame: Arc<crate::frame::Frame>) {
        *self.latest_raw_frame.write().await = Some(frame);
    }

    /// Get the latest frame if available
    pub async fn get_latest_frame(&self) -> Option<bytes::Bytes> {
        self.latest_frame.read().await.clone()
    }

    /// Get the latest raw frame if available
    pub async fn get_latest_raw_frame(&self) -> Option<Arc<crate::frame::Frame>> {
        self.latest_raw_frame.read().await.clone()
    }

    /// Number of clients currently watching a resolution tier.
    pub fn jpeg_tier_client_count(&self, tier: JpegTier) -> usize {
        self.jpeg_tier_clients[tier as usize].load(Ordering::SeqCst)
    }

    /// Look up the pre-encoded JPEG for a tier at the given frame.
    pub fn get_tier_jpeg(&self, tier: JpegTier, counter: u64) -> Option<bytes::Bytes> {
        self.jpeg_tier_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(tier, counter)
    }

    /// Publish a pre-encoded JPEG for a tier and return a shareable handle,
    /// which the render task reuses for tiers that produce the same output.
    pub fn set_tier_jpeg(
        &self,
        tier: JpegTier,
        counter: u64,
        data: impl Into<bytes::Bytes>,
    ) -> bytes::Bytes {
        self.jpeg_tier_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tier, counter, data)
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
        session.last_error = None;
        session.started_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        drop(session);
        self.dropped_frames.store(0, Ordering::SeqCst);
    }

    /// Reset frame counters without resetting session start time
    pub async fn reset_counters(&self) {
        let mut session = self.session.write().await;
        session.frame_count = 0;
        session.stacked_count = 0;
        session.rejected_count = 0;
        drop(session);
        self.dropped_frames.store(0, Ordering::SeqCst);
    }

    /// Set active camera cancel token
    pub async fn set_active_camera_token(&self, token: Arc<AtomicBool>) {
        *self.active_camera_cancel_token.write().await = Some(token);
    }

    /// Clear active camera cancel token
    pub async fn clear_active_camera_token(&self) {
        *self.active_camera_cancel_token.write().await = None;
    }

    /// Cancel currently active camera exposure
    pub async fn cancel_active_exposure(&self) {
        if let Some(token) = self.active_camera_cancel_token.read().await.as_ref() {
            token.store(true, Ordering::SeqCst);
        }
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

    /// Record a dropped frame (pipeline back-pressure) and broadcast event
    pub fn frame_dropped(&self) -> u64 {
        telemetry_metrics::record_frame_dropped();
        let count = self.dropped_frames.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.events.send(ServerEvent::frame_dropped(count));
        count
    }

    /// Get the current dropped frames count
    pub fn dropped_count(&self) -> u64 {
        self.dropped_frames.load(Ordering::SeqCst)
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

    #[tokio::test]
    async fn test_app_state_frame_tracking() {
        let (state, _disk_writer) = AppState::new_for_testing();
        state.reset_session().await;

        state.frame_captured(true).await;
        state.frame_captured(true).await;
        state.frame_captured(false).await;

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

        assert!(state.get_latest_frame().await.is_none());

        state.set_latest_frame(vec![1, 2, 3, 4]).await;
        let frame = state.get_latest_frame().await.unwrap();
        assert_eq!(frame.as_ref(), &[1, 2, 3, 4]);
    }

    /// Storing payloads must not advance the counter — only `begin_frame` does,
    /// so clients cannot wake on a frame whose payloads are still being written.
    #[tokio::test]
    async fn test_begin_frame_owns_the_counter() {
        let (state, _disk_writer) = AppState::new_for_testing();

        state.set_latest_frame(vec![1]).await;
        assert_eq!(state.frame_counter.load(Ordering::SeqCst), 0);

        assert_eq!(state.begin_frame(), 1);
        assert_eq!(state.begin_frame(), 2);
        assert_eq!(state.frame_counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_tier_jpeg_publish_and_lookup() {
        let (state, _disk_writer) = AppState::new_for_testing();

        assert!(state.get_tier_jpeg(JpegTier::Hd1080, 1).is_none());

        state.set_tier_jpeg(JpegTier::Hd1080, 1, vec![7, 8, 9]);
        assert_eq!(
            state.get_tier_jpeg(JpegTier::Hd1080, 1).unwrap().as_ref(),
            &[7, 8, 9]
        );
        assert!(state.get_tier_jpeg(JpegTier::Hd1080, 2).is_none());
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

        state.set_active_camera_token(Arc::clone(&token)).await;
        assert!(!token.load(Ordering::SeqCst));

        state.cancel_active_exposure().await;
        assert!(token.load(Ordering::SeqCst));

        state.clear_active_camera_token().await;
        // The token itself remains true, but AppState no longer holds it
        assert!(state.active_camera_cancel_token.read().await.is_none());
    }
}
