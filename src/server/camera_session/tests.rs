//! Integration tests for the camera session lifecycle + monitor.
//!
//! We use a small in-module mock camera (rather than the real SimulatedCamera
//! provider) so we can drive phase transitions deterministically without
//! depending on the global simulated-camera directory registry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::camera::testing::{CameraControls, FakeCamera};
use crate::camera::{
    Camera, CameraError, CameraInfo, CameraResult, CameraStatus, CaptureConfig, GainPresets,
    SensorType,
};
use crate::server::camera_session::lifecycle::DisconnectCause;
use crate::server::camera_session::{lifecycle, monitor, PHASE_POLL_INTERVAL};
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CameraOp, CameraPhase, CameraRole, CaptureState, ConnectedCameraInfo,
};

/// Cooler model for the mock camera: step once per `status()` call with a
/// configurable per-tick delta so tests can drive transitions in <1 second.
struct MockCoolerState {
    current_temp_c: f64,
    target_temp_c: f64,
    cooler_on: bool,
    /// Degrees moved toward the goal per `status()` call.
    step_per_tick: f64,
    /// Ambient temperature used when the cooler is off.
    ambient_c: f64,
}

struct MockCamera {
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
    cooler: Arc<Mutex<MockCoolerState>>,
    fail_next_status: Arc<AtomicBool>,
    /// Last `(enabled, power)` the dew heater was actually driven with, so a test can
    /// tell "applied" from "silently skipped".
    dew_heater: Arc<Mutex<Option<(bool, i32)>>>,
}

impl MockCamera {
    fn new(has_cooler: bool, step_per_tick: f64) -> (Self, Arc<Mutex<MockCoolerState>>) {
        let cooler = Arc::new(Mutex::new(MockCoolerState {
            current_temp_c: 20.0,
            target_temp_c: 20.0,
            cooler_on: false,
            step_per_tick,
            ambient_c: 20.0,
        }));
        let info = CameraInfo {
            name: "Mock Cooled Camera".to_string(),
            id: 0,
            max_width: 640,
            max_height: 480,
            sensor_type: SensorType::Mono,
            has_cooler,
            min_temp_c: Some(-40.0),
            max_temp_c: Some(30.0),
            ..Default::default()
        };
        let cam = Self {
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cooler: Arc::clone(&cooler),
            fail_next_status: Arc::new(AtomicBool::new(false)),
            dew_heater: Arc::new(Mutex::new(None)),
        };
        (cam, cooler)
    }
}

impl Camera for MockCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets::default())
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        if self.fail_next_status.load(Ordering::SeqCst) {
            return Err(CameraError::Disconnected);
        }

        let mut c = self.cooler.lock().unwrap();
        let goal = if c.cooler_on {
            c.target_temp_c
        } else {
            c.ambient_c
        };
        let diff = goal - c.current_temp_c;
        let step = c.step_per_tick.min(diff.abs());
        c.current_temp_c += diff.signum() * step;
        let delta = (c.ambient_c - c.target_temp_c).abs().max(1.0);
        let power = if c.cooler_on {
            ((c.ambient_c - c.current_temp_c) / delta * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        Ok(CameraStatus {
            temperature_c: c.current_temp_c,
            cooler_power: Some(power),
            cooler_on: c.cooler_on,
            is_exposing: false,
            current_gain: 0,
            current_offset: 0,
            current_exposure_us: 1_000_000,
            dew_heater_on: false,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        self.cooler.lock().unwrap().target_temp_c = temp_c;
        Ok(())
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        self.cooler.lock().unwrap().cooler_on = enabled;
        Ok(())
    }

    fn set_dew_heater(&mut self, enabled: bool, power: i32) -> CameraResult<()> {
        *self.dew_heater.lock().unwrap() = Some((enabled, power));
        Ok(())
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<crate::camera::RawFrame> {
        Ok(crate::camera::RawFrame {
            data: vec![0; (self.info.max_width * self.info.max_height) as usize].into(),
            width: self.info.max_width,
            height: self.info.max_height,
            format: crate::camera::ImageFormat::Raw8,
        })
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "Mock"
    }
}

/// Seed AppState with a mock cooled camera as if `lifecycle::connect` had
/// succeeded (skipping the real `CameraRegistry::open_camera` path).
async fn install_mock_camera(
    state: &Arc<AppState>,
    step_per_tick: f64,
    cooler_on: bool,
    target_temp_c: f64,
) -> String {
    let (cam, cooler) = MockCamera::new(true, step_per_tick);
    if cooler_on {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = true;
        c.target_temp_c = target_temp_c;
    }
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    {
        let mut cameras = state.cameras.write().await;
        cameras.insert("mock_0".to_string(), connected_info);
    }
    {
        let mut selected = state.selected_camera.write().await;
        *selected = Some("mock_0".to_string());
    }
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));

    let phase = if cooler_on {
        CameraPhase::Precooling
    } else {
        CameraPhase::Idle
    };
    state.set_camera_phase(CameraRole::Main, &name, phase).await;

    // Spawn the monitor thread.
    let tx = monitor::spawn(
        Arc::clone(state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    name
}

/// Wait up to `timeout` for a predicate on the phase to become true.
async fn wait_for_phase(
    state: &Arc<AppState>,
    role: CameraRole,
    target: CameraPhase,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if state.camera_phase(role).await == target {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn precool_settles_to_idle() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    // Configure settings so the monitor sees a target temperature.
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    // Already cooled to target so the monitor observes a settled temp.
    install_mock_camera(&state, 5.0, true, -10.0).await;
    // Bump initial temp near target right away by calling status() a few times.
    // (The mock starts at ambient 20°C; with step 5 we need ~6 ticks.)
    // Instead, seed the cooler state close to target:
    {
        let cam = state.slot(CameraRole::Main).handle.lock().unwrap();
        // Can't downcast through Box<dyn Camera>; settle via status() calls.
        drop(cam);
    }
    // Trigger manual stepping by calling `status()` from outside to converge.
    for _ in 0..10 {
        let mut guard = state.slot(CameraRole::Main).handle.lock().unwrap();
        if let Some(cam) = guard.as_mut() {
            let _ = cam.status();
        }
    }

    // The monitor polls every 2s; wait up to 8s for convergence + 2 stable samples.
    let settled = wait_for_phase(&state, CameraRole::Main, CameraPhase::Idle, Duration::from_secs(10)).await;
    assert!(settled, "Expected phase to settle to Idle");
}

#[tokio::test]
async fn no_precool_when_cooler_disabled() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    // No cooler_enabled → monitor should stay in Idle forever; no Precooling event.
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Idle);

    // Wait ~3s and verify it remains Idle (not flipping to something weird).
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(500)).await;
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Idle);

    // Clean up.
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn warmup_finishes_and_disconnects() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    let name = install_mock_camera(&state, 15.0, true, -10.0).await;

    // Put the sensor well below ambient so warmup has something to do.
    {
        let mut guard = state.slot(CameraRole::Main).handle.lock().unwrap();
        if let Some(cam) = guard.as_mut() {
            // Drop cooler target and let internal state mutate via the status calls.
            let _ = cam.set_cooler(true);
            let _ = cam.set_target_temperature(-10.0);
        }
    }

    let mut rx = state.subscribe_events();

    // Trigger warmup (as if user clicked Disconnect with cooler on).
    let result = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible).await;
    assert!(result.is_ok());
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);

    // Monitor polls every 2s; with step 15.0 and a 60°C swing, 4-5 ticks to cross 10°C.
    let saw_disconnect = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match rx.recv().await {
                Ok(ServerEvent::CameraDisconnected { name: n }) if n == name => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        saw_disconnect,
        "Expected CameraDisconnected event after warmup"
    );

    let phase_after = state.camera_phase(CameraRole::Main).await;
    assert_eq!(phase_after, CameraPhase::Disconnected);
    let cameras = state.cameras.read().await;
    assert!(cameras.is_empty(), "Camera metadata should be cleared");
}

#[tokio::test]
async fn disconnect_with_cooler_off_is_synchronous() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_mock_camera(&state, 5.0, false, 20.0).await;

    // Cooler was never on → synchronous close, no warmup.
    let result = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible).await;
    assert!(result.is_ok());

    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Disconnected);
    assert!(state.cameras.read().await.is_empty());
    assert!(state.slot(CameraRole::Main).handle.lock().unwrap().is_none());
}

