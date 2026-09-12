//! Quiet recovery and identity-checked reconnects, against a scripted USB bus.
//!
//! The fake catalog is the point: it lets a test unplug a device, bring it back, and
//! reorder the list in between — the three things the 2026-09-07 session did to the
//! real one.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::tests::eventually;
use super::{lifecycle, reconnect};
use crate::camera::{
    Camera, CameraEntry, CameraError, CameraInfo, CameraResult, CameraStatus, CaptureConfig,
    DeviceCatalog, DeviceIdentity, GainPresets, ImageFormat, OpenedCamera, RawFrame, SensorType,
};
use crate::server::camera_session::lifecycle::DisconnectCause;
use crate::server::events::ServerEvent;
use crate::server::services::{CameraService, CaptureService};
use crate::server::state::{
    AppState, CameraPhase, CameraRole, CaptureState, Recovery, SessionResumePlan,
};

const PROVIDER: &str = "Fake";

#[derive(Clone)]
struct FakeDevice {
    name: &'static str,
    serial: Option<&'static str>,
    device_id: i32,
}

const NEPTUNE: FakeDevice = FakeDevice {
    name: "Neptune-C II",
    serial: Some("NEP123"),
    device_id: 10,
};
const ARES: FakeDevice = FakeDevice {
    name: "Ares-C PRO",
    serial: Some("ARE456"),
    device_id: 11,
};

/// A USB bus a test can rearrange. `open` records every index it was asked for.
#[derive(Default)]
struct FakeCatalog {
    devices: Mutex<Vec<FakeDevice>>,
    opened: Mutex<Vec<&'static str>>,
    /// Makes `open` hand back this device instead of the listed one, the way a list
    /// that reordered between enumeration and open would.
    impostor: Mutex<Option<FakeDevice>>,
    released: Arc<AtomicUsize>,
    /// Opens of this device wait inside the "vendor SDK" until it is cleared, the way a
    /// real open takes seconds on a busy bus — or never returns.
    held: Mutex<Option<&'static str>>,
    open_calls: AtomicUsize,
    /// Captures still to fail with a stall, across every camera this bus opens.
    stalls: Arc<AtomicUsize>,
    /// Captures still to fail with an ordinary, non-fault error.
    failures: Arc<AtomicUsize>,
    /// Captures still to fail as a lost device.
    lost: Arc<AtomicUsize>,
    /// Run once from inside `install_camera`, when it applies the dew heater.
    during_install: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl FakeCatalog {
    fn with(devices: &[FakeDevice]) -> Arc<Self> {
        let catalog = Self::default();
        *catalog.devices.lock().unwrap() = devices.to_vec();
        Arc::new(catalog)
    }

    fn set(&self, devices: &[FakeDevice]) {
        *self.devices.lock().unwrap() = devices.to_vec();
    }

    fn hold(&self, device: Option<&FakeDevice>) {
        *self.held.lock().unwrap() = device.map(|device| device.name);
    }

    fn opened(&self) -> Vec<&'static str> {
        self.opened.lock().unwrap().clone()
    }

    fn info_for(device: &FakeDevice) -> CameraInfo {
        CameraInfo {
            name: device.name.to_string(),
            id: device.device_id,
            serial: device.serial.map(str::to_string),
            max_width: 32,
            max_height: 24,
            sensor_type: SensorType::Mono,
            supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
            has_dew_heater: true,
            ..Default::default()
        }
    }

    fn check_provider(provider: &str) -> CameraResult<()> {
        if provider.eq_ignore_ascii_case(PROVIDER) {
            return Ok(());
        }
        Err(CameraError::ProviderNotFound(provider.to_string()))
    }
}

impl DeviceCatalog for FakeCatalog {
    fn list_all(&self, _use_simulated: bool) -> CameraResult<Vec<CameraEntry>> {
        Ok(self
            .devices
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, device)| CameraEntry {
                provider: PROVIDER.to_string(),
                index,
                info: Self::info_for(device),
            })
            .collect())
    }

    fn identities(&self, provider: &str, _use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>> {
        Self::check_provider(provider)?;
        Ok(self
            .devices
            .lock()
            .unwrap()
            .iter()
            .map(|device| {
                DeviceIdentity::of(&Self::info_for(device)).with_device_id(device.device_id)
            })
            .collect())
    }

    fn open(&self, provider: &str, index: usize, _use_simulated: bool) -> CameraResult<OpenedCamera> {
        Self::check_provider(provider)?;
        self.open_calls.fetch_add(1, Ordering::SeqCst);
        let listed = self.devices.lock().unwrap().get(index).cloned();
        let Some(listed) = listed else {
            return Err(CameraError::InvalidCameraIndex { index, count: 0 });
        };
        while *self.held.lock().unwrap() == Some(listed.name) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let device = self.impostor.lock().unwrap().clone().unwrap_or(listed);
        self.opened.lock().unwrap().push(device.name);
        Ok(OpenedCamera {
            camera: Box::new(FakeCamera {
                info: Self::info_for(&device),
                cancel_flag: Arc::new(AtomicBool::new(false)),
                released: Arc::clone(&self.released),
                stalls: Arc::clone(&self.stalls),
                failures: Arc::clone(&self.failures),
                lost: Arc::clone(&self.lost),
                during_install: Arc::clone(&self.during_install),
            }),
            provider: PROVIDER.to_string(),
        })
    }
}

struct FakeCamera {
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
    released: Arc<AtomicUsize>,
    stalls: Arc<AtomicUsize>,
    failures: Arc<AtomicUsize>,
    lost: Arc<AtomicUsize>,
    during_install: Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl Drop for FakeCamera {
    fn drop(&mut self) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }
}

