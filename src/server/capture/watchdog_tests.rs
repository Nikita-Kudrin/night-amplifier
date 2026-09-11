//! The stall ladder's first rung: a lost frame restarts the stream in place, and only a
//! run of them is treated as a camera fault.

use crate::camera::{
    Camera, CameraError, CameraInfo, CameraResult, CaptureConfig, GainPresets, ImageFormat,
    RawFrame, SensorType, FRAME_STALL_ALLOWANCE, TRANSFER_FLOOR_BYTES_PER_SEC,
};
use crate::server::capture::channel::{PipelineCapacities, QueueDepth};
use crate::server::capture::task::{run_capture_task, CaptureChannels, FrameNumbers};
use crate::server::capture::watchdog::*;
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CameraRole, ConnectedCameraInfo};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const MB: usize = 1_000_000;

fn neptune_info() -> CameraInfo {
    CameraInfo {
        name: "Neptune-C II".to_string(),
        max_width: 2712,
        max_height: 1538,
        ..Default::default()
    }
}

fn ares_info() -> CameraInfo {
    CameraInfo {
        name: "Ares-C PRO".to_string(),
        max_width: 3008,
        max_height: 3008,
        ..Default::default()
    }
}

fn raw16(exposure_us: u64) -> CaptureConfig {
    CaptureConfig {
        exposure_us,
        format: ImageFormat::Raw16,
        ..Default::default()
    }
}

/// The two field cases from 2026-09-07. Before, the Neptune waited 7.6 s and then lost
/// its handle; the Ares waited 30.8 s.
#[test]
fn stall_budget_at_the_field_cases() {
    let neptune = raw16(500_000);
    let budget = neptune.stall_budget(neptune.frame_bytes(&neptune_info()));
    assert_eq!(neptune.frame_bytes(&neptune_info()), 8_342_112);
    assert!(
        (Duration::from_millis(4_300)..Duration::from_millis(4_400)).contains(&budget),
        "Neptune 0.5 s: {budget:?}"
    );

    let ares = raw16(5_000_000);
    let budget = ares.stall_budget(ares.frame_bytes(&ares_info()));
    assert!(
        (Duration::from_millis(9_750)..Duration::from_millis(9_850)).contains(&budget),
        "Ares 5 s: {budget:?}"
    );
}

/// Swept rather than sampled: at every exposure and frame size the shim's budget
/// covers the exposure and the transfer, and the watchdog sits above it by the slack.
/// The watchdog firing first is the failure this whole ladder exists to remove.
#[test]
fn the_watchdog_stays_above_the_stall_budget_everywhere() {
    let exposures_us = [0, 1_000, 10_000, 100_000, 500_000, 1_000_000, 5_000_000, 30_000_000, 600_000_000];
    let frame_sizes_mb = [1, 8, 18, 36, 61, 130];
    for exposure_us in exposures_us {
        for size_mb in frame_sizes_mb {
            let config = CaptureConfig {
                exposure_us,
                format: ImageFormat::Raw8,
                ..Default::default()
            };
            let info = CameraInfo {
                max_width: (size_mb * MB) as u32,
                max_height: 1,
                ..Default::default()
            };
            let bytes = config.frame_bytes(&info);
            let budget = config.stall_budget(bytes);
            let transfer = Duration::from_secs_f64(bytes as f64 / TRANSFER_FLOOR_BYTES_PER_SEC as f64);
            let floor = Duration::from_micros(exposure_us) + FRAME_STALL_ALLOWANCE + transfer;
            assert!(
                budget + Duration::from_millis(1) >= floor,
                "{exposure_us} us / {size_mb} MB: budget {budget:?} below {floor:?}"
            );
            assert_eq!(
                capture_watchdog_timeout(&config, &info),
                budget + WATCHDOG_SLACK,
                "{exposure_us} us / {size_mb} MB"
            );
        }
    }
}

