use super::*;
use night_amplifier_core::camera::testing::{CameraControls, FakeCamera};
use night_amplifier_core::camera::{CameraInfo, ImageFormat, SensorType};
use crate::services::CaptureService;
use crate::state::{CameraCaptureProfile, CaptureState, Resolution, StreamKind, ViewerGuard};

/// A cooled guide camera with a dew heater that raises `stop` on its `stop_after`th
/// frame, so a test drives an exact number of iterations rather than racing a clock.
fn counting_camera(stop_after: usize, stop: Arc<AtomicBool>) -> FakeCamera {
    FakeCamera::new("Mock Guide Camera")
        .cooled()
        .with_info(|info| info.has_dew_heater = true)
        .reporting_temperature(20.0)
        .on_frame(move |delivered| {
            if delivered >= stop_after {
                stop.store(true, Ordering::SeqCst);
            }
        })
}

fn guide_camera_info() -> ConnectedCameraInfo {
    ConnectedCameraInfo {
        id: "mock_0".to_string(),
        provider: "Mock".to_string(),
        index: 0,
        role: CameraRole::Guide,
        info: CameraInfo {
            name: "Mock Guide Camera".to_string(),
            max_width: 32,
            max_height: 24,
            sensor_type: SensorType::Mono,
            supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
            // Matched to `counting_camera`: `config_overrides` strips the cooler
            // fields from a config bound for a camera that says it has none.
            has_cooler: true,
            has_dew_heater: true,
            min_temp_c: Some(-40.0),
            max_temp_c: Some(30.0),
            ..Default::default()
        },
    }
}

/// Run the guide loop for `frames` exposures on a blocking thread, and report how
/// many the camera actually delivered.
async fn drive_guide_loop(state: &Arc<AppState>, frames: usize) -> usize {
    let stop = Arc::new(AtomicBool::new(false));
    let camera = counting_camera(frames, Arc::clone(&stop));
    let controls = camera.controls();
    let info = guide_camera_info();
    let state = Arc::clone(state);
    let rt = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || {
        run(&state, &info, Box::new(camera), &stop, None, &rt);
    })
    .await
    .expect("guide loop panicked");

    controls.frames.load(Ordering::SeqCst)
}

/// The requirement in one test: a guide camera nobody is looking at must not pay for
/// the preview pipeline. `frame_counter` only advances inside the watched branch, so
/// it staying at zero is proof the render and the encode never ran — while the camera
/// really did expose the frames.
#[tokio::test]
async fn an_unwatched_guide_stream_is_never_rendered() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let captured = drive_guide_loop(&state, 4).await;

    assert_eq!(captured, 4, "the camera should still have exposed frames");
    assert_eq!(
        state.guide_stream.frame_counter(),
        0,
        "the guide stream advanced a frame with nobody watching"
    );
    assert!(
        state.guide_stream.get_latest_raw_frame().is_none(),
        "an unwatched guide stream published a rendered frame"
    );
    for kind in StreamKind::all() {
        assert!(
            state.guide_stream.payload(kind, 1).is_none(),
            "{kind:?} was encoded with nobody watching the guide stream"
        );
    }
}

/// The other half: with a viewer registered the same loop does render and publish,
/// so the gate is throttling on demand rather than being permanently shut.
#[tokio::test]
async fn a_watched_guide_stream_renders_every_frame() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let _viewer = ViewerGuard::new(Arc::clone(&state.guide_stream), StreamKind::Jpeg);

    let captured = drive_guide_loop(&state, 3).await;

    assert_eq!(captured, 3);
    assert_eq!(
        state.guide_stream.frame_counter(),
        3,
        "a watched guide stream must publish every frame it captures"
    );
    assert!(state.guide_stream.get_latest_raw_frame().is_some());
    let payload = state
        .guide_stream
        .payload(StreamKind::Jpeg, 3)
        .expect("a watched guide stream published no JPEG");
    let height = u32::from_le_bytes(payload[8..12].try_into().unwrap());
    assert!(height <= 1440, "the guide JPEG ignored Streaming Resolution: height {height}");
}