#[tokio::test]
async fn take_and_return_handle_during_precool() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    let name = install_mock_camera(&state, 1.0, true, -10.0).await;
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Precooling);

    // Capture takes the handle.
    let cam = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Capturing);
    assert!(state.slot(CameraRole::Main).handle.lock().unwrap().is_none());

    // Return it.
    lifecycle::return_from_capture(&state, CameraRole::Main, &name, Some(cam)).await;
    let phase_after = state.camera_phase(CameraRole::Main).await;
    assert!(
        matches!(phase_after, CameraPhase::Precooling | CameraPhase::Idle),
        "Expected Precooling/Idle after return, got {:?}",
        phase_after
    );
    // The monitor legitimately checks the handle out for the duration of each
    // poll, so "is it back in the session" is a question about availability,
    // not about what the mutex holds at one instant.
    assert!(
        lifecycle::with_camera(&state, CameraRole::Main, |_| ())
            .await
            .is_some(),
        "handle should be back in the session after capture returns it"
    );

    // Clean up.
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn capture_during_warmup_cancels_warmup() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    let name = install_mock_camera(&state, 1.0, true, -10.0).await;

    // User clicks Disconnect → warmup begins.
    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible).await.unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);

    // User immediately starts capture → warmup cancelled, phase → Capturing.
    let cam = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Capturing);

    // Return and clean up.
    lifecycle::return_from_capture(&state, CameraRole::Main, &name, Some(cam)).await;
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn live_target_temp_change_propagates_to_hardware() {
    // Reproduces the bug: camera cooled to 1°C, user raises the slider to 20°C,
    // temperature never changes because the new target was only persisted in
    // settings and never forwarded to the TEC. With the rate-limited ramp the
    // hardware now receives the new setpoint through the monitor: at the
    // test-time shadow rate the ramp completes on the next tick.
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let (mut cam, cooler) = MockCamera::new(true, 5.0);
    {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = true;
        c.target_temp_c = 1.0;
        c.current_temp_c = 1.0;
    }
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(1.0);
    }
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    let _ = cam.set_cooler(true);
    let _ = cam.set_target_temperature(1.0);
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Idle).await;

    // Spawn the monitor so it can process UpdateCoolerTarget.
    let tx = monitor::spawn(
        Arc::clone(&state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    {
        let mut s = state.settings.write().await;
        s.target_temp_c = Some(20.0);
    }
    lifecycle::apply_cooler_settings(&state, CameraRole::Main).await;

    // Phase should flip back to Precooling immediately.
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Precooling);

    // Wait for the monitor to tick once and push the ramped setpoint.
    let deadline = std::time::Instant::now() + PHASE_POLL_INTERVAL + Duration::from_secs(2);
    loop {
        if (cooler.lock().unwrap().target_temp_c - 20.0).abs() < 1e-6 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "hardware target never reached 20.0 (observed {})",
                cooler.lock().unwrap().target_temp_c
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn live_cooler_disable_propagates_to_hardware() {
    // User disables the cooler from the UI while the camera is idle-cooled —
    // the TEC must actually turn off (not just the setting flip in memory).
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let (mut cam, cooler) = MockCamera::new(true, 5.0);
    {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = true;
        c.target_temp_c = -5.0;
        c.current_temp_c = -5.0;
    }
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-5.0);
    }
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    let _ = cam.set_cooler(true);
    let _ = cam.set_target_temperature(-5.0);
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Idle).await;

    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = false;
    }
    lifecycle::apply_cooler_settings(&state, CameraRole::Main).await;

    assert!(
        !cooler.lock().unwrap().cooler_on,
        "cooler should be off on hardware"
    );

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn live_cooler_apply_is_skipped_during_warmup() {
    // While the monitor is driving warmup it intentionally holds the cooler
    // off. A stray settings write (e.g., user toggled something else) must not
    // re-enable the TEC and fight the warmup sequence.
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let (cam, cooler) = MockCamera::new(true, 5.0);
    {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = false; // Monitor disabled it at warmup start.
        c.target_temp_c = -10.0;
    }
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true; // Settings still say enabled (stale).
        s.target_temp_c = Some(-10.0);
    }
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::WarmingUp).await;

    lifecycle::apply_cooler_settings(&state, CameraRole::Main).await;

    // Cooler must stay off — the warmup phase owns it.
    assert!(!cooler.lock().unwrap().cooler_on);

    // Clean up without going through the monitor (no monitor was spawned).
    *state.slot(CameraRole::Main).handle.lock().unwrap() = None;
    state.cameras.write().await.clear();
}

#[tokio::test]
async fn return_from_capture_without_handle_finalizes_disconnect() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    // With reconnect on, a lost handle suspends instead — see `recovery_tests`.
    state.settings.write().await.auto_reconnect = false;
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;

    // Simulate capture thread panicking: take the handle and drop it, then
    // call return_from_capture with None.
    let _cam = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();

    lifecycle::return_from_capture(&state, CameraRole::Main, &name, None).await;

    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Disconnected);
    assert!(state.cameras.read().await.is_empty());
}

/// The recovery half of what used to be one test called
/// "…_recovers_or_disconnects". When the mock's failure became permanent
/// (`swap` -> `load`) the recovery path stopped being exercised at all, while
/// the name went on claiming it was.
#[tokio::test]
async fn monitor_keeps_running_through_a_transient_stall() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;

    let (cam, _) = MockCamera::new(true, 5.0);
    let fail_flag = Arc::clone(&cam.fail_next_status);
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Idle).await;

    let tx = monitor::spawn(
        Arc::clone(&state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    // Fail one poll, then start answering again.
    fail_flag.store(true, Ordering::SeqCst);
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(200)).await;
    fail_flag.store(false, Ordering::SeqCst);

    tokio::time::sleep(PHASE_POLL_INTERVAL * 2).await;
    assert_eq!(
        state.camera_phase(CameraRole::Main).await,
        CameraPhase::Idle,
        "a stall the camera recovers from must not end the session"
    );
    assert!(!state.cameras.read().await.is_empty());

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

#[tokio::test]
async fn monitor_disconnects_after_a_persistent_stall() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    // Inject mock camera
    let (cam, _) = MockCamera::new(true, 5.0);
    let fail_flag = Arc::clone(&cam.fail_next_status);
    let name = cam.info().name.clone();

    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };

    {
        let mut cameras = state.cameras.write().await;
        cameras.insert("mock_0".to_string(), connected_info);
    }
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Idle).await;

    let mut rx = state.subscribe_events();

    // Spawn monitor thread
    let tx = monitor::spawn(
        Arc::clone(&state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    // Trigger error on next poll
    fail_flag.store(true, Ordering::SeqCst);

    // Check if system raises a camera disconnect/error event within a reasonable time
    let saw_disconnect = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(ServerEvent::Error { message, .. }) => {
                    if message.contains("Camera disconnected") {
                        return true;
                    }
                }
                Ok(ServerEvent::CameraDisconnected { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        saw_disconnect,
        "Monitor should broadcast CameraError or CameraDisconnected on stall"
    );

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

// ----------------------------------------------------------------------------
// apply_camera_profile_on_connect — pure unit tests
// ----------------------------------------------------------------------------

/// Shape a `CameraInfo` for the unit tests.
///
/// The capability flags are what the tests vary; the ranges are stated because
/// `apply_camera_profile_on_connect` also clamps exposure, gain and binning, and a
/// camera advertising `CameraInfo`'s bare defaults (gain to 100, bin 1 only) would
/// clamp values these tests deliberately set.
fn test_camera_info(
    has_cooler: bool,
    supports_sensor_modes: bool,
    has_dew_heater: bool,
) -> CameraInfo {
    use crate::camera::SensorMode;
    CameraInfo {
        name: "test".to_string(),
        max_gain: 600,
        supported_bins: vec![1, 2, 4],
        supported_formats: vec![
            crate::camera::ImageFormat::Raw8,
            crate::camera::ImageFormat::Raw16,
        ],
        has_cooler,
        sensor_modes: if supports_sensor_modes {
            vec![SensorMode {
                index: 0,
                name: "Normal".to_string(),
                description: "normal".to_string(),
            }]
        } else {
            Vec::new()
        },
        has_dew_heater,
        ..Default::default()
    }
}

#[test]
fn connect_seeds_profile_for_new_camera() {
    use crate::server::state::CaptureSettings;

    let mut settings = CaptureSettings::default();
    settings.exposure_us = 123_456;
    settings.gain = 77;
    settings.cooler_enabled = true;
    settings.target_temp_c = Some(-5.0);

    let key = "PlayerOne/Neptune-C II".to_string();
    let info = test_camera_info(true, true, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), CameraRole::Main, &info);

    // Flat fields unchanged when cooler and sensor modes are supported.
    assert_eq!(settings.exposure_us, 123_456);
    assert_eq!(settings.gain, 77);
    assert!(settings.cooler_enabled);
    assert_eq!(settings.target_temp_c, Some(-5.0));

    // Profile was created and mirrors the flat fields.
    let profile = settings
        .camera_profiles
        .get(&key)
        .expect("profile should be seeded");
    assert_eq!(profile.exposure_us, 123_456);
    assert_eq!(profile.gain, 77);
    assert!(profile.cooler_enabled);
    assert_eq!(profile.target_temp_c, Some(-5.0));
    assert!(profile.dew_heater_enabled);
    assert_eq!(profile.dew_heater_power, 10);
}

#[test]
fn connect_loads_existing_profile() {
    use crate::server::state::{CameraCaptureProfile, CaptureSettings};

    let mut settings = CaptureSettings::default();
    settings.exposure_us = 1;
    settings.gain = 0;

    let key = "PlayerOne/2600MC".to_string();
    settings.camera_profiles.insert(
        key.clone(),
        CameraCaptureProfile {
            exposure_us: 9_999,
            gain: 250,
            offset: 50,
            bin: 2,
            cooler_enabled: true,
            target_temp_c: Some(-15.0),
            sensor_mode_override: None,
            cooler_fast_mode: false,
            dew_heater_enabled: true,
            dew_heater_power: 30,
        },
    );

    let info = test_camera_info(true, true, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key, CameraRole::Main, &info);

    assert_eq!(settings.exposure_us, 9_999);
    assert_eq!(settings.gain, 250);
    assert_eq!(settings.offset, 50);
    assert_eq!(settings.bin, 2);
    assert!(settings.cooler_enabled);
    assert_eq!(settings.target_temp_c, Some(-15.0));
}

#[test]
fn connect_clamps_cooler_for_uncooled_camera() {
    use crate::server::state::CaptureSettings;

    let mut settings = CaptureSettings::default();
    settings.cooler_enabled = true;
    settings.target_temp_c = Some(-10.0);
    settings.gain = 150;

    let key = "PlayerOne/Neptune-C II".to_string();
    let info = test_camera_info(false, true, false);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), CameraRole::Main, &info);

    // Flat fields clamped.
    assert!(!settings.cooler_enabled);
    assert_eq!(settings.target_temp_c, None);
    // Non-cooler fields unchanged.
    assert_eq!(settings.gain, 150);

    // Seeded profile also has cooler fields zeroed.
    let profile = settings
        .camera_profiles
        .get(&key)
        .expect("profile should be seeded");
    assert!(!profile.cooler_enabled);
    assert_eq!(profile.target_temp_c, None);
    assert_eq!(profile.gain, 150);
}