impl Camera for FakeCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }
    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets::default())
    }
    fn status(&self) -> CameraResult<CameraStatus> {
        Ok(CameraStatus::default())
    }
    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Ok(())
    }
    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Ok(())
    }
    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        let hook = self.during_install.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }
    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<RawFrame> {
        std::thread::sleep(Duration::from_millis(20));
        if self.cancel_flag.swap(false, Ordering::SeqCst) {
            return Err(CameraError::Cancelled);
        }
        if take_one(&self.stalls) {
            return Err(CameraError::ExposureTimeout(Duration::from_millis(20)));
        }
        if take_one(&self.failures) {
            return Err(CameraError::ExposureFailed("scripted failure".to_string()));
        }
        if take_one(&self.lost) {
            return Err(CameraError::Disconnected);
        }
        let pixels = (self.info.max_width * self.info.max_height) as usize;
        Ok(RawFrame {
            data: vec![7u8; pixels].into(),
            width: self.info.max_width,
            height: self.info.max_height,
            format: ImageFormat::Raw8,
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
        PROVIDER
    }
}

fn take_one(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1))
        .is_ok()
}

fn rig(catalog: &Arc<FakeCatalog>) -> Arc<AppState> {
    build_rig(catalog, false)
}

/// A rig whose disk writer is running, for tests that look at the files a capture saves.
fn rig_with_disk_writer(catalog: &Arc<FakeCatalog>) -> Arc<AppState> {
    build_rig(catalog, true)
}

fn build_rig(catalog: &Arc<FakeCatalog>, run_disk_writer: bool) -> Arc<AppState> {
    let (mut state, disk_writer) = AppState::new_for_testing();
    if run_disk_writer {
        std::thread::spawn(move || disk_writer.run());
    }
    state.device_catalog = Arc::clone(catalog) as Arc<dyn DeviceCatalog>;
    let state = Arc::new(state);
    let settings = Arc::clone(&state);
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let mut settings = settings.settings.write().await;
            settings.auto_reconnect = true;
            settings.indi_server_host.clear();
        })
    });
    state
}

fn id_of(device: &FakeDevice) -> String {
    crate::camera::identity::camera_id(PROVIDER, 0, device.serial)
}

async fn connect(state: &Arc<AppState>, device: &FakeDevice, role: CameraRole) {
    lifecycle::connect(state, &id_of(device), role)
        .await
        .unwrap_or_else(|e| panic!("connecting {} failed: {e}", device.name));
}

fn drain(events: &mut tokio::sync::broadcast::Receiver<ServerEvent>) -> Vec<ServerEvent> {
    use tokio::sync::broadcast::error::TryRecvError;
    let mut seen = Vec::new();
    loop {
        match events.try_recv() {
            Ok(event) => seen.push(event),
            Err(TryRecvError::Lagged(_)) => continue,
            Err(_) => return seen,
        }
    }
}

fn is_loud(event: &ServerEvent) -> bool {
    matches!(
        event,
        ServerEvent::CameraDisconnected { .. }
            | ServerEvent::Error { .. }
            | ServerEvent::CameraReconnecting { .. }
            | ServerEvent::CameraReconnectFailed { .. }
            | ServerEvent::CaptureResumed { .. }
    )
}

async fn phase_of(state: &Arc<AppState>, role: CameraRole) -> CameraPhase {
    state.camera_phase(role).await
}

/// Back, and finished coming back: the phase turns before the install has let go of
/// the slot's recovery state.
async fn wait_recovered(state: &Arc<AppState>, role: CameraRole) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let phase = phase_of(state, role).await;
        let back = !matches!(phase, CameraPhase::Recovering | CameraPhase::Disconnected);
        if back && !state.slot(role).is_recovering() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn teardown(state: &Arc<AppState>) {
    for role in CameraRole::all() {
        if let Some(camera) = state.camera_in_role(role).await {
            lifecycle::finalize_disconnect(state, role, &camera.info.name, DisconnectCause::Requested).await;
        }
    }
}

// --- Identity ------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_serial_id_opens_the_device_wherever_it_is_listed() {
    let catalog = FakeCatalog::with(&[ARES, NEPTUNE]);
    let state = rig(&catalog);

    connect(&state, &NEPTUNE, CameraRole::Main).await;

    assert_eq!(catalog.opened(), vec!["Neptune-C II"]);
    let main = state.camera_in_role(CameraRole::Main).await.unwrap();
    assert_eq!(main.id, "fake_sn-NEP123");
    assert_eq!(main.index, 1);
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_serial_that_opens_as_another_camera_is_refused_and_closed() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    *catalog.impostor.lock().unwrap() = Some(ARES);

    let result = lifecycle::connect(&state, &id_of(&NEPTUNE), CameraRole::Main).await;

    assert!(
        matches!(result, Err(crate::server::error::ApiError::CameraIdentityMismatch { .. })),
        "got {result:?}"
    );
    assert_eq!(catalog.released.load(Ordering::SeqCst), 1, "the wrong camera is closed again");
    assert!(state.cameras.read().await.is_empty());
    assert!(!state.slot(CameraRole::Main).holds_handle());
}

/// A connected camera keeps the id it was connected under. After the list reorders,
/// discovery must still recognise it rather than offer it a second time.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_does_not_offer_a_connected_camera_again_after_a_reorder() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    catalog.set(&[ARES, NEPTUNE]);
    let listed = CameraService::list_cameras(&state).await;

    let neptunes = listed.iter().filter(|c| c.name == NEPTUNE.name).count();
    assert_eq!(neptunes, 1, "listed: {:?}", listed.iter().map(|c| (&c.id, c.connected)).collect::<Vec<_>>());
    assert!(listed.iter().any(|c| c.name == ARES.name && !c.connected));
    teardown(&state).await;
}

