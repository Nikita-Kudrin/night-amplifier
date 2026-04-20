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
        let goal = if c.cooler_on { c.target_temp_c } else { c.ambient_c };
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
        Frame::zeros(self.info.max_width as usize, self.info.max_height as usize, 1)
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
    let (state, _dw) = AppState::new_for_testing(5, 85);
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
    let (state, _dw) = AppState::new_for_testing(5, 85);
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
    let (state, _dw) = AppState::new_for_testing(5, 85);
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

    assert!(saw_disconnect, "Expected CameraDisconnected event after warmup");

    let phase_after = state.camera_phase(&name).await;
    assert_eq!(phase_after, CameraPhase::Disconnected);
    let cameras = state.cameras.read().await;
    assert!(cameras.is_empty(), "Camera metadata should be cleared");
}

#[tokio::test]
async fn disconnect_with_cooler_off_is_synchronous() {
    let (state, _dw) = AppState::new_for_testing(5, 85);
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
    let (state, _dw) = AppState::new_for_testing(5, 85);
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
    let (state, _dw) = AppState::new_for_testing(5, 85);
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
async fn return_from_capture_without_handle_finalizes_disconnect() {
    let (state, _dw) = AppState::new_for_testing(5, 85);
    let state = Arc::new(state);
    let name = install_mock_camera(&state, 5.0, false, 20.0).await;

    // Simulate capture thread panicking: take the handle and drop it, then
    // call return_from_capture with None.
    let _cam = lifecycle::take_for_capture(&state, &name).await.unwrap();

    lifecycle::return_from_capture(&state, &name, None).await;

    assert_eq!(state.camera_phase(&name).await, CameraPhase::Disconnected);
    assert!(state.cameras.read().await.is_empty());
}
