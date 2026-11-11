//! A camera parked between captures, after 2026-11-03: the Ares-C PRO was handed back with
//! its video stream still running, read -300 °C within 18 s, and gave no frame on the next
//! Start until it was reopened — once after 18 s, once after three minutes.

use std::sync::atomic::Ordering;
use std::time::Duration;

use std::sync::{Arc, Mutex};

use super::lifecycle::{self, DisconnectCause};
use super::recovery_tests::{connect, drain, rig, teardown, wait_recovered, FakeCatalog, ARES, NEPTUNE};
use super::tests::eventually;
use crate::camera::health as camera_health;
use crate::events::ServerEvent;
use crate::services::CaptureService;
use crate::state::{AppState, CameraPhase, CameraRole, CaptureState, RecoveryStatus};

// --- The stream ends with the capture -------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_capture_parks_its_camera_with_the_stream_ended() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;

    CaptureService::start_capture(&state, None).await.unwrap();
    assert!(eventually(|| state.stats.delivered() >= 2, Duration::from_secs(5)).await);
    assert_eq!(catalog.camera.stream_ends.load(Ordering::SeqCst), 0, "a capturing camera keeps its stream");
    CaptureService::stop_capture(&state).await;
    let parked = eventually(
        || matches!(state.capture_state(), CaptureState::Idle) && state.slot(CameraRole::Main).holds_handle(),
        Duration::from_secs(10),
    )
    .await;

    let ends = catalog.camera.stream_ends.load(Ordering::SeqCst);
    teardown(&state).await;
    assert!(parked, "the handle never came back to the slot");
    assert_eq!(ends, 1, "the parked camera was left streaming");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_guide_parks_its_camera_with_the_stream_ended() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| catalog.camera.frames.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await);

    assert!(CaptureService::stop(&state, CameraRole::Guide).await);
    let parked = eventually(|| state.slot(CameraRole::Guide).holds_handle(), Duration::from_secs(5)).await;

    let ends = catalog.camera.stream_ends.load(Ordering::SeqCst);
    teardown(&state).await;
    assert!(parked, "the guide handle never came back to the slot");
    assert_eq!(ends, 1, "the stopped guide camera was left streaming");
}

/// The stop is one more vendor call, on a camera that may have stopped answering: past
/// its bound the handle is given up on and reopened, never parked half-stopped.
#[tokio::test(flavor = "multi_thread")]
async fn a_camera_that_does_not_answer_the_stop_of_its_stream_is_reopened() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    CaptureService::start_capture(&state, None).await.unwrap();
    assert!(eventually(|| state.stats.delivered() >= 2, Duration::from_secs(5)).await);
    catalog.camera.end_stream_delay_ms.store(1_000, Ordering::SeqCst);

    CaptureService::stop_capture(&state).await;
    let reopened = eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await;
    catalog.camera.end_stream_delay_ms.store(0, Ordering::SeqCst);
    let recovered = wait_recovered(&state, CameraRole::Main).await;

    teardown(&state).await;
    assert!(reopened, "a handle that did not answer its stop was parked");
    assert!(recovered, "the reopen did not bring the camera back");
}

// --- A first frame that stalls --------------------------------------------------------

/// Restarting the stream in place cost two stall budgets on 2026-11-03 and never brought
/// a first frame back; the reopen did, every time it was tried.
#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_first_frame_reopens_the_camera_without_restarting_in_place() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    catalog.camera.stalls.store(1, Ordering::SeqCst);

    CaptureService::start_capture(&state, None).await.unwrap();
    let capturing = eventually(
        || state.stats.delivered() >= 2 && matches!(state.capture_state(), CaptureState::Capturing),
        Duration::from_secs(5),
    )
    .await;
    let opens = catalog.open_calls.load(Ordering::SeqCst);

    CaptureService::stop_capture(&state).await;
    eventually(|| matches!(state.capture_state(), CaptureState::Idle), Duration::from_secs(10)).await;
    teardown(&state).await;
    assert!(capturing, "the reopen did not resume the capture");
    assert_eq!(opens, 2, "the stalled first frame was restarted in place, not reopened");
}