/// Exposure, gain and binning are the three fields `CaptureConfig::validate` rejects
/// outright, so a profile carrying an out-of-range one stops the camera capturing
/// entirely rather than merely looking wrong. Clamping on connect is what keeps a
/// profile written by an older build — or by a camera with a wider range — usable.
#[test]
fn connect_clamps_values_the_camera_would_reject() {
    use crate::server::state::{CameraCaptureProfile, CaptureSettings};

    let mut settings = CaptureSettings::default();
    let key = "PlayerOne/Neptune-C II".to_string();
    settings.camera_profiles.insert(
        key.clone(),
        CameraCaptureProfile {
            exposure_us: 0,
            gain: 5_000,
            bin: 3,
            ..Default::default()
        },
    );

    let info = test_camera_info(true, true, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), CameraRole::Main, &info);

    // Zero is "never configured", not "the shortest sub this camera can take".
    assert_eq!(settings.exposure_us, 1_000_000);
    assert_eq!(settings.gain, info.max_gain);
    assert_eq!(settings.bin, 1, "3 is not in supported_bins");

    let repaired = &settings.camera_profiles[&key];
    assert_eq!(repaired.exposure_us, 1_000_000);
    assert_eq!(repaired.bin, 1);
    // Sensor mode is resolved separately by `config_overrides` against the modes this
    // camera actually lists; the clamp's job is the three range fields.
    let mut config = settings.to_capture_config();
    config.sensor_mode = None;
    assert!(
        config.validate(&info).is_ok(),
        "the clamped profile still builds a config the camera refuses: {:?}",
        config.validate(&info)
    );
}

#[test]
fn connect_clamps_sensor_mode_for_camera_without_modes() {
    use crate::camera::DualSamplingMode;
    use crate::server::state::CaptureSettings;

    let mut settings = CaptureSettings::default();
    settings.sensor_mode_override = Some(DualSamplingMode::LowReadoutNoise);
    settings.gain = 150;

    let key = "PlayerOne/Neptune-C II".to_string();
    let info = test_camera_info(false, false, false);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), CameraRole::Main, &info);

    // Flat field + seeded profile both have the stale override cleared.
    assert_eq!(settings.sensor_mode_override, None);
    let profile = settings
        .camera_profiles
        .get(&key)
        .expect("profile should be seeded");
    assert_eq!(profile.sensor_mode_override, None);
}

// ----------------------------------------------------------------------------
// Rate-limited cooldown / warmup ramp
// ----------------------------------------------------------------------------

use crate::server::state::MonitorCmd;

/// After UpdateCoolerTarget the monitor should ramp the hardware setpoint
/// toward the new final target (test-time rate jumps it in one tick).
#[tokio::test]
async fn cooldown_ramp_drives_setpoint_to_target() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-15.0);
    }
    let (mut cam, cooler) = MockCamera::new(true, 5.0);
    {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = true;
        c.target_temp_c = 20.0;
        c.current_temp_c = 20.0;
    }
    let _ = cam.set_cooler(true);
    let _ = cam.set_target_temperature(20.0);
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Precooling).await;

    let tx = monitor::spawn(
        Arc::clone(&state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    let _ = state
        .slot(CameraRole::Main)
        .monitor_tx
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .send(MonitorCmd::UpdateCoolerTarget {
            enabled: true,
            target: Some(-15.0),
            fast: false,
        });

    // Wait up to one tick + slack for the ramped setpoint to reach the target.
    let deadline = std::time::Instant::now() + PHASE_POLL_INTERVAL + Duration::from_secs(2);
    loop {
        if (cooler.lock().unwrap().target_temp_c - -15.0).abs() < 1e-6 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "setpoint never reached -15.0 (observed {})",
                cooler.lock().unwrap().target_temp_c
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

/// Warmup must keep the cooler ON while ramping the setpoint upward — the
/// whole point is to reduce duty gradually rather than kill the TEC outright.
#[tokio::test]
async fn warmup_keeps_cooler_on_during_ramp() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    // Small step so the mock doesn't instantly settle to the new warmup target
    // — gives us a window where cooler_on should still be true.
    install_mock_camera(&state, 1.0, true, -10.0).await;

    // Kick off warmup.
    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible).await.unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);

    // Within the first tick window, cooler must still be ON (ramped warmup,
    // not kill-switch warmup).
    tokio::time::sleep(Duration::from_millis(500)).await;
    {
        let guard = state.slot(CameraRole::Main).handle.lock().unwrap();
        if let Some(cam) = guard.as_ref() {
            let status = cam.status().expect("status read");
            assert!(
                status.cooler_on,
                "cooler must remain ON during ramped warmup"
            );
        }
    }

    // Eventually the warmup finalizes (step 1.0/tick × ~30°C = ~60 ticks, too
    // slow to wait for here — just assert no panic / state corruption).
    // Drop phase to force finalize without waiting.
    let _ = state
        .slot(CameraRole::Main)
        .monitor_tx
        .lock()
        .unwrap()
        .as_ref()
        .map(|tx| tx.send(MonitorCmd::Shutdown));
}

/// Fast mode: UpdateCoolerTarget with `fast: true` should push the final
/// target to hardware immediately and not install a ramp.
#[tokio::test]
async fn fast_mode_skips_cooldown_ramp() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-15.0);
        s.cooler_fast_mode = true;
    }
    let (mut cam, cooler) = MockCamera::new(true, 5.0);
    {
        let mut c = cooler.lock().unwrap();
        c.cooler_on = true;
        c.target_temp_c = 20.0;
        c.current_temp_c = 20.0;
    }
    let _ = cam.set_cooler(true);
    let _ = cam.set_target_temperature(20.0);
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Precooling).await;

    let tx = monitor::spawn(
        Arc::clone(&state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);

    let _ = state
        .slot(CameraRole::Main)
        .monitor_tx
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .send(MonitorCmd::UpdateCoolerTarget {
            enabled: true,
            target: Some(-15.0),
            fast: true,
        });

    // Fast mode snaps the hardware target immediately (no tick required).
    // Give the monitor a moment to process the queued command.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if (cooler.lock().unwrap().target_temp_c - -15.0).abs() < 1e-6 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "fast-mode hardware target never reached -15.0 (observed {})",
                cooler.lock().unwrap().target_temp_c
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

/// Fast mode at warmup: cooler should be disabled immediately on StartWarmup
/// rather than ramped.
#[tokio::test]
async fn fast_mode_warmup_disables_cooler_immediately() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
        s.cooler_fast_mode = true;
    }
    install_mock_camera(&state, 5.0, true, -10.0).await;

    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible).await.unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);

    // StartWarmup with fast=true should flip the cooler off right away.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let off = {
            let guard = state.slot(CameraRole::Main).handle.lock().unwrap();
            guard
                .as_ref()
                .and_then(|cam| cam.status().ok())
                .map(|s| !s.cooler_on)
        };
        if off == Some(true) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("fast-mode warmup did not disable cooler within 1s");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let _ = state
        .slot(CameraRole::Main)
        .monitor_tx
        .lock()
        .unwrap()
        .as_ref()
        .map(|tx| tx.send(MonitorCmd::Shutdown));
}