/// The 2026-09-07 12:40:53 incident: the guide camera drops out, the list reorders,
/// and its old position now holds the imaging camera.
#[tokio::test(flavor = "multi_thread")]
async fn the_guide_camera_comes_back_as_itself_after_the_list_reorders() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    connect(&state, &ARES, CameraRole::Main).await;
    let opened_before = catalog.opened().len();

    catalog.set(&[ARES, NEPTUNE]);
    lifecycle::finalize_disconnect(&state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    assert!(wait_recovered(&state, CameraRole::Guide).await, "guide camera never came back");
    assert_eq!(
        catalog.opened()[opened_before..],
        ["Neptune-C II"],
        "recovery must open only the guide camera's own device"
    );
    let guide = state.camera_in_role(CameraRole::Guide).await.unwrap();
    assert_eq!(guide.info.name, NEPTUNE.name);
    assert_eq!(guide.index, 1);
    let main = state.camera_in_role(CameraRole::Main).await.unwrap();
    assert_eq!(main.info.name, ARES.name, "the imaging camera is untouched");
    assert!(state.slot(CameraRole::Main).holds_handle());
    teardown(&state).await;
}

/// Without serials the model name is all there is; the SDK device id still tells the
/// other role's body apart, so recovery never steals it.
#[tokio::test(flavor = "multi_thread")]
async fn two_bodies_of_one_model_recover_without_swapping() {
    const MAIN_BODY: FakeDevice = FakeDevice { name: "ASI120MM", serial: None, device_id: 4 };
    const GUIDE_BODY: FakeDevice = FakeDevice { name: "ASI120MM", serial: None, device_id: 7 };
    let catalog = FakeCatalog::with(&[MAIN_BODY, GUIDE_BODY]);
    let state = rig(&catalog);
    lifecycle::connect(&state, "fake_0", CameraRole::Main).await.unwrap();
    lifecycle::connect(&state, "fake_1", CameraRole::Guide).await.unwrap();

    catalog.set(&[GUIDE_BODY, MAIN_BODY]);
    lifecycle::finalize_disconnect(&state, CameraRole::Guide, GUIDE_BODY.name, DisconnectCause::DeviceFault).await;

    assert!(
        eventually(
            || !state.slot(CameraRole::Guide).is_recovering(),
            Duration::from_secs(5)
        )
        .await
    );
    let guide = state.camera_in_role(CameraRole::Guide).await.unwrap();
    assert_eq!(guide.info.id, GUIDE_BODY.device_id);
    assert_eq!(guide.index, 0, "found at its new position");
    assert_eq!(guide.id, "fake_1", "but kept under its own id: `fake_0` is the imaging camera's key");
    let main = state.camera_in_role(CameraRole::Main).await.unwrap();
    assert_eq!(main.info.id, MAIN_BODY.device_id);
    assert_eq!(state.cameras.read().await.len(), 2);
    teardown(&state).await;
}

// --- Quiet recovery ------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_fault_suspends_the_camera_instead_of_disconnecting_it() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    assert_eq!(phase_of(&state, CameraRole::Main).await, CameraPhase::Recovering);
    assert!(state.cameras.read().await.contains_key(&id_of(&NEPTUNE)));
    assert_eq!(state.selected_camera.read().await.as_deref(), Some(id_of(&NEPTUNE).as_str()));
    assert!(state.slot(CameraRole::Main).is_recovering());
    assert!(!state.slot(CameraRole::Main).holds_handle());
    let loud: Vec<_> = drain(&mut events).into_iter().filter(is_loud).collect();
    assert!(loud.is_empty(), "nothing to tell the observer yet: {loud:?}");
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quick_recovery_is_invisible() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    assert!(wait_recovered(&state, CameraRole::Main).await);
    assert!(state.slot(CameraRole::Main).holds_handle());
    assert!(!state.slot(CameraRole::Main).is_recovering());
    let loud: Vec<_> = drain(&mut events).into_iter().filter(is_loud).collect();
    assert!(loud.is_empty(), "a recovery inside the notice window must stay silent: {loud:?}");
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_recovery_tells_the_observer_once_it_is_worth_it() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    tokio::time::sleep(reconnect::NOTICE_AFTER / 2).await;
    assert!(
        !drain(&mut events).iter().any(|e| matches!(e, ServerEvent::CameraReconnecting { .. })),
        "too early to say anything"
    );

    tokio::time::sleep(reconnect::NOTICE_AFTER).await;
    catalog.set(&[NEPTUNE]);
    assert!(wait_recovered(&state, CameraRole::Main).await);

    let seen = drain(&mut events);
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::CameraReconnecting { .. })));
    assert!(!seen.iter().any(|e| matches!(e, ServerEvent::CameraDisconnected { .. })));
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn giving_up_ends_the_session_and_says_why() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    assert!(
        eventually(
            || state.cameras.try_read().map(|c| c.is_empty()).unwrap_or(false),
            reconnect::TOTAL_BUDGET + Duration::from_secs(3)
        )
        .await,
        "the session should end once the budget is spent"
    );
    assert!(
        eventually(
            || !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(2)
        )
        .await
    );
    let seen = drain(&mut events);
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::CameraDisconnected { .. })));
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::CameraReconnectFailed { .. })));
    assert!(!state.slot(CameraRole::Main).is_recovering());
}

/// Twice on 2026-09-07 the observer clicked Connect while a reconnect was waiting, and
/// that cancelled it. Now the click finds the camera still there.
#[tokio::test(flavor = "multi_thread")]
async fn connecting_during_recovery_joins_it() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let answer = lifecycle::connect(&state, &id_of(&NEPTUNE), CameraRole::Main).await;
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(phase_of(&state, CameraRole::Main).await, CameraPhase::Recovering);
    assert!(state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst));

    catalog.set(&[NEPTUNE]);
    assert!(wait_recovered(&state, CameraRole::Main).await, "the supervisor must carry on");
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_during_recovery_stops_it_quietly() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opened_before = catalog.opened().len();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let mut events = state.subscribe_events();
    lifecycle::disconnect(&state, &id_of(&NEPTUNE)).await.unwrap();
    catalog.set(&[NEPTUNE]);

    assert!(
        eventually(
            || !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(3)
        )
        .await,
        "the supervisor should stop"
    );
    assert!(state.cameras.read().await.is_empty());
    assert_eq!(catalog.opened().len(), opened_before, "nothing reopened after the disconnect");
    assert!(
        !drain(&mut events).iter().any(|e| matches!(e, ServerEvent::CameraReconnectFailed { .. })),
        "the observer ended it; there is no failure to report"
    );
}