// --- The guide camera through a reopen ------------------------------------------------

async fn stop_guide(state: &Arc<AppState>) {
    assert!(CaptureService::stop(state, CameraRole::Guide).await);
    assert!(eventually(|| state.slot(CameraRole::Guide).holds_handle(), Duration::from_secs(5)).await);
}

/// Frames the guide camera takes over `window`, once it has settled.
async fn guide_frames_over(catalog: &FakeCatalog, window: Duration) -> usize {
    let before = catalog.camera.frames.load(Ordering::SeqCst);
    tokio::time::sleep(window).await;
    catalog.camera.frames.load(Ordering::SeqCst) - before
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_guide_is_not_restarted_by_its_reopen() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    stop_guide(&state).await;

    lifecycle::finalize_disconnect(&state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let recovered = wait_recovered(&state, CameraRole::Guide).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;
    let registered = state.guide_loops.is_registered();
    let phase = state.camera_phase(CameraRole::Guide);

    teardown(&state).await;
    assert!(recovered);
    assert!(!registered, "the reopen started a guide loop the observer had stopped");
    assert_eq!(frames, 0, "the stopped guide camera streamed after its reopen");
    assert_eq!(phase, CameraPhase::Idle);
}

/// The loop that faults has unregistered itself before recovery runs: what it was doing
/// is read off the phase it leaves behind.
#[tokio::test(flavor = "multi_thread")]
async fn a_guide_loop_that_faulted_resumes_after_its_reopen() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| catalog.camera.frames.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await);

    catalog.camera.lost.store(1, Ordering::SeqCst);
    let reopened = eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await;
    let recovered = wait_recovered(&state, CameraRole::Guide).await;
    let resumed = eventually(|| state.guide_loops.is_running(), Duration::from_secs(5)).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;

    teardown(&state).await;
    assert!(reopened && recovered);
    assert!(resumed, "a guide loop that was running did not resume after its reopen");
    assert!(frames > 0, "the resumed guide loop takes no frames");
}

/// The 2026-11-03 path: the guide was stopped, and closed only so the failing imaging
/// camera had the SDK to itself. Its reopen must leave it stopped, and the reopen counts
/// as judged — nothing is left waiting on a guide frame that will never come.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_guide_closed_for_its_sibling_stays_stopped() {
    let catalog = FakeCatalog::with(&[ARES, NEPTUNE]);
    let state = rig(&catalog);
    let ares = catalog.controls_for(&ARES);
    connect(&state, &ARES, CameraRole::Main).await;
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    stop_guide(&state).await;

    // The Ares frames once the Neptune has been closed for it.
    ares.stalls.store(usize::MAX, Ordering::SeqCst);
    let neptune_closes = catalog.camera.closes.load(Ordering::SeqCst);
    let unwedge = {
        let catalog = Arc::clone(&catalog);
        let ares = Arc::clone(&ares);
        tokio::spawn(async move {
            while catalog.camera.closes.load(Ordering::SeqCst) == neptune_closes {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            ares.stalls.store(0, Ordering::SeqCst);
        })
    };
    CaptureService::start_capture(&state, None).await.unwrap();
    let capturing = eventually(
        || state.stats.delivered() >= 2 && matches!(state.capture_state(), CaptureState::Capturing),
        Duration::from_secs(10),
    )
    .await;
    let isolated = catalog.camera.closes.load(Ordering::SeqCst) > neptune_closes;
    let guide_back = wait_recovered(&state, CameraRole::Guide).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;
    let registered = state.guide_loops.is_registered();

    unwedge.abort();
    CaptureService::stop_capture(&state).await;
    eventually(|| matches!(state.capture_state(), CaptureState::Idle), Duration::from_secs(10)).await;
    teardown(&state).await;
    assert!(capturing, "the Ares never framed");
    assert!(isolated, "the Neptune was never closed for the Ares");
    assert!(guide_back, "the Neptune did not come back");
    assert!(!registered, "the stopped guide's loop was restarted by its reopen");
    assert_eq!(frames, 0, "the stopped guide camera streamed after its reopen");
}