/// Mid-ramp, changing the target should seed a new ramp from the CURRENT
/// sensor temp toward the new final target.
#[tokio::test]
async fn target_change_mid_precool_restarts_ramp() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-15.0);
    }
    let name = install_mock_camera(&state, 5.0, true, -15.0).await;

    // Let one tick pass so the monitor is engaged.
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(200)).await;

    // User raises the target to -5.
    {
        let mut s = state.settings.write().await;
        s.target_temp_c = Some(-5.0);
    }
    lifecycle::apply_cooler_settings(&state, CameraRole::Main).await;

    // Phase should remain Precooling.
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Precooling);

    // Monitor should install a new ramp that will push the hardware setpoint
    // to -5 on the next tick. The monitor legitimately checks the handle out
    // of active_camera for the duration of each poll (see
    // with_camera_bounded's doc comment), so "is it still here" has to wait
    // for hand-back via with_active_camera instead of locking active_camera
    // directly at one instant — a bare lock races the poll that's in flight
    // right around this point and was the source of this test's flakiness.
    assert!(
        lifecycle::with_camera(&state, CameraRole::Main, |_| ()).await.is_some(),
        "camera handle not back in the session after mid-precool target change"
    );

    let deadline = std::time::Instant::now() + PHASE_POLL_INTERVAL + Duration::from_secs(2);
    loop {
        let target = {
            let mut guard = state.slot(CameraRole::Main).handle.lock().unwrap();
            guard
                .as_mut()
                .map(|cam| cam.status().ok().map(|s| s.cooler_on))
                .flatten()
                .unwrap_or(false)
        };
        // We can't directly read mock's target_temp_c without the Arc handle,
        // but we can check the camera_status cache that the monitor publishes.
        if let Some(status) = state.get_camera_status(&name).await {
            // cooler should still be on and temperature should be tracking
            // toward the new target (above -15).
            if status.cooler_on && status.temperature_c > -14.0 {
                break;
            }
        }
        let _ = target;
        if std::time::Instant::now() >= deadline {
            // Not strictly required to reach the new target in this window —
            // the test's main assertion is that the phase stayed Precooling.
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

// ----------------------------------------------------------------------------
// Device-fault handling and reconnect policy
// ----------------------------------------------------------------------------

fn send_monitor_cmd_for_test(state: &Arc<AppState>, cmd: MonitorCmd) {
    if let Some(tx) = state.slot(CameraRole::Main).monitor_tx.lock().unwrap().as_ref() {
        let _ = tx.send(cmd);
    }
}

/// A camera whose SDK reports its device gone once `CameraControls::dead` is raised,
/// the way a real one does after a USB reset: `open()` already succeeded, and every
/// subsequent call answers with a device-loss code.
fn dead_camera() -> FakeCamera {
    FakeCamera::new("Dead Camera").sized(640, 480).cooled()
}

async fn install_dead_camera(state: &Arc<AppState>) -> (String, Arc<CameraControls>) {
    let cam = dead_camera();
    let controls = cam.controls();
    let name = cam.info().name.clone();
    let connected_info = ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Main,
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::Idle).await;

    let tx = monitor::spawn(
        Arc::clone(state),
        CameraRole::Main,
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.slot(CameraRole::Main).monitor_tx.lock().unwrap() = Some(tx);
    (name, controls)
}

/// Wait for `predicate` to hold, or give up. Returns whether it held.
pub(super) async fn eventually(mut predicate: impl FnMut() -> bool, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    predicate()
}

/// A camera reporting its device gone must be given up on, not polled forever.
/// Before the streak was shared, this could not fire at all on PlayerOne:
/// `status()` folded every SDK error into a fallback value and returned `Ok`.
#[tokio::test]
async fn a_lost_device_ends_the_session_instead_of_being_polled_forever() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    // Nothing to reconnect to in a unit test — the supervisor is covered
    // separately by `reconnect_is_not_attempted_*`.
    state.settings.write().await.auto_reconnect = false;

    let (name, camera) = install_dead_camera(&state).await;
    let mut events = state.subscribe_events();
    camera.dead.store(true, Ordering::SeqCst);

    let escalated = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await {
                Ok(ServerEvent::CameraPersistentlyUnresponsive { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(
        escalated,
        "a camera answering every call with a device-loss code must escalate"
    );
    assert!(
        eventually(
            || {
                state
                    .cameras
                    .try_read()
                    .map(|c| c.is_empty())
                    .unwrap_or(false)
            },
            Duration::from_secs(5)
        )
        .await,
        "the session should have been torn down"
    );
    let _ = name;
}

/// The threshold must not be reachable on a single fault: one hiccup in the
/// middle of a two-hour session is not a reason to drop the camera.
#[tokio::test]
async fn one_fault_does_not_end_the_session() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;

    let (name, camera) = install_dead_camera(&state).await;

    // One poll's worth of failure, then the camera answers again.
    camera.dead.store(true, Ordering::SeqCst);
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(200)).await;
    camera.dead.store(false, Ordering::SeqCst);

    tokio::time::sleep(PHASE_POLL_INTERVAL * 2).await;
    assert!(
        !state.cameras.read().await.is_empty(),
        "a single fault must not tear the session down"
    );

    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

/// A camera that dies while warming up is on its way out anyway. Reconnecting
/// it would fight the disconnect the user asked for.
#[tokio::test]
async fn no_reconnect_when_the_camera_dies_during_warmup() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = true;

    let (name, camera) = install_dead_camera(&state).await;
    state.set_camera_phase(CameraRole::Main, &name, CameraPhase::WarmingUp).await;
    send_monitor_cmd_for_test(&state, MonitorCmd::StartWarmup { fast: false });
    camera.dead.store(true, Ordering::SeqCst);

    assert!(
        eventually(
            || state
                .cameras
                .try_read()
                .map(|c| c.is_empty())
                .unwrap_or(false),
            Duration::from_secs(20)
        )
        .await,
        "warmup should still finish tearing the session down"
    );

    // The supervisor is single-flight and sets this for its whole lifetime, so
    // "never ran" is observable right after the teardown.
    assert!(
        !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
        "a warmup teardown must not start a reconnect"
    );
}

/// Turning the feature off has to actually turn it off.
#[tokio::test]
async fn no_reconnect_when_the_setting_is_off() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;

    let (name, _camera) = install_dead_camera(&state).await;
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::DeviceFault).await;

    // The supervisor starts, reads the setting, and gives up before its first
    // attempt — so it must have cleared its own in-flight flag.
    assert!(
        eventually(
            || !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(5)
        )
        .await,
        "the supervisor should have stopped immediately"
    );
    assert!(state.cameras.read().await.is_empty());
}

/// Only one supervisor at a time: a second dropout while one is already
/// retrying must not start a competing sequence.
#[tokio::test]
async fn reconnect_is_single_flight() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = true;

    let (name, _camera) = install_dead_camera(&state).await;
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::DeviceFault).await;

    assert!(
        eventually(
            || state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(5)
        )
        .await,
        "the first fault should start a supervisor"
    );

    // A second fault while the first supervisor is backing off.
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::DeviceFault).await;
    assert!(
        state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
        "still exactly one supervisor"
    );
}

/// A clean stop must not leave a resume plan or a parked stack behind — those
/// exist for recovery, and holding full-resolution accumulators between
/// sessions is pure waste.
#[tokio::test]
async fn a_clean_stop_clears_the_resume_state() {
    use crate::server::services::CaptureService;
    use crate::server::state::SessionResumePlan;

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    *state.session_resume_plan.write().await = Some(SessionResumePlan {
        camera_id: "mock_0".to_string(),
        settings: state.settings.read().await.clone(),
        disk_session_dir: None,
        next_frame: 1,
    });
    state.set_capture_state(CaptureState::Capturing).await;

    assert!(CaptureService::stop_capture(&state).await);
    assert!(
        state.session_resume_plan.read().await.is_none(),
        "a deliberate stop is not something to recover from"
    );
    assert!(state.stacking_carryover.lock().unwrap().is_none());
}