async fn start_capture_plan(state: &Arc<AppState>, device: &FakeDevice) {
    *state.session_resume_plan.write().await = Some(SessionResumePlan {
        camera_id: id_of(device),
        settings: state.settings.read().await.clone(),
        disk_session_dir: None,
        next_frame: 1,
    });
    state.session.write().await.stacked_count = 514;
    state.set_capture_state(CaptureState::Capturing).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_is_paused_through_recovery_and_resumed_after_it() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;
    let mut events = state.subscribe_events();

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let paused = state.capture_state().await;
    // The pipeline ending after the fault must not mark the session over.
    state.end_capture_state().await;
    let still_paused = state.capture_state().await;

    catalog.set(&[NEPTUNE]);
    let resumed = eventually(
        || {
            matches!(
                state.session.try_read().map(|s| s.state),
                Ok(CaptureState::Capturing)
            )
        },
        Duration::from_secs(5),
    )
    .await;
    let seen = drain(&mut events);

    // Stopped before asserting: a resumed pipeline left running would hang the runtime's
    // shutdown instead of reporting the failure.
    CaptureService::stop_capture(&state).await;
    let stopped = eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;

    assert_eq!(paused, CaptureState::Recovering);
    assert_eq!(still_paused, CaptureState::Recovering);
    assert!(resumed, "the capture should resume on its own");
    let loud: Vec<_> = seen.iter().filter(|e| is_loud(e)).collect();
    assert!(loud.is_empty(), "a quick recovery resumes without a word: {loud:?}");
    assert!(stopped);
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_during_recovery_leaves_nothing_to_resume() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(CaptureService::stop_capture(&state).await);
    assert_eq!(state.capture_state().await, CaptureState::Idle, "no pipeline left to wind down");
    assert!(state.session_resume_plan.read().await.is_none());

    catalog.set(&[NEPTUNE]);
    assert!(wait_recovered(&state, CameraRole::Main).await, "the camera still comes back");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.capture_state().await, CaptureState::Idle, "but the capture does not");
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_capture_cannot_start_over_a_paused_one() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    let refused = CaptureService::start_capture(&state, None).await;
    assert!(matches!(refused, Err(crate::server::error::ApiError::CaptureInProgress)), "{refused:?}");
    CaptureService::stop_capture(&state).await;
    teardown(&state).await;
}

/// Reopening while an abandoned call is still inside the vendor SDK is the race the
/// wait exists for; the wait must also end as soon as the call does.
#[tokio::test(flavor = "multi_thread")]
async fn the_first_attempt_waits_for_an_abandoned_call_to_return() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opened_before = catalog.opened().len();

    let abandoned = state.slot(CameraRole::Main).sdk_calls.begin();
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    tokio::time::sleep(reconnect::ABANDONED_CALL_WAIT / 2).await;
    assert_eq!(catalog.opened().len(), opened_before, "reopened while a call was still in the SDK");

    drop(abandoned);
    assert!(wait_recovered(&state, CameraRole::Main).await);
    teardown(&state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recovering_guide_camera_keeps_plate_solving() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| state.guide_loop_running(), Duration::from_secs(3)).await);

    catalog.set(&[ARES]);
    lifecycle::finalize_disconnect(&state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    assert!(!state.guide_loop_running());
    assert!(
        state.guide_holds_solving(),
        "the imaging camera must not take solving over against the guide scope's optics"
    );
    teardown(&state).await;
    assert!(!state.guide_holds_solving(), "a disconnect hands it back");
}

// --- Review of 0d4e075: races and stranded recoveries ---------------------------------

/// `reopen_for_recovery` checks the camera is still being recovered before it opens the
/// device, never after, and `disconnect` does not take the connect lock. An observer's
/// Disconnect that lands while the open is inside the SDK is undone when it returns.
#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_during_a_reopen_is_not_undone() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opens_before = catalog.open_calls.load(Ordering::SeqCst);
    catalog.hold(Some(&NEPTUNE));

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(
        eventually(|| catalog.open_calls.load(Ordering::SeqCst) > opens_before, Duration::from_secs(3)).await,
        "the supervisor never started reopening"
    );
    lifecycle::disconnect(&state, &id_of(&NEPTUNE)).await.unwrap();
    catalog.hold(None);

    assert!(
        eventually(
            || !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(3)
        )
        .await
    );
    let still_registered = state.cameras.read().await.len();
    let holds_handle = state.slot(CameraRole::Main).holds_handle();
    teardown(&state).await;
    assert_eq!(still_registered, 0, "the observer disconnected this camera; recovery brought it back");
    assert!(!holds_handle, "a disconnected camera's device was left open");
}

/// Phase is keyed by model name, and `is_recovering` reads it. With two bodies of one
/// model, the imaging camera starting and ending a capture rewrites the phase the
/// recovering guide camera is recognised by, the supervisor walks away, and the guide
/// slot stays `recovering` — holding plate solving — with nobody to bring it back.
#[tokio::test(flavor = "multi_thread")]
async fn a_twin_imaging_camera_does_not_strand_the_guide_recovery() {
    const MAIN_BODY: FakeDevice = FakeDevice { name: "ASI120MM", serial: None, device_id: 4 };
    const GUIDE_BODY: FakeDevice = FakeDevice { name: "ASI120MM", serial: None, device_id: 7 };
    let catalog = FakeCatalog::with(&[MAIN_BODY, GUIDE_BODY]);
    let state = rig(&catalog);
    lifecycle::connect(&state, "fake_0", CameraRole::Main).await.unwrap();
    lifecycle::connect(&state, "fake_1", CameraRole::Guide).await.unwrap();

    catalog.set(&[MAIN_BODY]);
    lifecycle::finalize_disconnect(&state, CameraRole::Guide, GUIDE_BODY.name, DisconnectCause::DeviceFault).await;
    let camera = lifecycle::take_for_capture(&state, CameraRole::Main, MAIN_BODY.name).await.unwrap();
    lifecycle::return_from_capture(&state, CameraRole::Main, MAIN_BODY.name, Some(camera)).await;
    catalog.set(&[MAIN_BODY, GUIDE_BODY]);

    let back = eventually(|| state.guide_loop_running(), Duration::from_secs(5)).await;
    let stranded = state.slot(CameraRole::Guide).is_recovering()
        && !state.slot(CameraRole::Guide).reconnect_in_flight.load(Ordering::SeqCst);
    teardown(&state).await;
    assert!(!stranded, "guide slot left recovering with no supervisor");
    assert!(back, "the guide camera was plugged back in but never reopened");
}