/// The loop snapshots settings before it exposes, so a Streaming Resolution change made
/// during the exposure exists only in the live settings — and the frame that exposure
/// renders must already follow it.
#[tokio::test]
async fn a_resolution_change_during_an_exposure_applies_to_that_frame() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.update(|s| s.streaming_resolution = Resolution::Native);
    let _viewer = ViewerGuard::new(Arc::clone(&state.guide_stream), StreamKind::Jpeg);

    let stop = Arc::new(AtomicBool::new(false));
    let editor = Arc::clone(&state);
    let camera = counting_camera(1, Arc::clone(&stop));
    let camera = camera.sized(2400, 1600).during_exposure(move || {
        editor.settings.update(|s| s.streaming_resolution = Resolution::Hd1080);
    });
    let info = guide_camera_info();
    let loop_state = Arc::clone(&state);
    let rt = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        run(&loop_state, &info, Box::new(camera), &stop, None, &rt);
    })
    .await
    .expect("guide loop panicked");

    let payload = state
        .guide_stream
        .payload(StreamKind::Jpeg, 1)
        .expect("a watched guide stream published no JPEG");
    let size = (
        u32::from_le_bytes(payload[4..8].try_into().unwrap()),
        u32::from_le_bytes(payload[8..12].try_into().unwrap()),
    );
    assert_eq!(size, (1620, 1080), "the guide JPEG followed the exposure's snapshot");
}

/// A viewer on the guide stream must not make the *main* stream produce anything —
/// the two are independent producers, and a guide frame advancing the main counter
/// would invalidate the imaging camera's payloads on every guide exposure.
#[tokio::test]
async fn the_guide_loop_never_touches_the_main_stream() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let _viewer = ViewerGuard::new(Arc::clone(&state.guide_stream), StreamKind::Jpeg);
    drive_guide_loop(&state, 2).await;

    assert_eq!(state.main_stream.frame_counter(), 0);
    assert!(state.main_stream.get_latest_raw_frame().is_none());
}

/// Raw saving sits above both early exits: an unwatched guide camera with no solve
/// target still writes the subs the user asked for.
#[tokio::test]
async fn guide_raw_frames_are_saved_even_when_nothing_is_watching() {
    let (state, disk_writer) = AppState::new_for_testing();
    let state = Arc::new(state);
    std::thread::spawn(move || disk_writer.run());

    state.settings.update(|s| s.raw_frame_saving.guide = true);

    drive_guide_loop(&state, 3).await;

    let dir = guide_session_dir(&state)
        .await
        .expect("no guide raw-frame directory was opened");
    assert!(
        dir.file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-guide"),
        "guide frames went to {dir:?}, which does not name the guide mode"
    );

    // The writer runs on its own thread; give it a moment to drain the queue.
    for _ in 0..50 {
        let written = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        if written >= 3 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("guide raw frames never reached {dir:?}");
}

/// A dropout rejoins the folder the interrupted session was filling, so the resumed
/// run has to carry on its numbering: the writer names files `frame_{:06}.fits`, and
/// restarting at 1 wrote straight over the frames already in there.
#[tokio::test]
async fn a_resumed_guide_session_appends_to_the_folder_it_rejoined() {
    let (state, disk_writer) = AppState::new_for_testing();
    let state = Arc::new(state);
    std::thread::spawn(move || disk_writer.run());

    state.settings.update(|s| s.raw_frame_saving.guide = true);

    drive_guide_loop(&state, 3).await;
    let dir = guide_session_dir(&state)
        .await
        .expect("no guide raw-frame directory was opened");
    wait_for_files(&dir, 3).await;

    // Second run, resuming the way `spawn_loop` does — from what the slot parked.
    let resume = state.slot(CameraRole::Guide).raw_session();
    assert_eq!(
        resume.as_ref().map(|r| r.next_frame),
        Some(4),
        "the interrupted run must park the number to carry on from"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let camera = counting_camera(2, Arc::clone(&stop));
    let info = guide_camera_info();
    {
        let state = Arc::clone(&state);
        let rt = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            run(&state, &info, Box::new(camera), &stop, resume, &rt);
        })
        .await
        .expect("guide loop panicked");
    }
    wait_for_files(&dir, 5).await;

    for n in 1..=5u64 {
        let path = dir.join(format!("frame_{n:06}.fits"));
        assert!(path.exists(), "{path:?} is missing — the resume overwrote it");
    }
}