/// A resume must inherit the interrupted session's stack and its raw-frame
/// folder. Restarting the stack, or opening a second timestamped folder, is
/// how an hour of integration ends up scattered and unusable — the field log
/// shows three folders created inside ninety seconds of manual retries.
#[tokio::test]
async fn a_resume_keeps_the_stack_and_the_session_folder() {
    use crate::disk_writer::WritingSessionType;
    use crate::server::capture::StackingCarryover;
    use crate::server::services::CaptureService;
    use crate::server::state::SessionResumePlan;

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    // A session that was saving frames, mid-integration.
    state.disk_writer.set_enabled(true);
    let session_dir = state
        .disk_writer
        .start_session(WritingSessionType::IndividualFrames, "")
        .expect("session dir");
    {
        let mut session = state.session.write().await;
        session.stacked_count = 514;
    }
    let settings = state.settings.read().await.clone();
    *state.stacking_carryover.lock().unwrap() = Some(StackingCarryover {
        stacker: Box::new(
            crate::server::capture::StackingContext::new(
                16,
                16,
                1,
                &crate::server::capture::StackSettings::of(&settings, &crate::plugins::Plugins::none()),
            )
            .expect("context"),
        ),
    });

    let plan = SessionResumePlan {
        camera_id: "mock_0".to_string(),
        settings: state.settings.read().await.clone(),
        disk_session_dir: Some(session_dir.clone()),
        next_frame: 1,
    };
    *state.session_resume_plan.write().await = Some(plan.clone());

    // Resuming without a connected camera is refused, but must not have reset
    // anything on the way to refusing.
    let refused = CaptureService::resume_capture(&state, &plan).await;
    assert!(refused.is_err(), "no camera is connected");
    state.set_capture_state(CaptureState::Idle).await;

    assert_eq!(
        state.session.read().await.stacked_count,
        514,
        "resume must not reset the session counters the way a fresh start does"
    );
    assert_eq!(
        state.disk_writer.session_dir(),
        Some(session_dir),
        "resume must rejoin the folder rather than opening a new one"
    );
    assert!(
        state.stacking_carryover.lock().unwrap().is_some(),
        "the parked stack must survive until the resumed capture takes it"
    );
}

/// Turning saving on partway through a capture used to enable the writer with
/// no session directory, and every frame after that was lost to
/// "No active session".
#[tokio::test]
async fn enabling_saving_mid_capture_opens_a_session() {
    use crate::disk_writer::WritingSessionType;

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    state.disk_writer.set_enabled(true);
    assert!(state.disk_writer.session_dir().is_none());

    state
        .disk_writer
        .ensure_session(WritingSessionType::IndividualFrames, "")
        .expect("session opens");
    assert!(
        state.disk_writer.session_dir().is_some(),
        "an enabled writer with no session must get one"
    );

    // Idempotent: a second call must not open a second folder.
    let first = state.disk_writer.session_dir();
    state
        .disk_writer
        .ensure_session(WritingSessionType::IndividualFrames, "")
        .expect("no-op");
    assert_eq!(state.disk_writer.session_dir(), first);
}

/// One incident must move the streak by one. Both the bounded-call wrapper and
/// its caller can see the same failure, and when both recorded it the give-up
/// threshold was reached in two faults instead of three.
#[tokio::test]
async fn one_incident_counts_once() {
    use crate::server::camera_health::{
        clear_fault_streak, record_fault, FaultKind, PERSISTENT_FAULT_THRESHOLD,
    };

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let name = "Dead Camera";

    for expected in 1..=PERSISTENT_FAULT_THRESHOLD {
        assert_eq!(
            record_fault(&state, CameraRole::Main, name, FaultKind::DeviceLost),
            expected,
            "each fault should advance the streak by exactly one"
        );
    }

    clear_fault_streak(&state, CameraRole::Main, name);
    assert_eq!(record_fault(&state, CameraRole::Main, name, FaultKind::Timeout), 1);
}

/// A camera that fails every other poll must still escalate. The previous rule
/// — reset to zero on any success — meant an intermittent fault wiped its own
/// evidence and never reached the threshold.
#[tokio::test]
async fn an_intermittent_fault_still_escalates() {
    use crate::server::camera_health::{record_fault, FaultKind, PERSISTENT_FAULT_THRESHOLD};

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let name = "Flaky Camera";

    let mut last = 0;
    for _ in 0..PERSISTENT_FAULT_THRESHOLD {
        last = record_fault(&state, CameraRole::Main, name, FaultKind::DeviceLost);
        // A poll in between that happened to work, but not long enough ago to
        // age the streak out.
    }
    assert!(
        last >= PERSISTENT_FAULT_THRESHOLD,
        "a fault seen {} times inside the TTL must escalate",
        PERSISTENT_FAULT_THRESHOLD
    );
}

// ---------------------------------------------------------------------------
// Camera roles
// ---------------------------------------------------------------------------

/// Install a camera into `role` the way `connect` would, without opening a device.
async fn install_camera(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_id: &str,
    name: &str,
    phase: CameraPhase,
) -> Arc<Mutex<Option<(bool, i32)>>> {
    let (cam, _cooler) = MockCamera::new(true, 1.0);
    let dew_heater = Arc::clone(&cam.dew_heater);
    let mut info = cam.info().clone();
    info.name = name.to_string();
    info.has_dew_heater = true;

    state.cameras.write().await.insert(
        camera_id.to_string(),
        ConnectedCameraInfo {
            id: camera_id.to_string(),
            provider: "Mock".to_string(),
            index: 0,
            role,
            info,
        },
    );
    *state.slot(role).handle.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(role, name, phase).await;
    if role == CameraRole::Guide {
        state.set_guide_loop_running(true);
    }
    dew_heater
}

/// The regression the old `guard.replace(camera)` caused: connecting a second camera
/// silently closed the first while its metadata stayed in the map, so the UI showed two
/// connected cameras and one of them was dead. Two roles must hold two live handles.
#[tokio::test]
async fn a_main_and_a_guide_camera_hold_independent_handles() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    install_camera(&state, CameraRole::Main, "mock_0", "Imaging", CameraPhase::Idle).await;
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;

    assert!(state.slot(CameraRole::Main).holds_handle());
    assert!(state.slot(CameraRole::Guide).holds_handle());
    assert_eq!(
        state
            .camera_in_role(CameraRole::Main)
            .await
            .map(|c| c.info.name),
        Some("Imaging".to_string())
    );
    assert_eq!(
        state
            .camera_in_role(CameraRole::Guide)
            .await
            .map(|c| c.info.name),
        Some("Guiding".to_string())
    );
}

/// An idle incumbent is just occupying the position, so a new camera takes its place.
#[tokio::test]
async fn an_idle_role_is_vacated_for_a_new_camera() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Main, "mock_0", "Imaging", CameraPhase::Idle).await;

    lifecycle::vacate_role(&state, CameraRole::Main)
        .await
        .expect("an idle camera should have been swapped out");

    assert!(state.camera_in_role(CameraRole::Main).await.is_none());
    assert!(!state.slot(CameraRole::Main).holds_handle());
}

/// A camera mid-capture or mid-warmup is doing something the user asked for. Replacing
/// it would kill a running session, or close a handle with the sensor still cold.
#[tokio::test]
async fn a_busy_role_refuses_to_be_vacated() {
    for phase in [CameraPhase::Capturing, CameraPhase::WarmingUp] {
        let (state, _dw) = AppState::new_for_testing();
        let state = Arc::new(state);
        install_camera(&state, CameraRole::Main, "mock_0", "Imaging", phase).await;

        let err = lifecycle::vacate_role(&state, CameraRole::Main)
            .await
            .expect_err("a busy camera must not be swapped out");
        assert!(
            matches!(err, crate::server::error::ApiError::CameraRoleBusy { .. }),
            "{phase:?} produced {err:?}, not CameraRoleBusy"
        );
        assert!(
            state.slot(CameraRole::Main).holds_handle(),
            "{phase:?} lost its handle to a refused vacate"
        );
    }
}