#[test]
fn binning_and_roi_shrink_the_frame_the_budget_is_sized_for() {
    let info = ares_info();
    let binned = CaptureConfig {
        bin: 2,
        ..raw16(1_000)
    };
    assert_eq!(binned.frame_dimensions(&info), (1504, 1504));
    let roi = CaptureConfig {
        roi: Some((10, 10, 640, 480)),
        ..raw16(1_000)
    };
    assert_eq!(roi.frame_bytes(&info), 640 * 480 * 2);
}

#[test]
fn stalls_escalate_only_when_consecutive() {
    let mut tracker = StallTracker::default();
    for _ in 1..STALL_ESCALATION {
        assert_eq!(tracker.stalled(), StallVerdict::RestartInPlace);
    }
    tracker.frame_delivered();
    for _ in 1..STALL_ESCALATION {
        assert_eq!(
            tracker.stalled(),
            StallVerdict::RestartInPlace,
            "a delivered frame must reset the run"
        );
    }
    assert_eq!(tracker.stalled(), StallVerdict::Escalate);
    assert_eq!(
        tracker.stalled(),
        StallVerdict::RestartInPlace,
        "an escalation starts a fresh run"
    );
}

/// A call the watchdog abandoned is still inside the SDK, and a reconnect must be able
/// to see that until it finally returns.
#[test]
fn an_abandoned_capture_stays_in_flight_until_it_returns() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let release = Arc::new(AtomicBool::new(false));
    let camera = ScriptedCamera::new(vec![Step::BlockUntil(Arc::clone(&release))]);

    let outcome = capture_frame_bounded(
        Box::new(camera),
        CaptureConfig::default(),
        1,
        Duration::from_millis(50),
        &state,
        CameraRole::Guide,
    );
    assert!(matches!(outcome, CaptureOutcome::TimedOut));
    let calls = &state.slot(CameraRole::Guide).sdk_calls;
    assert_eq!(calls.in_flight(), 1, "the stuck call is still in the SDK");
    assert_eq!(state.slot(CameraRole::Main).sdk_calls.in_flight(), 0);

    release.store(true, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while calls.in_flight() > 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(calls.in_flight(), 0, "returning must take it off the count");
}

// --- The capture loops ---------------------------------------------------------------

enum Step {
    Frame,
    Stall,
    BlockUntil(Arc<AtomicBool>),
}

/// Plays a script of outcomes, then keeps delivering frames and cancels whatever loop
/// is driving it once the script and `extra_frames` are both spent.
struct ScriptedCamera {
    info: CameraInfo,
    steps: Mutex<VecDeque<Step>>,
    extra_frames: AtomicUsize,
    on_done: Option<Box<dyn Fn() + Send + Sync>>,
    cancel_flag: Arc<AtomicBool>,
    frames: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    close_blocks_until: Option<Arc<AtomicBool>>,
}

impl ScriptedCamera {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            info: CameraInfo {
                name: "Scripted Camera".to_string(),
                max_width: 32,
                max_height: 24,
                sensor_type: SensorType::Mono,
                supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
                ..Default::default()
            },
            steps: Mutex::new(steps.into()),
            extra_frames: AtomicUsize::new(0),
            on_done: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(AtomicUsize::new(0)),
            closes: Arc::new(AtomicUsize::new(0)),
            close_blocks_until: None,
        }
    }

    fn then_frames(mut self, count: usize, on_done: impl Fn() + Send + Sync + 'static) -> Self {
        self.extra_frames = AtomicUsize::new(count);
        self.on_done = Some(Box::new(on_done));
        self
    }

    fn frame(&self) -> CameraResult<RawFrame> {
        self.frames.fetch_add(1, Ordering::SeqCst);
        let pixels = (self.info.max_width * self.info.max_height) as usize;
        Ok(RawFrame {
            data: vec![7u8; pixels].into(),
            width: self.info.max_width,
            height: self.info.max_height,
            format: ImageFormat::Raw8,
        })
    }
}