/// Drive `frames` exposures and hand back everything the loop asked of the camera.
async fn drive_and_log(state: &Arc<AppState>, frames: usize) -> Arc<CameraControls> {
    let stop = Arc::new(AtomicBool::new(false));
    let camera = counting_camera(frames, Arc::clone(&stop));
    let log = camera.controls();
    let info = guide_camera_info();
    let state = Arc::clone(state);
    let rt = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || {
        run(&state, &info, Box::new(camera), &stop, None, &rt);
    })
    .await
    .expect("guide loop panicked");

    log
}

/// `CaptureConfig` has no dew-heater field, so a queued call is the only way the
/// switch can reach a camera whose handle the loop holds for the whole connection.
#[tokio::test]
async fn the_loop_runs_hardware_calls_queued_against_its_slot() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    state.slot(CameraRole::Guide).queue_op(CameraOp::SetDewHeater {
        enabled: true,
        power: 65,
    });

    let log = drive_and_log(&state, 2).await;

    assert_eq!(*log.dew_heater.lock().unwrap(), vec![(true, 65)]);
    assert!(
        state.slot(CameraRole::Guide).drain_ops().is_empty(),
        "the loop must consume what it applied"
    );
}

/// The monitor is the imaging camera's status source and cannot check out a handle
/// the guide loop never returns, so the loop reports for itself — otherwise the
/// guide camera's temperature readout never updates for the whole session.
#[tokio::test]
async fn the_loop_publishes_its_own_status_samples() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    let log = drive_and_log(&state, 2).await;

    assert!(
        log.status_reads.load(Ordering::SeqCst) >= 1,
        "the guide loop never read the sensor"
    );
    let status = state
        .get_camera_status("Mock Guide Camera")
        .expect("no status was broadcast for the guide camera");
    assert_eq!(status.temperature_c, 20.0);
}

/// The setpoint the loop commands has to walk toward the target at
/// `RAMP_RATE_C_PER_MIN`, not jump to it. Pushing the final target per frame is how
/// the 5 °C/min limit stopped applying to one of the two cameras on the rig.
#[tokio::test]
async fn the_cooler_setpoint_is_ramped_not_snapped() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.update(|settings| {
        settings.guide_camera.cooler_enabled = true;
        settings.guide_camera.target_temp_c = Some(-15.0);
    });

    let log = drive_and_log(&state, 1).await;

    let first = log.setpoints.lock().unwrap()[0];
    // The sensor reports 20 °C, so a ramp starts there. Under the test-time rate the
    // first step may already reach the target; what must never happen is the loop
    // commanding the target before it has looked at the sensor at all.
    assert!(
        first.is_some_and(|sp| (-15.0..=20.0).contains(&sp)),
        "expected a setpoint between the sensor and the target, got {first:?}"
    );
}

/// The arithmetic, at a controlled clock: `RAMP_RATE_C_PER_MIN` is shadowed in test
/// builds, so the step is derived from the constant rather than hard-coded, and the
/// interval is short enough that even the test rate cannot reach the target.
#[test]
fn the_ramp_walks_from_the_sensor_toward_the_target() {
    use crate::camera::RAMP_RATE_C_PER_MIN;

    let profile = CameraCaptureProfile {
        cooler_enabled: true,
        target_temp_c: Some(-15.0),
        ..Default::default()
    };
    let mut cooler = GuideCooler::default();
    let start = std::time::Instant::now();

    // First call installs the ramp at the sensor temperature; it has not moved yet.
    assert_eq!(cooler.setpoint(&profile, Some(20.0), start), Some(20.0));

    let dt = Duration::from_millis(100);
    let expected = 20.0 - dt.as_secs_f64() * RAMP_RATE_C_PER_MIN / 60.0;
    assert!(
        expected > -15.0,
        "the test interval must stay short of the target to prove a ramp"
    );
    let stepped = cooler
        .setpoint(&profile, Some(20.0), start + dt)
        .expect("a ramping cooler must command a setpoint");
    assert_eq!(stepped, expected.round());
    assert!(stepped < 20.0 && stepped > -15.0, "got {stepped}");
}