// --- A Start while the camera is being reopened ---------------------------------------

#[tokio::test]
async fn a_start_during_recovery_keeps_the_frameless_run() {
    let (state, _dw) = AppState::new_for_testing();
    camera_health::record_frameless_open(&state, CameraRole::Main, ARES.name);
    camera_health::record_frameless_open(&state, CameraRole::Main, ARES.name);
    let _ = state.slot(CameraRole::Main).begin_suspend();

    camera_health::forget_frameless_opens_for_start(&state, CameraRole::Main, ARES.name);
    let during = camera_health::frameless_opens(&state, CameraRole::Main, ARES.name);
    state.slot(CameraRole::Main).end_recovery();
    camera_health::forget_frameless_opens_for_start(&state, CameraRole::Main, ARES.name);
    let after = camera_health::frameless_opens(&state, CameraRole::Main, ARES.name);

    assert_eq!(during, 2, "a Start into a reopen under way restarted its run");
    assert_eq!(after, 0, "a Start on a camera that is not being reopened is a fresh attempt");
}

// --- A parked camera that stops answering ---------------------------------------------

/// The night's second outage, caught before the Start: the idle Ares read -300 °C for
/// 2.5 minutes, and only the Start's stall reopened it.
#[tokio::test(flavor = "multi_thread")]
async fn a_parked_camera_that_stops_answering_is_reopened_before_the_next_start() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    // The monitor's own reading, not the install's.
    let installed = catalog.camera.status_reads.load(Ordering::SeqCst);
    let answered = eventually(|| catalog.camera.status_reads.load(Ordering::SeqCst) > installed, Duration::from_secs(5)).await;

    *catalog.camera.temperature_c.lock().unwrap() = Some(-300.0);
    let reopened = eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(20)).await;
    *catalog.camera.temperature_c.lock().unwrap() = None;
    let recovered = wait_recovered(&state, CameraRole::Main).await;
    let phase = state.camera_phase(CameraRole::Main);

    teardown(&state).await;
    assert!(answered, "the monitor never read the camera");
    assert!(reopened, "a parked camera reading -300 °C was left alone");
    assert!(recovered, "the reopen did not bring the camera back");
    assert_eq!(phase, CameraPhase::Idle);
}

// --- Telling the observer -------------------------------------------------------------

/// Each of the night's reopens succeeded at its first attempt, so "Reconnecting" was
/// never sent and three minutes of "Waiting for frames" went unexplained. A reopen after
/// `NOTICE_AFTER` without a frame says so, and the frame that ends it clears it.
#[tokio::test(flavor = "multi_thread")]
async fn a_reopen_that_follows_a_frameless_spell_says_so_until_the_camera_frames() {
    use crate::camera::reconnect::NOTICE_AFTER;

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();
    catalog.camera.stalls.store(1, Ordering::SeqCst);
    // Gone from the bus past the quiet spell, so the reopen lands after it.
    catalog.set(&[]);
    CaptureService::start_capture(&state, None).await.unwrap();
    tokio::time::sleep(NOTICE_AFTER + Duration::from_millis(100)).await;
    catalog.set(&[NEPTUNE]);

    let capturing = eventually(
        || state.stats.delivered() >= 2 && matches!(state.capture_state(), CaptureState::Capturing),
        Duration::from_secs(5),
    )
    .await;
    let seen = drain(&mut events);
    let status = state.recovery_status.get(CameraRole::Main);

    CaptureService::stop_capture(&state).await;
    eventually(|| matches!(state.capture_state(), CaptureState::Idle), Duration::from_secs(10)).await;
    teardown(&state).await;
    assert!(capturing, "the camera never came back");
    let told = seen.iter().position(|e| {
        matches!(e, ServerEvent::CameraUnresponsive { role: CameraRole::Main, opens: 1, .. })
    });
    let told = told.unwrap_or_else(|| panic!("the frameless spell was never announced: {seen:?}"));
    assert!(
        seen[told..].iter().any(|e| matches!(e, ServerEvent::CameraRecoveryCleared { role: CameraRole::Main })),
        "the frame that ended it did not clear the notice"
    );
    assert_eq!(status, RecoveryStatus::None);
}