/// `suspend` starts a supervisor through `reconnect::spawn`, which refuses while the
/// previous supervisor is still in flight — finishing a resume, say. A device that fails
/// again inside that window must still get one when the previous supervisor ends; the old
/// flag left it suspended with nobody recovering it.
#[tokio::test(flavor = "multi_thread")]
async fn a_fault_while_the_previous_supervisor_winds_down_is_still_recovered() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    // The previous supervisor has reinstalled the camera but its task has not ended.
    state.slot(CameraRole::Main).reconnect_in_flight.store(true, Ordering::SeqCst);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert_eq!(state.slot(CameraRole::Main).recovery(), Recovery::Suspended);
    // ... and now it ends.
    reconnect::release_flight(&state, CameraRole::Main).await;

    let back = wait_recovered(&state, CameraRole::Main).await;
    teardown(&state).await;
    assert!(back, "suspended with no supervisor: the camera stays Recovering forever");
}

/// A reopened camera that fails again straight away — here the guide loop's first
/// exposure after the reinstall — belongs to whichever path the timing picks: deferred
/// mid-install, orphaned as the supervisor ends, or a fresh suspension. Every one of them
/// must end with the camera back once the device is.
#[tokio::test(flavor = "multi_thread")]
async fn a_camera_that_fails_again_right_after_its_reinstall_still_comes_back() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| state.guide_loop_running(), Duration::from_secs(3)).await);

    // Two losses in a row: the running loop's exposure, then the reopened loop's first.
    catalog.lost.store(2, Ordering::SeqCst);
    assert!(
        eventually(|| catalog.lost.load(Ordering::SeqCst) == 0, Duration::from_secs(5)).await,
        "both losses should have been reported"
    );

    let back = eventually(
        || state.guide_loop_running() && !state.slot(CameraRole::Guide).is_recovering(),
        Duration::from_secs(5),
    )
    .await;
    let stranded = state.slot(CameraRole::Guide).is_recovering()
        && !state.slot(CameraRole::Guide).reconnect_in_flight.load(Ordering::SeqCst);
    teardown(&state).await;
    assert!(!stranded, "guide slot left recovering with no supervisor");
    assert!(back, "the guide camera never came back after failing twice");
}

/// The manual promises that a lost frame costs that frame. The first frame of a capture
/// is taken outside the stall ladder, so one stall there ends the session with an error
/// — and after a reopen, the first frame is the one most likely to be slow.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_first_frame_does_not_end_the_capture() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();
    catalog.stalls.store(1, Ordering::SeqCst);

    CaptureService::start_capture(&state, None).await.unwrap();
    let capturing = eventually(
        || {
            state.delivered_frames.load(Ordering::SeqCst) >= 2
                && matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Capturing))
        },
        Duration::from_secs(5),
    )
    .await;
    let errors: Vec<_> = drain(&mut events)
        .into_iter()
        .filter(|e| matches!(e, ServerEvent::Error { .. }))
        .collect();

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;
    assert!(capturing, "one stalled probe frame ended the capture");
    assert!(errors.is_empty(), "a restartable stall is not worth an error: {errors:?}");
}

/// The resume plan is cleared only by Stop and by a recovery that ends the session. A
/// capture that ends on its own — here its first frame fails outright — leaves it
/// behind, and the next quiet recovery of the now-idle camera starts that capture again
/// without the observer asking, into the old raw folder and on the old stack.
#[tokio::test(flavor = "multi_thread")]
async fn a_capture_that_ended_on_its_own_is_not_resumed_by_a_later_recovery() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    catalog.failures.store(1, Ordering::SeqCst);

    CaptureService::start_capture(&state, None).await.unwrap();
    assert!(
        eventually(
            || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
            Duration::from_secs(5)
        )
        .await,
        "the failed first frame should have ended the capture"
    );

    assert!(state.session_resume_plan.read().await.is_none(), "an ended capture kept its resume plan");
    assert!(
        state.stacking_carryover.lock().unwrap().is_none(),
        "an ended capture kept its parked stack"
    );

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(wait_recovered(&state, CameraRole::Main).await);
    let restarted = eventually(
        || !matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_millis(500),
    )
    .await;

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;
    assert!(!restarted, "recovery restarted a capture that had already ended");
}

/// A fault on the reopened handle *while it is being installed* read as a duplicate of the
/// fault being recovered and was dropped, so the install finished over the handle that had
/// just failed. It must send recovery round again instead.
#[tokio::test(flavor = "multi_thread")]
async fn a_fault_during_the_reinstall_is_recovered_not_swallowed() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opened_before = catalog.opened().len();

    let fired = Arc::new(AtomicBool::new(false));
    {
        let (state, fired, rt) = (Arc::clone(&state), Arc::clone(&fired), tokio::runtime::Handle::current());
        *catalog.during_install.lock().unwrap() = Some(Box::new(move || {
            // Reported from another thread, the way the new monitor or guide loop would.
            std::thread::spawn(move || {
                rt.block_on(lifecycle::finalize_disconnect(
                    &state,
                    CameraRole::Main,
                    NEPTUNE.name,
                    DisconnectCause::DeviceFault,
                ));
                fired.store(true, Ordering::SeqCst);
            })
            .join()
            .unwrap();
        }));
    }
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    let back = wait_recovered(&state, CameraRole::Main).await;
    let reopened = catalog.opened().len() - opened_before;
    teardown(&state).await;
    assert!(fired.load(Ordering::SeqCst), "the fault never landed mid-install");
    assert!(back, "the camera never came back");
    assert_eq!(reopened, 2, "the handle that failed mid-install was kept instead of reopened");
}

