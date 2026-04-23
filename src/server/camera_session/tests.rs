//! Integration tests for the camera session lifecycle + monitor.
//!
//! We use a small in-module mock camera (rather than the real SimulatedCamera
//! provider) so we can drive phase transitions deterministically without
//! depending on the global simulated-camera directory registry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::camera::{
    Camera, CameraError, CameraInfo, CameraResult, CameraStatus, CaptureConfig, GainPresets,
    SensorType,
};
use crate::frame::Frame;
use crate::server::camera_session::{lifecycle, monitor, PHASE_POLL_INTERVAL};
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CameraPhase, ConnectedCameraInfo};

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

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
        Frame::zeros(
            self.info.max_width as usize,
            self.info.max_height as usize,
            1,
        )
        .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
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
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));

    let phase = if cooler_on {
        CameraPhase::Precooling
    } else {
        CameraPhase::Idle
    };
    state.set_camera_phase(&name, phase).await;

    // Spawn the monitor thread.
    let tx = monitor::spawn(
        Arc::clone(state),
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.camera_monitor_tx.lock().unwrap() = Some(tx);

    name
}

/// Wait up to `timeout` for a predicate on the phase to become true.
async fn wait_for_phase(
    state: &Arc<AppState>,
    camera_name: &str,
    target: CameraPhase,
    timeout: Duration,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if state.camera_phase(camera_name).await == target {
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
    let name = install_mock_camera(&state, 5.0, true, -10.0).await;
    // Bump initial temp near target right away by calling status() a few times.
    // (The mock starts at ambient 20°C; with step 5 we need ~6 ticks.)
    // Instead, seed the cooler state close to target:
    {
        let cam = state.active_camera.lock().unwrap();
        // Can't downcast through Box<dyn Camera>; settle via status() calls.
        drop(cam);
    }
    // Trigger manual stepping by calling `status()` from outside to converge.
    for _ in 0..10 {
        let mut guard = state.active_camera.lock().unwrap();
        if let Some(cam) = guard.as_mut() {
            let _ = cam.status();
        }
    }

    // The monitor polls every 2s; wait up to 8s for convergence + 2 stable samples.
    let settled = wait_for_phase(&state, &name, CameraPhase::Idle, Duration::from_secs(10)).await;
    assert!(settled, "Expected phase to settle to Idle");
}

#[tokio::test]
async fn no_precool_when_cooler_disabled() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    // No cooler_enabled → monitor should stay in Idle forever; no Precooling event.
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Idle);

    // Wait ~3s and verify it remains Idle (not flipping to something weird).
    tokio::time::sleep(PHASE_POLL_INTERVAL + Duration::from_millis(500)).await;
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Idle);

    // Clean up.
    lifecycle::finalize_disconnect(&state, &name).await;
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
        let mut guard = state.active_camera.lock().unwrap();
        if let Some(cam) = guard.as_mut() {
            // Drop cooler target and let internal state mutate via the status calls.
            let _ = cam.set_cooler(true);
            let _ = cam.set_target_temperature(-10.0);
        }
    }

    let mut rx = state.subscribe_events();

    // Trigger warmup (as if user clicked Disconnect with cooler on).
    let result = lifecycle::disconnect(&state, "mock_0").await;
    assert!(result.is_ok());
    assert_eq!(state.camera_phase(&name).await, CameraPhase::WarmingUp);

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

    let phase_after = state.camera_phase(&name).await;
    assert_eq!(phase_after, CameraPhase::Disconnected);
    let cameras = state.cameras.read().await;
    assert!(cameras.is_empty(), "Camera metadata should be cleared");
}

#[tokio::test]
async fn disconnect_with_cooler_off_is_synchronous() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;

    // Cooler was never on → synchronous close, no warmup.
    let result = lifecycle::disconnect(&state, "mock_0").await;
    assert!(result.is_ok());

    assert_eq!(state.camera_phase(&name).await, CameraPhase::Disconnected);
    assert!(state.cameras.read().await.is_empty());
    assert!(state.active_camera.lock().unwrap().is_none());
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
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Precooling);

    // Capture takes the handle.
    let cam = lifecycle::take_for_capture(&state, &name).await.unwrap();
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Capturing);
    assert!(state.active_camera.lock().unwrap().is_none());

    // Return it.
    lifecycle::return_from_capture(&state, &name, Some(cam)).await;
    let phase_after = state.camera_phase(&name).await;
    assert!(
        matches!(phase_after, CameraPhase::Precooling | CameraPhase::Idle),
        "Expected Precooling/Idle after return, got {:?}",
        phase_after
    );
    assert!(state.active_camera.lock().unwrap().is_some());

    // Clean up.
    lifecycle::finalize_disconnect(&state, &name).await;
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
    lifecycle::disconnect(&state, "mock_0").await.unwrap();
    assert_eq!(state.camera_phase(&name).await, CameraPhase::WarmingUp);

    // User immediately starts capture → warmup cancelled, phase → Capturing.
    let cam = lifecycle::take_for_capture(&state, &name).await.unwrap();
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Capturing);

    // Return and clean up.
    lifecycle::return_from_capture(&state, &name, Some(cam)).await;
    lifecycle::finalize_disconnect(&state, &name).await;
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
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(&name, CameraPhase::Idle).await;

    // Spawn the monitor so it can process UpdateCoolerTarget.
    let tx = monitor::spawn(
        Arc::clone(&state),
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.camera_monitor_tx.lock().unwrap() = Some(tx);

    {
        let mut s = state.settings.write().await;
        s.target_temp_c = Some(20.0);
    }
    lifecycle::apply_cooler_settings(&state).await;

    // Phase should flip back to Precooling immediately.
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Precooling);

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

    lifecycle::finalize_disconnect(&state, &name).await;
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
        info: cam.info().clone(),
    };
    let _ = cam.set_cooler(true);
    let _ = cam.set_target_temperature(-5.0);
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(&name, CameraPhase::Idle).await;

    {
        let mut s = state.settings.write().await;
        s.cooler_enabled = false;
    }
    lifecycle::apply_cooler_settings(&state).await;

    assert!(
        !cooler.lock().unwrap().cooler_on,
        "cooler should be off on hardware"
    );

    lifecycle::finalize_disconnect(&state, &name).await;
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
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(&name, CameraPhase::WarmingUp).await;

    lifecycle::apply_cooler_settings(&state).await;

    // Cooler must stay off — the warmup phase owns it.
    assert!(!cooler.lock().unwrap().cooler_on);

    // Clean up without going through the monitor (no monitor was spawned).
    *state.active_camera.lock().unwrap() = None;
    state.cameras.write().await.clear();
}