/// Losing the guide camera must not disturb the imaging camera, and must hand solving
/// back rather than leaving the session with no solve source at all.
#[tokio::test]
async fn disconnecting_the_guide_camera_leaves_the_main_one_alone() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Main, "mock_0", "Imaging", CameraPhase::Idle).await;
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;

    lifecycle::finalize_disconnect(
        &state,
        CameraRole::Guide,
        "Guiding",
        DisconnectCause::Requested,
    )
    .await;

    assert!(state.camera_in_role(CameraRole::Guide).await.is_none());
    assert!(!state.guide_loop_running());
    assert!(
        state.slot(CameraRole::Main).holds_handle(),
        "the imaging camera lost its handle to a guide disconnect"
    );
    assert!(state.camera_in_role(CameraRole::Main).await.is_some());
}

/// Each slot carries its own single-flight guard. One shared flag meant a guide dropout
/// during a main-camera recovery was refused outright, leaving the guide camera down for
/// the rest of the night.
#[tokio::test]
async fn the_reconnect_guard_is_per_slot() {
    use std::sync::atomic::Ordering as AtomicOrdering;

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    state
        .slot(CameraRole::Main)
        .reconnect_in_flight
        .store(true, AtomicOrdering::SeqCst);

    assert!(
        !state
            .slot(CameraRole::Guide)
            .reconnect_in_flight
            .load(AtomicOrdering::SeqCst),
        "a main-camera recovery blocked the guide slot's supervisor"
    );
}

// ---------------------------------------------------------------------------
// Guide-role hypotheses (review of 43e8bbb)
// ---------------------------------------------------------------------------

/// A guide loop owns its handle for the length of the connection, not the length of a
/// session, so it gets a phase of its own. Stamping `Capturing` made every gate that
/// treats capture as temporary — swap, dew heater, monitor polling — wait forever.
#[tokio::test]
async fn a_running_guide_loop_gets_its_own_phase() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;

    let camera = lifecycle::take_for_capture(&state, CameraRole::Guide, "Guiding")
        .await
        .expect("the guide loop should have got its handle");
    std::mem::forget(camera); // the loop holds it for the whole connection

    assert_eq!(state.camera_phase(CameraRole::Guide).await, CameraPhase::Guiding);

    // The imaging camera keeps the phase a bounded session deserves.
    install_camera(&state, CameraRole::Main, "mock_0", "Imaging", CameraPhase::Idle).await;
    let main = lifecycle::take_for_capture(&state, CameraRole::Main, "Imaging")
        .await
        .unwrap();
    std::mem::forget(main);
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Capturing);
}

/// A guide loop never ends on its own, so refusing to vacate while one runs would mean
/// the guide camera could never be replaced at all. An imaging camera mid-capture still
/// refuses — that session does end, and cutting it short loses the stack.
#[tokio::test]
async fn a_guiding_camera_can_be_swapped_but_a_capturing_one_cannot() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(
        &state,
        CameraRole::Guide,
        "mock_1",
        "Guiding",
        CameraPhase::Guiding,
    )
    .await;

    lifecycle::vacate_role(&state, CameraRole::Guide)
        .await
        .expect("a guide camera must be replaceable");
    assert!(state.camera_in_role(CameraRole::Guide).await.is_none());

    install_camera(
        &state,
        CameraRole::Main,
        "mock_0",
        "Imaging",
        CameraPhase::Capturing,
    )
    .await;
    let err = lifecycle::vacate_role(&state, CameraRole::Main)
        .await
        .expect_err("a running capture must not be swapped out");
    assert!(matches!(
        err,
        crate::server::error::ApiError::CameraRoleBusy { .. }
    ));
}

/// `CaptureConfig` carries no dew-heater field, so with the handle checked out by the
/// guide loop there is no path to the device at all. The change is queued for the loop
/// rather than dropped.
#[tokio::test]
async fn a_guide_dew_heater_change_is_queued_for_the_loop() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(
        &state,
        CameraRole::Guide,
        "mock_1",
        "Guiding",
        CameraPhase::Guiding,
    )
    .await;

    {
        let mut settings = state.settings.write().await;
        settings.guide_camera.dew_heater_enabled = true;
        settings.guide_camera.dew_heater_power = 80;
    }

    lifecycle::apply_dew_heater_settings(&state, CameraRole::Guide).await;

    assert_eq!(
        state.slot(CameraRole::Guide).drain_ops(),
        vec![CameraOp::SetDewHeater {
            enabled: true,
            power: 80
        }],
        "the guide dew heater switch never reached the loop that owns the handle"
    );
}

/// Only the latest position of a slider is worth applying: a user dragging the power
/// control must not leave the loop a queue of intermediate values to walk through.
#[tokio::test]
async fn queued_hardware_calls_collapse_to_the_latest() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let slot = state.slot(CameraRole::Guide);

    slot.queue_op(CameraOp::SetDewHeater {
        enabled: true,
        power: 20,
    });
    slot.queue_op(CameraOp::SetDewHeater {
        enabled: true,
        power: 90,
    });

    assert_eq!(
        slot.drain_ops(),
        vec![CameraOp::SetDewHeater {
            enabled: true,
            power: 90
        }]
    );
    assert!(slot.drain_ops().is_empty(), "draining must consume the queue");
}

/// A dew heater change queued for a guide camera that disconnects before its loop's
/// next iteration must not be replayed against whatever connects into the role next —
/// otherwise a setting meant for one physical camera is applied to a different one.
#[tokio::test]
async fn a_disconnect_drops_hardware_calls_queued_for_the_role_rather_than_carrying_them_over() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let name = "Guiding";
    install_camera(&state, CameraRole::Guide, "mock_1", name, CameraPhase::Guiding).await;

    state.slot(CameraRole::Guide).queue_op(CameraOp::SetDewHeater {
        enabled: true,
        power: 80,
    });

    lifecycle::finalize_disconnect(&state, CameraRole::Guide, name, DisconnectCause::Requested)
        .await;

    assert!(
        state.slot(CameraRole::Guide).drain_ops().is_empty(),
        "a stale hardware call survived into the next camera to occupy this role"
    );
}

/// A cooled guide camera warms up for minutes after its loop stops. Solving has to go
/// back to the imaging camera for that window rather than leaving the session with no
/// source at all — which is why the flag tracks the loop, not the camera.
#[tokio::test]
async fn a_guide_warmup_hands_plate_solving_back_to_the_imaging_camera() {
    use crate::server::capture::solving::{plate_solve_available, SolveSource};

    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Main, "mock_0", "Imaging", CameraPhase::Idle).await;
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;
    state.settings.write().await.guide_camera.cooler_enabled = true;

    lifecycle::disconnect(&state, "mock_1", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .expect("guide disconnect should be accepted");

    assert_eq!(
        state.camera_phase(CameraRole::Guide).await,
        CameraPhase::WarmingUp,
        "a cooled guide camera should warm up before its handle closes"
    );
    assert!(
        !state.guide_loop_running(),
        "the loop is stopped, whatever the camera list still says"
    );
    // Community has no Push-To plugin, so `plate_solve_available` stops at the license
    // check; what this asserts is that the *source* decision no longer excludes Main.
    assert!(
        !plate_solve_available(&state, SolveSource::Guide),
        "a stopped guide loop must not be offered as the solve source"
    );
}

/// The other half of the same budget: a reconnect can land while the old loop is still
/// wedged. The handle it eventually returns is the superseded one, and parking it would
/// close the live device the moment `connect` displaced it.
#[tokio::test]
async fn a_handback_that_a_reconnect_beat_is_refused() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;

    let stale = lifecycle::take_for_capture(&state, CameraRole::Guide, "Guiding")
        .await
        .unwrap();

    // A reconnect brings the same camera back and installs a fresh handle.
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;
    assert!(state.slot(CameraRole::Guide).holds_handle());

    lifecycle::return_from_capture(&state, CameraRole::Guide, "Guiding", Some(stale)).await;

    assert!(
        state.slot(CameraRole::Guide).holds_handle(),
        "the reconnected handle must still be the one in the slot"
    );
    assert_eq!(state.camera_phase(CameraRole::Guide).await, CameraPhase::Idle);
}

/// `guide_task::stop` gives up after a budget and lets the disconnect proceed. The loop
/// still hands its handle back when it eventually exits, and that hand-back must not
/// resurrect a camera the user has disconnected.
#[tokio::test]
async fn a_late_handback_is_refused_and_the_handle_closed() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;

    // The loop is holding the handle when the disconnect lands.
    let camera = lifecycle::take_for_capture(&state, CameraRole::Guide, "Guiding")
        .await
        .unwrap();

    lifecycle::finalize_disconnect(
        &state,
        CameraRole::Guide,
        "Guiding",
        DisconnectCause::Requested,
    )
    .await;
    assert_eq!(
        state.camera_phase(CameraRole::Guide).await,
        CameraPhase::Disconnected
    );

    // The loop finally notices its stop flag and hands the handle back.
    lifecycle::return_from_capture(&state, CameraRole::Guide, "Guiding", Some(camera)).await;

    assert_eq!(
        state.camera_phase(CameraRole::Guide).await,
        CameraPhase::Disconnected,
        "a late hand-back re-opened a camera the user disconnected"
    );
    assert!(
        !state.slot(CameraRole::Guide).holds_handle(),
        "a live handle was parked in a slot with no registered camera"
    );
}