#[tokio::test]
async fn a_frameless_run_is_announced_only_past_the_quiet_spell() {
    let (state, _dw) = AppState::new_for_testing();
    let quiet = Duration::from_secs(10);
    let ttl = Duration::from_secs(600);
    let now = std::time::Instant::now();
    assert_eq!(camera_health::frameless_notice(&state, CameraRole::Main, ARES.name, quiet), RecoveryStatus::None);

    state.roster.bump_frameless_opens(CameraRole::Main, ARES.name, ttl, now);
    assert_eq!(
        camera_health::frameless_notice(&state, CameraRole::Main, ARES.name, quiet),
        RecoveryStatus::None,
        "a young run stays quiet"
    );

    state.roster.bump_frameless_opens(CameraRole::Guide, NEPTUNE.name, ttl, now - Duration::from_secs(25));
    state.roster.bump_frameless_opens(CameraRole::Guide, NEPTUNE.name, ttl, now);
    let notice = camera_health::frameless_notice(&state, CameraRole::Guide, NEPTUNE.name, quiet);
    assert!(
        matches!(&notice, RecoveryStatus::Unresponsive { opens: 2, for_s, .. } if *for_s >= 25),
        "{notice:?}"
    );
}

/// No capture judges an idle camera's reopen, so its run is forgotten — and so is its
/// notice, which no frame would ever come to clear.
#[tokio::test(flavor = "multi_thread")]
async fn a_reopen_nothing_judges_leaves_no_notice_behind() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let long_ago = std::time::Instant::now() - Duration::from_secs(25);
    state.roster.bump_frameless_opens(CameraRole::Main, NEPTUNE.name, Duration::from_secs(600), long_ago);

    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let recovered = wait_recovered(&state, CameraRole::Main).await;
    let status = state.recovery_status.get(CameraRole::Main);

    teardown(&state).await;
    assert!(recovered);
    assert_eq!(status, RecoveryStatus::None, "an idle reopen left a notice no frame will clear");
}

// --- Review hypotheses, 2026-11-11 ----------------------------------------------------

/// The notice says recovery is reopening the camera. A resumed capture whose first frame
/// fails outright ends without a frame or a reopen, so nothing would ever clear it.
#[tokio::test(flavor = "multi_thread")]
async fn a_capture_ending_in_a_frameless_spell_takes_its_notice_down() {
    use crate::camera::reconnect::NOTICE_AFTER;

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();
    catalog.camera.stalls.store(1, Ordering::SeqCst);
    catalog.set(&[]);
    CaptureService::start_capture(&state, None).await.unwrap();
    tokio::time::sleep(NOTICE_AFTER + Duration::from_millis(100)).await;
    // The resumed capture's first exposure fails: "Failed to capture initial frame".
    catalog.camera.failures.store(1, Ordering::SeqCst);
    catalog.set(&[NEPTUNE]);
    let reopened = eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await;
    let parked = eventually(
        || matches!(state.capture_state(), CaptureState::Idle) && state.slot(CameraRole::Main).holds_handle(),
        Duration::from_secs(10),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = state.recovery_status.get(CameraRole::Main);
    let frames = catalog.camera.frames.load(Ordering::SeqCst);
    let seen = drain(&mut events);

    teardown(&state).await;
    assert!(reopened && parked, "the camera was not reopened, then parked after its capture ended");
    let told = seen.iter().position(|e| matches!(e, ServerEvent::CameraUnresponsive { role: CameraRole::Main, .. }));
    let told = told.unwrap_or_else(|| panic!("the frameless spell was never announced: {seen:?}"));
    assert!(
        seen[told..].iter().any(|e| matches!(e, ServerEvent::CameraRecoveryCleared { role: CameraRole::Main })),
        "the parking never took the notice down"
    );
    assert_eq!(frames, 0, "a frame came, which would have cleared the notice");
    assert_eq!(status, RecoveryStatus::None, "an idle camera still says it is being reopened");
}

/// The stop of a stream that does not answer hands the camera to recovery with its phase
/// still `Guiding`, which `detach` reads as a loop that was running.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_guide_whose_stream_stop_timed_out_stays_stopped() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| catalog.camera.frames.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await);
    catalog.camera.end_stream_delay_ms.store(1_000, Ordering::SeqCst);

    assert!(CaptureService::stop(&state, CameraRole::Guide).await);
    let reopened = eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await;
    catalog.camera.end_stream_delay_ms.store(0, Ordering::SeqCst);
    let recovered = wait_recovered(&state, CameraRole::Guide).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;
    let registered = state.guide_loops.is_registered();

    teardown(&state).await;
    assert!(reopened && recovered);
    assert!(!registered, "the guide loop the observer stopped was restarted by the reopen");
    assert_eq!(frames, 0, "the stopped guide camera streamed after its reopen");
}