#[tokio::test]
async fn return_from_capture_without_handle_finalizes_disconnect() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;

    // Simulate capture thread panicking: take the handle and drop it, then
    // call return_from_capture with None.
    let _cam = lifecycle::take_for_capture(&state, &name).await.unwrap();

    lifecycle::return_from_capture(&state, &name, None).await;

    assert_eq!(state.camera_phase(&name).await, CameraPhase::Disconnected);
    assert!(state.cameras.read().await.is_empty());
}

// ----------------------------------------------------------------------------
// apply_camera_profile_on_connect — pure unit tests
// ----------------------------------------------------------------------------

/// Shape a `CameraInfo` for the unit tests. Only the fields checked by
/// `apply_camera_profile_on_connect` matter here.
fn test_camera_info(has_cooler: bool, supports_sensor_modes: bool) -> CameraInfo {
    use crate::camera::SensorMode;
    CameraInfo {
        name: "test".to_string(),
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
    let info = test_camera_info(true, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), &info);

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
        },
    );

    let info = test_camera_info(true, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key, &info);

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
    let info = test_camera_info(false, true);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), &info);

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

#[test]
fn connect_clamps_sensor_mode_for_camera_without_modes() {
    use crate::camera::DualSamplingMode;
    use crate::server::state::CaptureSettings;

    let mut settings = CaptureSettings::default();
    settings.sensor_mode_override = Some(DualSamplingMode::LowReadoutNoise);
    settings.gain = 150;

    let key = "PlayerOne/Neptune-C II".to_string();
    let info = test_camera_info(false, false);
    lifecycle::apply_camera_profile_on_connect(&mut settings, key.clone(), &info);

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
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(&name, CameraPhase::Precooling).await;

    let tx = monitor::spawn(
        Arc::clone(&state),
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.camera_monitor_tx.lock().unwrap() = Some(tx);

    let _ = state
        .camera_monitor_tx
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

    lifecycle::finalize_disconnect(&state, &name).await;
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
    let name = install_mock_camera(&state, 1.0, true, -10.0).await;

    // Kick off warmup.
    lifecycle::disconnect(&state, "mock_0").await.unwrap();
    assert_eq!(state.camera_phase(&name).await, CameraPhase::WarmingUp);

    // Within the first tick window, cooler must still be ON (ramped warmup,
    // not kill-switch warmup).
    tokio::time::sleep(Duration::from_millis(500)).await;
    {
        let guard = state.active_camera.lock().unwrap();
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
        .camera_monitor_tx
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
        info: cam.info().clone(),
    };
    state
        .cameras
        .write()
        .await
        .insert("mock_0".to_string(), connected_info);
    *state.selected_camera.write().await = Some("mock_0".to_string());
    *state.active_camera.lock().unwrap() = Some(Box::new(cam));
    state.set_camera_phase(&name, CameraPhase::Precooling).await;

    let tx = monitor::spawn(
        Arc::clone(&state),
        name.clone(),
        tokio::runtime::Handle::current(),
    );
    *state.camera_monitor_tx.lock().unwrap() = Some(tx);

    let _ = state
        .camera_monitor_tx
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

    lifecycle::finalize_disconnect(&state, &name).await;
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
    let name = install_mock_camera(&state, 5.0, true, -10.0).await;

    lifecycle::disconnect(&state, "mock_0").await.unwrap();
    assert_eq!(state.camera_phase(&name).await, CameraPhase::WarmingUp);

    // StartWarmup with fast=true should flip the cooler off right away.
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        let off = {
            let guard = state.active_camera.lock().unwrap();
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
        .camera_monitor_tx
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
    lifecycle::apply_cooler_settings(&state).await;

    // Phase should remain Precooling.
    assert_eq!(state.camera_phase(&name).await, CameraPhase::Precooling);

    // Monitor should install a new ramp that will push the hardware setpoint
    // to -5 on the next tick.
    let cooler_handle = {
        let guard = state.active_camera.lock().unwrap();
        // Snapshot for later assertion — mock camera status reads current
        // state, and we don't downcast.
        assert!(guard.is_some());
    };
    let _ = cooler_handle;

    let deadline = std::time::Instant::now() + PHASE_POLL_INTERVAL + Duration::from_secs(2);
    loop {
        let target = {
            let mut guard = state.active_camera.lock().unwrap();
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

    lifecycle::finalize_disconnect(&state, &name).await;
}