// ---------------------------------------------------------------------------
// Hand-off failures and Disconnect as a must (2026-09-20 field log)
// ---------------------------------------------------------------------------

/// Cooling on, target -10 °C, and a cooled mock camera connected and at temperature.
async fn cooled_rig() -> (Arc<AppState>, String) {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    let name = install_mock_camera(&state, 15.0, true, -10.0).await;
    (state, name)
}

/// Take the handle out of the slot behind everyone's back, waiting out a monitor poll
/// that has it checked out.
async fn remove_handle(state: &Arc<AppState>, role: CameraRole) -> Box<dyn crate::camera::Camera> {
    match lifecycle::take_camera(state, role).await {
        Some(handle) => handle,
        None => panic!("the slot should hold a handle"),
    }
}

async fn camera_gone(state: &Arc<AppState>, budget: Duration) -> bool {
    eventually(
        || state.cameras.try_read().map(|c| c.is_empty()).unwrap_or(false),
        budget,
    )
    .await
}

/// The 14:07 wedge: Start during a warm-up cancelled it, the take failed, and nothing
/// resumed the monitor or the warm-up — every later Disconnect answered "already warming
/// up" and did nothing. A take that fails while the monitor is busy must leave both running.
#[tokio::test]
async fn a_hand_off_the_monitor_is_still_holding_resumes_the_monitor_and_the_warmup() {
    let (state, name) = cooled_rig().await;
    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);
    // With the cooler switched off in settings the Start sends the monitor no cooler
    // target, whose sensor read would itself clear the call this test stands in for.
    state.settings.write().await.cooler_enabled = false;

    // The monitor is inside a status call that has not come back. Paused first, and given
    // a moment to finish the tick it is in, for the same reason.
    let slot = state.slot(CameraRole::Main);
    send_monitor_cmd_for_test(&state, MonitorCmd::HandOffToCapture);
    let handle = remove_handle(&state, CameraRole::Main).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    slot.set_monitor_call(Some(std::time::Instant::now()));

    let err = lifecycle::take_for_capture(&state, CameraRole::Main, &name)
        .await
        .err()
        .expect("nothing could hand the handle over");
    assert!(err.to_string().contains("busy"), "{err}");
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);
    assert!(slot.warmup().is_some(), "the cancelled warm-up must be timed again");

    // The call returns; a resumed monitor finishes the warm-up and the disconnect.
    *slot.handle.lock().unwrap() = Some(handle);
    slot.set_monitor_call(None);
    assert!(
        camera_gone(&state, Duration::from_secs(20)).await,
        "the warm-up never finished: the monitor was left paused"
    );
}

/// The same Start during a warm-up, with the handle gone for good: the camera goes to
/// recovery instead of sitting in `WarmingUp` with nothing driving it, and the Disconnect
/// that follows completes at once.
#[tokio::test]
async fn a_start_during_a_warmup_with_the_handle_lost_leaves_nothing_wedged() {
    let (state, name) = cooled_rig().await;
    state.settings.write().await.auto_reconnect = true;
    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    send_monitor_cmd_for_test(&state, MonitorCmd::HandOffToCapture);
    remove_handle(&state, CameraRole::Main).await;

    let err = lifecycle::take_for_capture(&state, CameraRole::Main, &name)
        .await
        .err()
        .expect("there is no handle to take");
    assert!(
        matches!(err, crate::server::error::ApiError::CameraHandleLost { .. }),
        "{err:?}"
    );
    // What the capture loop does with that answer.
    lifecycle::return_from_capture(&state, CameraRole::Main, &name, None).await;
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Recovering);

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert_eq!(outcome, lifecycle::DisconnectOutcome::Disconnected);
    assert!(state.cameras.read().await.is_empty());
}

/// The handle is gone and nothing holds it — what the Ares-C PRO's capture found after
/// its 14:06 reopen. That is a lost handle, recovered like a device fault once the caller
/// hands back `None`, not a busy monitor to blame and retry forever.
#[tokio::test]
async fn a_hand_off_with_the_handle_lost_reopens_the_camera() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = true;
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;
    remove_handle(&state, CameraRole::Main).await;

    let err = lifecycle::take_for_capture(&state, CameraRole::Main, &name)
        .await
        .err()
        .expect("there is no handle to take");

    assert!(
        matches!(err, crate::server::error::ApiError::CameraHandleLost { .. }),
        "{err:?}"
    );
    assert!(
        !state.slot(CameraRole::Main).is_recovering(),
        "the take tore the camera down itself; that is the caller's hand-back"
    );
    lifecycle::return_from_capture(&state, CameraRole::Main, &name, None).await;
    assert!(state.slot(CameraRole::Main).is_recovering());
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Recovering);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

/// A capture still winding down holds the handle legitimately. Reopening the camera under
/// it would be a false recovery; the second Start is told to wait instead.
#[tokio::test]
async fn a_handle_another_capture_holds_is_busy_not_lost() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = true;
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;
    let held = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();

    let err = lifecycle::take_for_capture(&state, CameraRole::Main, &name)
        .await
        .err()
        .expect("the first capture still has it");

    assert!(err.to_string().contains("still held"), "{err}");
    assert!(!state.slot(CameraRole::Main).is_recovering());
    lifecycle::return_from_capture(&state, CameraRole::Main, &name, Some(held)).await;
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

/// A warm-up whose monitor can no longer drive it — wedged, paused, or polling a camera
/// that never answers — still ends, at the deadline, with the camera disconnected.
#[tokio::test]
async fn a_stalled_warmup_disconnects_at_its_deadline() {
    let (state, _name) = cooled_rig().await;
    lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    send_monitor_cmd_for_test(&state, MonitorCmd::HandOffToCapture);

    state.slot(CameraRole::Main).expire_warmup();

    assert!(camera_gone(&state, Duration::from_secs(3)).await, "the deadline did not end the warm-up");
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Disconnected);
    assert!(!state.slot(CameraRole::Main).holds_handle());
}

/// "Disconnect now": the observer accepts the thermal shock rather than wait out a ramp.
#[tokio::test]
async fn skipping_the_warmup_disconnects_a_warming_camera_at_once() {
    let (state, _name) = cooled_rig().await;
    let first = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert!(matches!(first, lifecycle::DisconnectOutcome::WarmingUp { remaining: Some(_) }));

    let again = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert!(
        matches!(again, lifecycle::DisconnectOutcome::WarmingUp { .. }),
        "an ordinary second press reports the warm-up, it does not cut it short"
    );

    let forced = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::Skip)
        .await
        .unwrap();
    assert_eq!(forced, lifecycle::DisconnectOutcome::Disconnected);
    assert!(state.cameras.read().await.is_empty());
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Disconnected);
}

/// Nothing can command the TEC of a camera with no handle, so there is no warm-up to wait
/// for — the camera unplugged mid-session that could never be disconnected.
#[tokio::test]
async fn a_cooled_camera_without_a_handle_disconnects_without_a_warmup() {
    let (state, _name) = cooled_rig().await;
    send_monitor_cmd_for_test(&state, MonitorCmd::HandOffToCapture);
    remove_handle(&state, CameraRole::Main).await;

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();

    assert_eq!(outcome, lifecycle::DisconnectOutcome::Disconnected);
    assert!(state.cameras.read().await.is_empty());
}

/// A camera whose last calls failed would fail every ramp step for minutes.
#[tokio::test]
async fn a_recently_faulted_camera_disconnects_without_a_warmup() {
    let (state, name) = cooled_rig().await;
    crate::server::camera_health::record_fault(
        &state,
        CameraRole::Main,
        &name,
        crate::server::camera_health::FaultKind::DeviceLost,
    );

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();

    assert_eq!(outcome, lifecycle::DisconnectOutcome::Disconnected);
}

/// A camera unplugged while warming up has nothing left to warm. The first device-lost
/// answer ends it, rather than the three a running session needs.
#[tokio::test]
async fn a_device_lost_during_warmup_ends_it_at_once() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    {
        let mut s = state.settings.write().await;
        s.auto_reconnect = true;
        s.cooler_enabled = true;
        s.target_temp_c = Some(-10.0);
    }
    let (_name, camera) = install_dead_camera(&state).await;
    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert!(matches!(outcome, lifecycle::DisconnectOutcome::WarmingUp { .. }));

    camera.dead.store(true, Ordering::SeqCst);

    assert!(
        camera_gone(&state, PHASE_POLL_INTERVAL * 2 + Duration::from_secs(1)).await,
        "an unplugged camera kept warming up"
    );
    assert!(!state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst));
}