impl Camera for ScriptedCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }
    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets::default())
    }
    fn status(&self) -> CameraResult<crate::camera::CameraStatus> {
        Ok(Default::default())
    }
    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Ok(())
    }
    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Ok(())
    }
    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Ok(())
    }
    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<RawFrame> {
        let step = self.steps.lock().unwrap().pop_front();
        match step {
            Some(Step::Frame) => self.frame(),
            Some(Step::Stall) => Err(CameraError::ExposureTimeout(Duration::from_millis(1))),
            Some(Step::BlockUntil(release)) => {
                while !release.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                self.frame()
            }
            None => {
                let left = self.extra_frames.load(Ordering::SeqCst);
                if left == 0 {
                    if let Some(done) = &self.on_done {
                        done();
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    return Err(CameraError::Cancelled);
                }
                self.extra_frames.store(left - 1, Ordering::SeqCst);
                self.frame()
            }
        }
    }
    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }
    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }
    fn close(&mut self) -> CameraResult<()> {
        self.closes.fetch_add(1, Ordering::SeqCst);
        if let Some(release) = &self.close_blocks_until {
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    }
    fn provider_name(&self) -> &'static str {
        "Scripted"
    }
}

fn connected(info: &CameraInfo, role: CameraRole) -> ConnectedCameraInfo {
    ConnectedCameraInfo {
        id: "scripted_0".to_string(),
        provider: "Scripted".to_string(),
        index: 0,
        role,
        info: info.clone(),
    }
}

struct MainLoopRun {
    returned_handle: bool,
    frames: usize,
    delivered: u64,
    events: Vec<ServerEvent>,
    fault_streak: Option<u32>,
}

async fn drive_main_loop(steps: Vec<Step>, extra_frames: usize) -> MainLoopRun {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let mut events = state.subscribe_events();

    let camera = {
        let state = Arc::clone(&state);
        ScriptedCamera::new(steps).then_frames(extra_frames, move || state.request_cancel())
    };
    let frames = Arc::clone(&camera.frames);
    state
        .cameras
        .write()
        .await
        .insert("scripted_0".to_string(), connected(&camera.info, CameraRole::Main));
    let name = camera.info.name.clone();

    let (stacking_tx, stacking_rx) = mpsc::sync_channel(64);
    let (storage_tx, storage_rx) = mpsc::sync_channel(64);
    let channels = CaptureChannels {
        stacking_tx,
        storage_tx,
        stacking_depth: QueueDepth::default(),
        storage_depth: QueueDepth::default(),
        capacities: PipelineCapacities {
            stacking: 64,
            storage: 64,
            render: 1,
        },
    };
    let returned = {
        let state = Arc::clone(&state);
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            run_capture_task(state, Box::new(camera), channels, FrameNumbers::starting_at(1), rt)
        })
        .await
        .unwrap()
    };
    drop((stacking_rx, storage_rx));

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    let fault_streak = state
        .consecutive_watchdog_timeouts
        .lock()
        .unwrap()
        .get(&name)
        .map(|(count, _)| *count);
    MainLoopRun {
        returned_handle: returned.is_some(),
        frames: frames.load(Ordering::SeqCst),
        delivered: state.delivered_frames.load(Ordering::SeqCst),
        events: seen,
        fault_streak,
    }
}

fn has_error_or_disconnect(events: &[ServerEvent]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            ServerEvent::Error { .. } | ServerEvent::CameraDisconnected { .. }
        )
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_stall_costs_one_frame_not_the_camera() {
    let run = drive_main_loop(vec![Step::Frame, Step::Stall, Step::Frame], 3).await;

    assert!(run.returned_handle, "the loop must keep the handle through a stall");
    assert_eq!(run.frames, 5);
    assert_eq!(run.delivered, 5, "frames after the stall still reach the pipeline");
    assert_eq!(run.fault_streak, None, "an answered stall is not a camera fault");
    assert!(!has_error_or_disconnect(&run.events), "nothing to tell the user");
}