/// The monitor now reopens idle cameras on its own, so a Start can land mid-reopen. It
/// must either be refused with a reason or run once the camera is back (it runs).
#[tokio::test(flavor = "multi_thread")]
async fn a_start_while_an_idle_camera_is_reopened_is_not_dropped_silently() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    let mut events = state.subscribe_events();
    catalog.hold(Some(&NEPTUNE));
    lifecycle::finalize_disconnect(&state, CameraRole::Main, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    assert!(eventually(|| catalog.open_calls.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await);

    let started = CaptureService::start_capture(&state, None).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    catalog.hold(None);
    let recovered = wait_recovered(&state, CameraRole::Main).await;
    let capturing = eventually(
        || state.stats.delivered() >= 2 && matches!(state.capture_state(), CaptureState::Capturing),
        Duration::from_secs(3),
    )
    .await;
    let ended_as = state.capture_state();
    let told = drain(&mut events).iter().any(|e| matches!(e, ServerEvent::Error { .. }));

    if capturing {
        CaptureService::stop_capture(&state).await;
        eventually(|| matches!(state.capture_state(), CaptureState::Idle), Duration::from_secs(10)).await;
    }
    teardown(&state).await;
    assert!(recovered);
    assert!(
        started.is_err() || capturing || told,
        "Start answered {started:?}, then the capture ended as {ended_as:?} with no frame and no error"
    );
}

/// A quirk a reopen does not cure: each handle reads plausibly, then -300 °C. The idle
/// reopen ran every ~13 s for as long as the camera sat parked; now once, until a frame.
#[tokio::test(flavor = "multi_thread")]
async fn a_parked_camera_whose_reads_keep_failing_is_reopened_once() {
    use std::sync::atomic::AtomicUsize;

    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Main).await;
    // The status reads counted when each handle's spell began, by spell.
    let spells = Arc::new(Mutex::new(Vec::<usize>::new()));
    let quirk = {
        let catalog = Arc::clone(&catalog);
        let spells = Arc::clone(&spells);
        tokio::spawn(async move {
            let handled = AtomicUsize::new(0);
            loop {
                let opens = catalog.open_calls.load(Ordering::SeqCst);
                if opens != handled.swap(opens, Ordering::SeqCst) {
                    *catalog.camera.temperature_c.lock().unwrap() = None;
                    // At least one monitor poll (2 s) reads the new handle plausibly.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    spells.lock().unwrap().push(catalog.camera.status_reads.load(Ordering::SeqCst));
                    *catalog.camera.temperature_c.lock().unwrap() = Some(-300.0);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    };

    let second_spell = eventually(|| spells.lock().unwrap().len() >= 2, Duration::from_secs(15)).await;
    let began_at = spells.lock().unwrap().last().copied().unwrap_or_default();
    // Twice the polls the first spell took to reopen the camera.
    let outlasted = eventually(
        || catalog.camera.status_reads.load(Ordering::SeqCst) >= began_at + 4,
        Duration::from_secs(15),
    )
    .await;
    let opens = catalog.open_calls.load(Ordering::SeqCst);
    let phase = state.camera_phase(CameraRole::Main);
    quirk.abort();
    *catalog.camera.temperature_c.lock().unwrap() = None;

    teardown(&state).await;
    assert!(second_spell, "the first spell never reopened the camera");
    assert!(outlasted, "the monitor stopped reading the parked camera");
    assert_eq!(opens, 2, "a quirk the first reopen did not cure was reopened again");
    assert_eq!(phase, CameraPhase::Idle, "the camera left alone is still parked and usable");
}

#[tokio::test]
async fn an_idle_reopen_is_allowed_again_once_the_camera_frames() {
    let (state, _dw) = AppState::new_for_testing();
    assert!(camera_health::claim_idle_reopen(&state, CameraRole::Main, ARES.name));
    assert!(!camera_health::claim_idle_reopen(&state, CameraRole::Main, ARES.name), "no frame since");
    assert!(camera_health::claim_idle_reopen(&state, CameraRole::Guide, NEPTUNE.name), "per camera");

    camera_health::frame_arrived(&state, CameraRole::Main, ARES.name);
    assert!(camera_health::claim_idle_reopen(&state, CameraRole::Main, ARES.name), "the frame cured it");
}

// --- The observer's Stop and Start, while the guide camera is reopened ---------------

/// Fault the running or stopped guide camera and hold its reopen inside the vendor open,
/// under the scaled-down `OPEN_TIMEOUT`, so a Stop or Start lands mid-reopen.
async fn mid_guide_reopen(state: &Arc<AppState>, catalog: &FakeCatalog, during: impl std::future::Future<Output = ()>) {
    let opens = catalog.open_calls.load(Ordering::SeqCst);
    catalog.hold(Some(&NEPTUNE));
    lifecycle::finalize_disconnect(state, CameraRole::Guide, NEPTUNE.name, DisconnectCause::DeviceFault).await;
    let opening = eventually(|| catalog.open_calls.load(Ordering::SeqCst) > opens, Duration::from_secs(5)).await;
    during.await;
    catalog.hold(None);
    assert!(opening, "the guide camera was never reopened");
    assert!(wait_recovered(state, CameraRole::Guide).await, "the guide camera did not come back");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_pressed_while_the_guide_is_reopened_keeps_it_stopped() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    assert!(eventually(|| catalog.camera.frames.load(Ordering::SeqCst) >= 2, Duration::from_secs(5)).await);

    let mut stopped = false;
    mid_guide_reopen(&state, &catalog, async { stopped = CaptureService::stop(&state, CameraRole::Guide).await }).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;
    let registered = state.guide_loops.is_registered();

    teardown(&state).await;
    assert!(stopped, "a Stop during the reopen was answered as nothing to stop");
    assert!(!registered, "the reopen restarted a guide loop stopped during it");
    assert_eq!(frames, 0, "the guide camera streamed after a Stop pressed during its reopen");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_pressed_while_a_stopped_guide_is_reopened_runs_it() {
    let catalog = FakeCatalog::with(&[NEPTUNE]);
    let state = rig(&catalog);
    connect(&state, &NEPTUNE, CameraRole::Guide).await;
    stop_guide(&state).await;

    mid_guide_reopen(&state, &catalog, async {
        CaptureService::start(&state, None, CameraRole::Guide).await.expect("Start during the reopen");
    })
    .await;
    let running = eventually(|| state.guide_loops.is_running(), Duration::from_secs(5)).await;
    let frames = guide_frames_over(&catalog, Duration::from_millis(300)).await;

    teardown(&state).await;
    assert!(running, "a Start during the reopen did not start the guide loop");
    assert!(frames > 0, "the started guide loop takes no frames");
}