/// A dying loop can report its camera lost after the observer has already replaced that
/// camera. The report named the old camera, but the teardown took the new one's handle.
#[tokio::test(flavor = "multi_thread")]
async fn a_late_fault_for_a_replaced_camera_leaves_the_replacement_alone() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    lifecycle::disconnect(&state, &id_of(&NEPTUNE)).await.unwrap();
    connect(&state, &ARES, CameraRole::Main).await;

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;

    let main = state.camera_in_role(CameraRole::Main).await.map(|c| c.info.name);
    let holds_handle = state.slot(CameraRole::Main).holds_handle();
    let phase = phase_of(&state, CameraRole::Main).await;
    teardown(&state).await;
    assert_eq!(main.as_deref(), Some(ARES.name));
    assert!(holds_handle, "the replacement camera's handle was closed");
    assert_ne!(phase, CameraPhase::Recovering);
}

/// A recovering slot refuses the guide loop's hand-back and closes the handle, so "the
/// slot holds a handle again" never comes true. `stop` still has to wait for the loop to
/// be gone — it used to return at once, with the loop still exposing on the handle.
#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_guide_loop_of_a_recovering_slot_waits_for_the_loop() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| state.guide_loop_running(), Duration::from_secs(3)).await);
    let released_before = catalog.released.load(Ordering::SeqCst);

    assert_eq!(state.slot(CameraRole::Guide).begin_suspend(), crate::server::state::SuspendVerdict::Suspended);
    let started = std::time::Instant::now();
    crate::server::capture::guide_task::stop(&state).await;
    let waited = started.elapsed();
    let released = catalog.released.load(Ordering::SeqCst) - released_before;

    teardown(&state).await;
    assert_eq!(released, 1, "stop returned while the loop still held its handle");
    assert!(waited < Duration::from_secs(2), "stop sat out its whole budget: {waited:?}");
}

/// Past `STALL_ESCALATION` stalls the first frame is a fault like any other: the capture
/// pauses for recovery, comes back, and resumes — without an error on the way.
#[tokio::test(flavor = "multi_thread")]
async fn a_first_frame_that_never_comes_hands_the_capture_to_recovery() {
    use crate::server::capture::watchdog::STALL_ESCALATION;

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();
    catalog.stalls.store(STALL_ESCALATION as usize, Ordering::SeqCst);

    CaptureService::start_capture(&state, None).await.unwrap();
    let resumed = eventually(
        || {
            state.delivered_frames.load(Ordering::SeqCst) >= 2
                && matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Capturing))
        },
        Duration::from_secs(5),
    )
    .await;
    let seen = drain(&mut events);
    let paused = seen.iter().any(|e| {
        matches!(e, ServerEvent::StateChanged { state: crate::server::events::CaptureStateDto::Recovering })
    });
    let errors: Vec<_> = seen.iter().filter(|e| matches!(e, ServerEvent::Error { .. })).collect();

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;
    assert!(paused, "an unending first frame should pause the capture for recovery");
    assert!(resumed, "the capture should resume once the camera is back");
    assert!(errors.is_empty(), "recovery decides what the observer hears: {errors:?}");
}

/// A vendor open that never returns held the connect lock for good: every Connect hung
/// and the recovery budget never ran out. And since a late open takes the device lease,
/// nothing may open the device again until it has returned.
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_reopen_neither_blocks_connect_nor_is_opened_past() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opens_before = catalog.open_calls.load(Ordering::SeqCst);
    catalog.hold(Some(&NEPTUNE));

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(eventually(|| catalog.open_calls.load(Ordering::SeqCst) > opens_before, Duration::from_secs(3)).await);

    let other = tokio::time::timeout(
        reconnect::OPEN_TIMEOUT + Duration::from_secs(1),
        lifecycle::connect(&state, &id_of(&ARES), CameraRole::Guide),
    )
    .await;
    // Several retries' worth of time with the first open still hung: only ARES's own
    // connect may have reached the SDK besides it.
    tokio::time::sleep(reconnect::OPEN_TIMEOUT * 2).await;
    let opens_while_hung = catalog.open_calls.load(Ordering::SeqCst) - opens_before;
    catalog.hold(None);
    let connected_other = matches!(other, Ok(Ok(_)));

    let back = wait_recovered(&state, CameraRole::Main).await;
    teardown(&state).await;

    assert!(connected_other, "connecting another camera waited on a hung reopen: {other:?}");
    assert!(back, "recovery should carry on once the open returns");
    assert_eq!(opens_while_hung, 2, "reopened past a pending open");
}

/// Disconnect waits for a reopen that is inside the SDK. When that open then succeeds,
/// the camera is back — and the Disconnect still has to disconnect it.
#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_waiting_on_a_successful_reopen_still_disconnects() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let opens_before = catalog.open_calls.load(Ordering::SeqCst);
    catalog.hold(Some(&NEPTUNE));

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(eventually(|| catalog.open_calls.load(Ordering::SeqCst) > opens_before, Duration::from_secs(3)).await);
    let disconnect = tokio::spawn({
        let state = Arc::clone(&state);
        async move { lifecycle::disconnect(&state, &id_of(&NEPTUNE)).await }
    });
    tokio::time::sleep(reconnect::OPEN_TIMEOUT / 4).await;
    catalog.hold(None);

    let answer = disconnect.await.unwrap();
    assert!(
        eventually(
            || !state.slot(CameraRole::Main).reconnect_in_flight.load(Ordering::SeqCst),
            Duration::from_secs(3)
        )
        .await
    );
    let registered = state.cameras.read().await.len();
    let holds_handle = state.slot(CameraRole::Main).holds_handle();
    teardown(&state).await;
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(registered, 0, "the reopened camera outlived the disconnect");
    assert!(!holds_handle);
}