/// Two stalls with a frame between them are two separate hiccups, not a run.
#[tokio::test(flavor = "multi_thread")]
async fn stalls_separated_by_a_frame_never_escalate() {
    let mut steps = Vec::new();
    for _ in 0..4 {
        steps.extend([Step::Stall, Step::Stall, Step::Frame]);
    }
    let run = drive_main_loop(steps, 1).await;

    assert!(run.returned_handle);
    assert_eq!(run.fault_streak, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_of_stalls_hands_the_camera_to_recovery() {
    let steps = (0..STALL_ESCALATION).map(|_| Step::Stall).collect();
    let run = drive_main_loop(steps, 5).await;

    assert!(!run.returned_handle, "an escalated stall ends as a device fault");
    assert_eq!(run.frames, 0);
    assert_eq!(run.fault_streak, Some(1), "recorded once, as one incident");
    assert!(!has_error_or_disconnect(&run.events), "recovery decides what the user sees");
}

async fn drive_guide_loop(steps: Vec<Step>, extra_frames: usize) -> (bool, usize) {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let cancel = Arc::new(AtomicBool::new(false));
    let camera = {
        let cancel = Arc::clone(&cancel);
        ScriptedCamera::new(steps).then_frames(extra_frames, move || cancel.store(true, Ordering::SeqCst))
    };
    let frames = Arc::clone(&camera.frames);
    let info = connected(&camera.info, CameraRole::Guide);
    let rt = tokio::runtime::Handle::current();
    let returned = tokio::task::spawn_blocking(move || {
        crate::server::capture::guide_task::run(&state, &info, Box::new(camera), &cancel, None, &rt)
    })
    .await
    .unwrap();
    (returned.is_some(), frames.load(Ordering::SeqCst))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guide_loop_restarts_a_stalled_stream_in_place() {
    let (kept_handle, frames) = drive_guide_loop(vec![Step::Stall, Step::Stall, Step::Frame], 2).await;
    assert!(kept_handle);
    assert_eq!(frames, 3);
}

/// A run of stalls means the bus is wedged — exactly when a vendor close can hang (the
/// field log has a ~3 min freeze inside PlayerOne's SDK). The escalation closes the
/// handle synchronously on the capture thread, unbounded, so the pipeline never ends,
/// `return_from_capture` never runs, and recovery never starts.
#[tokio::test(flavor = "multi_thread")]
async fn an_escalated_stall_does_not_wait_on_a_hung_close() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    let release = Arc::new(AtomicBool::new(false));
    let mut camera = ScriptedCamera::new((0..STALL_ESCALATION).map(|_| Step::Stall).collect());
    camera.close_blocks_until = Some(Arc::clone(&release));
    state
        .cameras
        .write()
        .await
        .insert("scripted_0".to_string(), connected(&camera.info, CameraRole::Main));

    let (stacking_tx, _stacking_rx) = mpsc::sync_channel(4);
    let (storage_tx, _storage_rx) = mpsc::sync_channel(4);
    let channels = CaptureChannels {
        stacking_tx,
        storage_tx,
        stacking_depth: QueueDepth::default(),
        storage_depth: QueueDepth::default(),
        capacities: PipelineCapacities { stacking: 4, storage: 4, render: 1 },
    };
    let rt = tokio::runtime::Handle::current();
    let task = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        move || run_capture_task(state, Box::new(camera), channels, FrameNumbers::starting_at(1), rt)
    });

    let ended = tokio::time::timeout(Duration::from_secs(2), task).await;
    release.store(true, Ordering::SeqCst);
    assert!(ended.is_ok(), "the capture task is stuck inside close(); recovery cannot start");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guide_loop_hands_a_run_of_stalls_to_recovery() {
    let steps = (0..STALL_ESCALATION).map(|_| Step::Stall).collect();
    let (kept_handle, frames) = drive_guide_loop(steps, 2).await;
    assert!(!kept_handle);
    assert_eq!(frames, 0);
}