/// Moving the target restarts the ramp from where the sensor actually is, rather
/// than continuing from a setpoint aimed somewhere else.
#[test]
fn a_new_target_restarts_the_ramp_at_the_sensor() {
    let mut profile = CameraCaptureProfile {
        cooler_enabled: true,
        target_temp_c: Some(-15.0),
        ..Default::default()
    };
    let mut cooler = GuideCooler::default();
    let t0 = std::time::Instant::now();
    cooler.setpoint(&profile, Some(20.0), t0);
    cooler.setpoint(&profile, Some(5.0), t0 + Duration::from_secs(60));

    profile.target_temp_c = Some(-5.0);
    assert_eq!(
        cooler.setpoint(&profile, Some(5.0), t0 + Duration::from_secs(61)),
        Some(5.0),
        "the ramp should have restarted at the sensor, not resumed"
    );
}

/// Nothing to ramp: the config's own target stands.
#[test]
fn a_cooler_with_nothing_to_ramp_leaves_the_config_alone() {
    let mut cooler = GuideCooler::default();
    let now = std::time::Instant::now();

    let off = CameraCaptureProfile {
        cooler_enabled: false,
        target_temp_c: Some(-15.0),
        ..Default::default()
    };
    assert_eq!(cooler.setpoint(&off, Some(20.0), now), None);

    let no_target = CameraCaptureProfile {
        cooler_enabled: true,
        target_temp_c: None,
        ..Default::default()
    };
    assert_eq!(cooler.setpoint(&no_target, Some(20.0), now), None);
}

/// Fast mode is the switch a user flips to accept a snap, and the UI warns while it
/// is on — so the loop must leave that config's target alone.
#[tokio::test]
async fn fast_mode_leaves_the_target_alone() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    state.settings.update(|settings| {
        settings.guide_camera.cooler_enabled = true;
        settings.guide_camera.target_temp_c = Some(-15.0);
        settings.guide_camera.cooler_fast_mode = true;
    });

    let log = drive_and_log(&state, 1).await;

    assert_eq!(log.setpoints.lock().unwrap()[0], Some(-15.0));
}

/// The directory the guide loop is currently filling, if any.
async fn guide_session_dir(state: &Arc<AppState>) -> Option<std::path::PathBuf> {
    state.slot(CameraRole::Guide).raw_session().map(|r| r.dir)
}

async fn wait_for_files(dir: &std::path::Path, want: usize) {
    for _ in 0..100 {
        if std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0) >= want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "only {} of {want} files reached {dir:?}",
        std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    );
}

/// Connect a guide camera the way `lifecycle::connect` leaves things — registered in
/// the map with a live handle in its slot — and start its loop through the real
/// entry point, so the cancel token and the phase are the production ones.
///
/// The camera is paced rather than free-running flat out: these tests watch a loop
/// they do not step, and an instant `capture()` fills a directory faster than the
/// assertions can read it.
async fn connect_and_start_guide(state: &Arc<AppState>) {
    let never_stops = Arc::new(AtomicBool::new(false));
    let camera = counting_camera(usize::MAX, never_stops).stuck_for(Duration::from_millis(20));
    let info = guide_camera_info();

    state.roster.install(info.clone(), false);
    *state
        .slot(CameraRole::Guide)
        .handle
        .lock()
        .expect("camera handle mutex poisoned") = Some(Box::new(camera));

    start(state, &info);
}