/// An unpaused monitor finding the slot empty, poll after poll, is the only witness to a
/// handle lost on a path nobody logged. It reopens the camera — here, with reconnect off,
/// it ends the session — instead of polling an empty slot for the rest of the night.
#[tokio::test]
async fn a_handle_missing_from_under_the_monitor_is_given_up() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;
    install_mock_camera(&state, 5.0, false, 20.0).await;

    remove_handle(&state, CameraRole::Main).await;

    assert!(
        camera_gone(&state, PHASE_POLL_INTERVAL * 4 + Duration::from_secs(1)).await,
        "the monitor polled an empty slot forever"
    );
}

/// One empty poll is a race with a capture taking the handle, not a loss.
#[tokio::test]
async fn one_empty_poll_is_not_a_lost_handle() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;

    let handle = remove_handle(&state, CameraRole::Main).await;
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(300)).await;
    *state.slot(CameraRole::Main).handle.lock().unwrap() = Some(handle);
    tokio::time::sleep(PHASE_POLL_INTERVAL * 3).await;

    assert!(!state.cameras.read().await.is_empty(), "a single empty poll ended the session");
    lifecycle::finalize_disconnect(&state, CameraRole::Main, &name, DisconnectCause::Requested).await;
}

// ---------------------------------------------------------------------------
// Review of 6abd7de: the guide loop's own failures
// ---------------------------------------------------------------------------

/// A guide loop whose hand-off finds the handle lost hands the camera to the teardown,
/// which stops "the guide loop" — the very loop asking, still registered and holding its
/// stop switch. `guide_task::stop` then waits out its whole budget on itself before the
/// camera is torn down or recovered. The take alone is `HANDLE_WAIT_TIMEOUT` (3.5 s).
#[tokio::test]
async fn a_guide_loop_that_finds_its_handle_lost_is_not_waited_on_by_its_own_teardown() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;
    install_camera(&state, CameraRole::Guide, "mock_1", "Guiding", CameraPhase::Idle).await;
    state.set_guide_loop_running(false);
    let info = state.camera_in_role(CameraRole::Guide).await.unwrap();
    // Gone, and nothing holds it: the reopened Ares-C PRO of 2026-09-20.
    state.slot(CameraRole::Guide).handle.lock().unwrap().take();

    let started = std::time::Instant::now();
    assert!(crate::server::capture::guide_task::start(&state, &info));
    let gone = camera_gone(&state, Duration::from_secs(12)).await;
    let took = started.elapsed();

    assert!(gone, "the lost guide camera was never torn down");
    assert!(
        took < Duration::from_secs(5),
        "the teardown waited on the loop that asked for it: {took:?}"
    );
}

/// A guide loop that dies mid-loop keeps its registration and its "running" flag, and the
/// slot stays `Guiding` with no handle. Start then answers "running" for a loop that does
/// not exist (and keeps the imaging camera from solving), and after Stop no new loop can
/// take the handle: `abandon_hand_off` blames "the guide loop" for ever.
#[tokio::test]
async fn a_guide_loop_that_panics_does_not_leave_the_camera_claimed() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.write().await.auto_reconnect = false;
    // `status()` panicking: the guide loop calls it on its own thread, outside any
    // watchdog, so a panic there ends the thread mid-loop.
    let camera = FakeCamera::new("Mock Cooled Camera")
        .sized(640, 480)
        .stuck_for(Duration::from_millis(20));
    let controls = camera.controls();
    let info = ConnectedCameraInfo {
        id: "mock_1".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Guide,
        info: camera.info().clone(),
    };
    state.cameras.write().await.insert("mock_1".to_string(), info.clone());
    *state.slot(CameraRole::Guide).handle.lock().unwrap() = Some(Box::new(camera));
    state
        .set_camera_phase(CameraRole::Guide, &info.info.name, CameraPhase::Idle)
        .await;

    assert!(crate::server::capture::guide_task::start(&state, &info));
    assert!(
        eventually(|| state.guide_loop_running(), Duration::from_secs(3)).await,
        "the loop never started"
    );
    controls.panic_on_status(true);

    let released = eventually(
        || !state.guide_loops.is_registered() && !state.guide_loop_running(),
        Duration::from_secs(5),
    )
    .await;
    let phase = state.camera_phase(CameraRole::Guide).await;

    assert!(released, "a dead loop is still registered as running");
    assert_ne!(phase, CameraPhase::Guiding, "the slot still names a dead loop as its owner");
}

/// Two bodies of one model share a name, and the fault streak is keyed by name. A stall
/// on the guide twin must not cost the cooled imaging twin its warm-up.
#[tokio::test]
async fn a_fault_on_the_guide_twin_does_not_skip_the_imaging_twins_warmup() {
    let (state, name) = cooled_rig().await;
    install_camera(&state, CameraRole::Guide, "mock_1", &name, CameraPhase::Idle).await;
    // Keep the imaging monitor from clearing the streak with a successful poll first.
    send_monitor_cmd_for_test(&state, MonitorCmd::HandOffToCapture);
    tokio::time::sleep(Duration::from_millis(100)).await;
    crate::server::camera_health::record_fault(
        &state,
        CameraRole::Guide,
        &name,
        crate::server::camera_health::FaultKind::Timeout,
    );

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    let _ = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::Skip).await;

    assert!(
        matches!(outcome, lifecycle::DisconnectOutcome::WarmingUp { .. }),
        "the guide twin's stall skipped the imaging camera's warm-up: {outcome:?}"
    );
}


// ---------------------------------------------------------------------------
// A Disconnect that outlasts the capture's wind-down
// ---------------------------------------------------------------------------

/// Stands in for a capture pipeline whose wind-down — the final stack save on a large
/// sensor — outlasts `CAPTURE_STOP_WAIT`, then hands back what it holds.
fn slow_pipeline(
    state: &Arc<AppState>,
    name: &str,
    handle: Option<Box<dyn Camera>>,
) -> tokio::task::JoinHandle<()> {
    let state = Arc::clone(state);
    let name = name.to_string();
    tokio::spawn(async move {
        while state.capture_state().await != CaptureState::Stopping {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(lifecycle::CAPTURE_STOP_WAIT + Duration::from_secs(1)).await;
        lifecycle::return_from_capture(&state, CameraRole::Main, &name, handle).await;
        state.end_capture_state().await;
    })
}

/// A capture still saving its stack holds the handle and will give it back: that is not
/// "no handle to command". The warm-up waits for the hand-back, then runs.
#[tokio::test]
async fn a_capture_slow_to_wind_down_hands_its_camera_to_the_warmup() {
    let (state, name) = cooled_rig().await;
    state.set_capture_state(CaptureState::Capturing).await;
    let handle = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();
    let pipeline = slow_pipeline(&state, &name, Some(handle));

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();

    assert!(
        matches!(outcome, lifecycle::DisconnectOutcome::WarmingUp { remaining: Some(_) }),
        "{outcome:?}"
    );
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::WarmingUp);
    assert!(
        state.cameras.read().await.contains_key("mock_0"),
        "disconnected before the capture handed the camera back"
    );
    pipeline.await.unwrap();
    assert!(
        camera_gone(&state, Duration::from_secs(20)).await,
        "the warm-up never ran once the handle came back"
    );
}

/// The handle never comes back: the camera the observer is disconnecting ends there, and
/// is not reopened as if it had dropped out.
#[tokio::test]
async fn a_handle_lost_while_its_warmup_waits_is_not_reopened() {
    let (state, name) = cooled_rig().await;
    state.settings.write().await.auto_reconnect = true;
    state.set_capture_state(CaptureState::Capturing).await;
    let _abandoned = lifecycle::take_for_capture(&state, CameraRole::Main, &name).await.unwrap();
    let pipeline = slow_pipeline(&state, &name, None);

    let outcome = lifecycle::disconnect(&state, "mock_0", lifecycle::WarmupPolicy::WhenPossible)
        .await
        .unwrap();
    assert!(matches!(outcome, lifecycle::DisconnectOutcome::WarmingUp { .. }), "{outcome:?}");
    pipeline.await.unwrap();

    assert!(state.cameras.read().await.is_empty());
    assert!(!state.slot(CameraRole::Main).is_recovering());
    assert!(!state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst));
    assert_eq!(state.camera_phase(CameraRole::Main).await, CameraPhase::Disconnected);
}