// --- Review of bb229a5 ---------------------------------------------------------------

/// `OPEN_TIMEOUT` bounds the open and the probe, but the install that follows still
/// calls the vendor SDK — status, cooler, dew heater — inline, under the connect lock. A
/// device that answers the probe and then hangs holds every Connect, exactly as a hung
/// open used to.
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_vendor_call_during_the_reinstall_does_not_block_connect() {
    let catalog = FakeCatalog::with(&[NEPTUNE, ARES]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    let (entered, release) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    {
        let (entered, release) = (Arc::clone(&entered), Arc::clone(&release));
        *catalog.during_install.lock().unwrap() = Some(Box::new(move || {
            entered.store(true, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !release.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }));
    }
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(
        eventually(|| entered.load(Ordering::SeqCst), Duration::from_secs(3)).await,
        "the reinstall never reached its vendor calls"
    );

    let other = tokio::time::timeout(
        reconnect::OPEN_TIMEOUT + Duration::from_secs(1),
        lifecycle::connect(&state, &id_of(&ARES), CameraRole::Guide),
    )
    .await;
    release.store(true, Ordering::SeqCst);

    wait_recovered(&state, CameraRole::Main).await;
    teardown(&state).await;
    assert!(matches!(other, Ok(Ok(_))), "connecting another camera waited on a hung reinstall: {other:?}");
}

const MAIN_BODY: FakeDevice = FakeDevice { name: "ASI294MC", serial: None, device_id: 0 };
const FREE_BODY: FakeDevice = FakeDevice { name: "ASI120MM", serial: None, device_id: 1 };

/// Two bodies without serials, the ZWO case. A reset device comes back with a new SDK id,
/// which sorts it after the free camera; the recovered entry keeps its old index id.
async fn recover_imaging_camera_past_a_free_one(catalog: &Arc<FakeCatalog>, state: &Arc<AppState>) {
    const MAIN_REENUMERATED: FakeDevice = FakeDevice { name: "ASI294MC", serial: None, device_id: 2 };
    lifecycle::connect(state, "fake_0", CameraRole::Main).await.unwrap();

    catalog.set(&[FREE_BODY, MAIN_REENUMERATED]);
    lifecycle::finalize_disconnect(state, CameraRole::Main, MAIN_BODY.name, DisconnectCause::DeviceFault).await;
    assert!(wait_recovered(state, CameraRole::Main).await, "the imaging camera never came back");
    let main = state.camera_in_role(CameraRole::Main).await.unwrap();
    assert_eq!((main.id.as_str(), main.index), ("fake_0", 1), "precondition: kept its id, moved its index");
}

/// Discovery matches serial-less cameras by id string. After the move above, `fake_0`
/// names the free camera's position and `fake_1` the imaging camera's, so the free camera
/// is hidden as "connected" and the imaging camera is offered for connecting.
#[tokio::test(flavor = "multi_thread")]
async fn discovery_after_a_serialless_camera_moved_offers_the_free_camera_not_the_connected_one() {
    let catalog = FakeCatalog::with(&[MAIN_BODY, FREE_BODY]);
    let state = rig(&catalog);
    recover_imaging_camera_past_a_free_one(&catalog, &state).await;

    let listed = CameraService::list_cameras(&state).await;
    let offered: Vec<_> = listed.iter().filter(|c| !c.connected).map(|c| (c.id.clone(), c.name.clone())).collect();
    teardown(&state).await;

    assert!(
        offered.iter().any(|(_, name)| name == FREE_BODY.name),
        "the free camera is hidden: offered {offered:?}"
    );
    assert!(
        !offered.iter().any(|(_, name)| name == MAIN_BODY.name),
        "the connected imaging camera is offered again: {offered:?}"
    );
}

/// What discovery offers is what the observer clicks: connecting it as the guide camera
/// must not open the device the imaging camera is capturing with.
#[tokio::test(flavor = "multi_thread")]
async fn connecting_the_offered_position_never_opens_the_other_roles_device() {
    let catalog = FakeCatalog::with(&[MAIN_BODY, FREE_BODY]);
    let state = rig(&catalog);
    recover_imaging_camera_past_a_free_one(&catalog, &state).await;
    let main_device = state.camera_in_role(CameraRole::Main).await.unwrap().info.id;

    let result = lifecycle::connect(&state, "fake_1", CameraRole::Guide).await;
    let guide_device = state.camera_in_role(CameraRole::Guide).await.map(|c| c.info.id);
    teardown(&state).await;

    assert_ne!(
        guide_device,
        Some(main_device),
        "one device installed in both roles (connect returned {result:?})"
    );
}

/// `resume_capture_if_planned` checks the slot once, then resumes. A reopened camera that
/// fails again in between is suspended over a capture already told to start: its
/// pipeline finds no handle and ends the session — plan and parked stack with it — so the
/// recovery that follows brings back a camera with nothing to resume.
#[tokio::test(flavor = "multi_thread")]
async fn a_resume_that_races_a_new_fault_leaves_the_capture_paused() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;

    // The supervisor has reinstalled the camera and passed its check; the new fault's own
    // supervisor is refused while it is still in flight.
    state.slot(CameraRole::Main).reconnect_in_flight.store(true, Ordering::SeqCst);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let plan = state.session_resume_plan.read().await.clone().expect("a paused capture has a plan");
    let mut events = state.subscribe_events();

    // ... and now makes the resume call it had already decided on.
    CaptureService::resume_capture(&state, &plan).await.unwrap();
    let settled = eventually(
        || {
            state.slot(CameraRole::Main).is_recovering()
                && matches!(
                    state.session.try_read().map(|s| s.state),
                    Ok(CaptureState::Idle | CaptureState::Recovering)
                )
        },
        Duration::from_secs(6),
    )
    .await;
    let after = state.capture_state().await;
    let plan_kept = state.session_resume_plan.read().await.is_some();

    reconnect::release_flight(&state, CameraRole::Main).await;
    wait_recovered(&state, CameraRole::Main).await;
    let resumed = eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Capturing)),
        Duration::from_secs(3),
    )
    .await;
    let errors: Vec<_> = drain(&mut events)
        .into_iter()
        .filter(|e| matches!(e, ServerEvent::Error { .. }))
        .collect();

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;

    assert!(settled, "the resumed pipeline never gave up on the missing handle");
    assert_eq!(after, CaptureState::Recovering, "the paused capture was ended");
    assert!(plan_kept, "the resume plan was discarded");
    assert!(resumed, "the capture did not resume once the camera was back");
    assert!(errors.is_empty(), "recovery decides what the observer hears: {errors:?}");
}