/// Wait until the loop is up, so a test never asserts against a loop still queued on
/// the tokio task `start` spawns.
async fn wait_until_running(state: &Arc<AppState>) {
    for _ in 0..200 {
        if state.guide_loop_running() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the guide loop never started");
}

fn files_in(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

/// The bug this fixes: raw saving sits above every gate in the loop, and the loop
/// only ended on disconnect — so a guide camera asked to save subs kept filling its
/// folder after the user pressed Stop, with nothing short of unplugging it to stop.
#[tokio::test]
async fn stopping_the_guide_camera_stops_its_raw_frame_writing() {
    let (state, disk_writer) = AppState::new_for_testing();
    let state = Arc::new(state);
    std::thread::spawn(move || disk_writer.run());

    state.settings.update(|s| s.raw_frame_saving.guide = true);
    connect_and_start_guide(&state).await;
    wait_until_running(&state).await;

    let dir = loop_until_session_dir(&state).await;
    wait_for_files(&dir, 2).await;

    assert!(
        CaptureService::stop(&state, CameraRole::Guide).await,
        "stopping a running guide camera must report that it was running"
    );
    assert!(!state.guide_loop_running());

    // The writer queue drains asynchronously, so settle before taking the reading
    // that the next one is compared against.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after_stop = files_in(&dir);
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert_eq!(
        files_in(&dir),
        after_stop,
        "the guide camera kept writing frames into {dir:?} after it was stopped"
    );
}

/// A deliberate stop ends the observation. Keeping the resume record would send the
/// next Start back into the old folder, where it would carry on the numbering into
/// frames from a different session.
#[tokio::test]
async fn a_stopped_guide_camera_does_not_rejoin_its_old_folder() {
    let (state, disk_writer) = AppState::new_for_testing();
    let state = Arc::new(state);
    std::thread::spawn(move || disk_writer.run());

    state.settings.update(|s| s.raw_frame_saving.guide = true);
    connect_and_start_guide(&state).await;
    wait_until_running(&state).await;
    let first = loop_until_session_dir(&state).await;
    wait_for_files(&first, 1).await;

    CaptureService::stop(&state, CameraRole::Guide).await;
    assert!(
        guide_session_dir(&state).await.is_none(),
        "a deliberate stop must not park a folder for a later run to rejoin"
    );

    // Restarting opens a folder of its own rather than appending to the first.
    CaptureService::start(&state, None, CameraRole::Guide)
        .await
        .expect("a stopped guide camera must be startable again");
    wait_until_running(&state).await;
    let second = loop_until_session_dir(&state).await;
    wait_for_files(&second, 1).await;
    assert_ne!(first, second, "the restarted run rejoined the stopped one");
    assert!(second.join("frame_000001.fits").exists());

    CaptureService::stop(&state, CameraRole::Guide).await;
}

/// Stop is idempotent: the disconnect path calls it too, and a second press must not
/// claim to have stopped something that was already stopped.
#[tokio::test]
async fn stopping_a_guide_camera_that_is_not_running_reports_nothing_to_stop() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    connect_and_start_guide(&state).await;
    wait_until_running(&state).await;

    assert!(CaptureService::stop(&state, CameraRole::Guide).await);
    assert!(!CaptureService::stop(&state, CameraRole::Guide).await);
}

/// The imaging camera and the guide camera are stopped separately, on purpose: a
/// guide camera exists to keep solving and framing while no capture is running.
#[tokio::test]
async fn stopping_the_capture_leaves_the_guide_camera_running() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);
    connect_and_start_guide(&state).await;
    wait_until_running(&state).await;
    state.set_capture_state(CaptureState::Capturing);

    assert!(CaptureService::stop(&state, CameraRole::Main).await);

    assert!(
        state.guide_loop_running(),
        "stopping the capture stopped the guide camera with it"
    );
    CaptureService::stop(&state, CameraRole::Guide).await;
}

/// The directory is opened by the first frame that needs one, so a test that reads it
/// straight after the loop starts is racing the first exposure.
async fn loop_until_session_dir(state: &Arc<AppState>) -> std::path::PathBuf {
    for _ in 0..200 {
        if let Some(dir) = guide_session_dir(state).await {
            return dir;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no guide raw-frame directory was opened");
}

/// Off by default, and it must stay a separate decision from the imaging switches.
#[tokio::test]
async fn guide_raw_saving_is_off_unless_asked_for() {
    let (state, _dw) = AppState::new_for_testing();
    let state = Arc::new(state);

    // Saving every imaging mode must not implicitly save guide frames.
    state.settings.update(|settings| {
        settings.raw_frame_saving.live_view = true;
        settings.raw_frame_saving.wanderer = true;
        settings.raw_frame_saving.stacking = true;
    });

    drive_guide_loop(&state, 2).await;

    assert!(
        guide_session_dir(&state).await.is_none(),
        "a guide session was opened without the guide switch"
    );
}