fn saved_frame_numbers(dir: &std::path::Path) -> Vec<u64> {
    let mut numbers: Vec<u64> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| {
                    let name = entry.ok()?.file_name().into_string().ok()?;
                    name.strip_prefix("frame_")?.strip_suffix(".fits")?.parse().ok()
                })
                .collect()
        })
        .unwrap_or_default();
    numbers.sort_unstable();
    numbers
}

/// The guide loop parks `next_frame` with its raw folder because the writer names files
/// `frame_{:06}.fits`. The imaging camera's resumed pipeline rejoins its folder the same
/// way but numbers from 1 again, and the FITS writer deletes an existing file before it
/// writes — every quiet recovery overwrites the subs the session started with.
#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_capture_appends_to_the_raw_folder_it_rejoined() {
    use crate::server::state::RawFrameSaving;

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig_with_disk_writer(&catalog);
    state.settings.write().await.raw_frame_saving =
        RawFrameSaving { live_view: true, wanderer: true, stacking: true, guide: false };
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    CaptureService::start_capture(&state, None).await.unwrap();
    let dir = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(dir) = state.disk_writer.session_dir() {
                if saved_frame_numbers(&dir).len() >= 3 {
                    break dir;
                }
            }
            assert!(tokio::time::Instant::now() < deadline, "no raw frames were saved");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let first = dir.join("frame_000001.fits");
    let first_written = std::fs::metadata(&first).and_then(|m| m.modified()).unwrap();
    let before = state.delivered_frames.load(Ordering::SeqCst);

    catalog.lost.store(1, Ordering::SeqCst);
    let resumed = eventually(
        || {
            state.delivered_frames.load(Ordering::SeqCst) >= before + 5
                && matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Capturing))
        },
        Duration::from_secs(8),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;
    let rewritten = std::fs::metadata(&first).and_then(|m| m.modified()).unwrap() != first_written;
    let saved = saved_frame_numbers(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(resumed, "the capture never resumed after the device loss");
    assert!(!rewritten, "frame_000001.fits was overwritten by the resumed run; saved {saved:?}");
}

/// The imaging camera is back but its paused capture has not resumed yet. Disconnect
/// used to refuse only `Capturing`/`Starting`, so it went ahead and left the pause behind
/// — for the resume to restart the capture on the camera being disconnected.
#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_the_camera_of_a_paused_capture_ends_the_capture() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;
    state.set_capture_state(CaptureState::Recovering).await;

    let answer = lifecycle::disconnect(&state, &id_of(&NEPTUNE)).await;
    let capture = state.capture_state().await;
    let plan_left = state.session_resume_plan.read().await.is_some();
    teardown(&state).await;

    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(capture, CaptureState::Idle, "the disconnect left the capture paused");
    assert!(!plan_left, "a resume plan outlived the disconnect that ended its capture");
}

/// A resume only ever continues a pause. Once a Stop or a Disconnect has ended it, the
/// supervisor's resume — decided a moment earlier — must find nothing to restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_resume_finds_nothing_to_restart_once_the_pause_was_ended() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;
    let plan = state.session_resume_plan.read().await.clone().unwrap();
    state.set_capture_state(CaptureState::Idle).await;

    let resumed = CaptureService::resume_capture(&state, &plan).await;
    let capture = state.capture_state().await;
    teardown(&state).await;

    assert!(
        matches!(resumed, Err(crate::server::error::ApiError::CaptureNotPaused)),
        "{resumed:?}"
    );
    assert_eq!(capture, CaptureState::Idle, "an ended capture was restarted");
}

/// The resume restores the plan's settings. Snapshotted at capture start, the plan undid an
/// exposure changed mid-session — or during the pause itself — without a word to the UI.
#[tokio::test(flavor = "multi_thread")]
async fn a_settings_edit_during_the_pause_is_what_the_capture_resumes_with() {
    use axum::extract::{Json, State};

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    start_capture_plan(&state, &NEPTUNE).await;
    let started_with = state.settings.read().await.exposure_us;
    let edited = started_with + 250_000;

    catalog.set(&[]);
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let request = crate::server::dto::UpdateSettingsRequest {
        exposure_us: Some(edited),
        ..Default::default()
    };
    let _ = crate::server::api::settings::update_settings(State(Arc::clone(&state)), Json(request)).await;
    let mut events = state.subscribe_events();

    catalog.set(&[NEPTUNE]);
    let resumed = eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Capturing)),
        Duration::from_secs(5),
    )
    .await;
    let exposure = state.settings.read().await.exposure_us;
    let announced = drain(&mut events).iter().any(|e| matches!(e, ServerEvent::SettingsUpdated));

    CaptureService::stop_capture(&state).await;
    eventually(
        || matches!(state.session.try_read().map(|s| s.state), Ok(CaptureState::Idle)),
        Duration::from_secs(10),
    )
    .await;
    teardown(&state).await;

    assert!(resumed, "the capture never resumed");
    assert_eq!(exposure, edited, "the resume put back the exposure the capture started with");
    assert!(announced, "restoring the session's settings must tell the clients");
}
