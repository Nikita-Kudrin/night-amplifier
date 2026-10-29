//! Stop behind a slow stacker, end to end: 2026-10-24's planetary session, scaled down.
//!
//! That night each frame took 134 s to stack and 19 were queued: Stop waited for them, the
//! camera stayed held, and when the power was cut no stack was saved and the SER's header
//! still counted 0 frames. Here a planetary plugin takes 400 ms a frame behind a queue of
//! 10 ms exposures; draining it whole would take over a minute. A Disconnect pressed then
//! must still warm the cooled camera up and close it, even when its request is dropped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::lifecycle::{self, DisconnectOutcome, WarmupPolicy};
use super::recovery_tests::{connect, id_of, saving_rig_with, teardown, FakeCatalog, FakeDevice, ARES, NEPTUNE};
use super::tests::eventually;
use crate::capture::STOP_DRAIN;
use crate::services::CaptureService;
use crate::state::{AppState, CameraPhase, CameraRole, CaptureState, RawFrameSaving};
use night_amplifier_core::frame::Frame;
use night_amplifier_core::planetary::{AlignmentRoi, LuminancePyramid, PlanetaryStackerPlugin, Search};
use night_amplifier_core::plugins::Plugins;
use night_amplifier_core::stacking::StackingType;

/// Multi-point alignment taking `delay` a frame: the stacker's whole cost, made slow.
struct Slow {
    delay: Duration,
    calls: AtomicUsize,
}

impl PlanetaryStackerPlugin for Slow {
    fn warp_frame(
        &self,
        frame: &Frame,
        _: &LuminancePyramid,
        _: &LuminancePyramid,
        _: &AlignmentRoi,
        _: Search,
    ) -> night_amplifier_core::error::Result<Frame> {
        std::thread::sleep(self.delay);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(frame.clone())
    }

    fn clear_cache(&self) {}
}

/// `(frames the header counts, whole frames in the file)`; Mono 32x24 at 8 bits.
fn ser_frames(path: &std::path::Path) -> Option<(u32, u64)> {
    let bytes = std::fs::read(path).ok()?;
    let header = u32::from_le_bytes(bytes.get(38..42)?.try_into().ok()?);
    Some((header, (bytes.len() as u64 - 178) / (32 * 24)))
}

/// A planetary capture on `device` that has fallen behind `slow`'s stacker: started, its
/// stacker busy and frames queued. A `cooled` camera has its cooler on.
async fn behind_a_slow_stacker(device: &FakeDevice, cooled: bool) -> (Arc<AppState>, Arc<Slow>, Arc<FakeCatalog>) {
    let slow = Arc::new(Slow { delay: Duration::from_millis(400), calls: AtomicUsize::new(0) });
    let catalog = FakeCatalog::with(std::slice::from_ref(device));
    catalog.cooled.store(cooled, Ordering::SeqCst);
    let state = saving_rig_with(&catalog, Plugins::none().with_planetary(slow.clone()).always_licensed());
    state.settings.update(|s| {
        s.stacking = true;
        s.stacking_type = StackingType::Planetary;
        s.planetary_multi_point_alignment = true;
        s.exposure_us = 10_000;
        s.raw_frame_saving = RawFrameSaving { stacking: true, ..RawFrameSaving::default() };
        s.save_stacked_image = true;
        if cooled {
            s.cooler_enabled = true;
            s.target_temp_c = Some(-10.0);
        }
    });
    connect(&state, device, CameraRole::Main).await;

    CaptureService::start_capture(&state, None).await.unwrap();
    let busy = eventually(|| slow.calls.load(Ordering::SeqCst) >= 2 && state.stats.delivered() >= 40, Duration::from_secs(10)).await;
    assert!(busy, "the stacker never fell behind");
    (state, slow, catalog)
}

/// Whether the session's stack was saved, its raw folder removed either way.
fn saved_stack_then_clean(session: &std::path::Path) -> bool {
    let name = session.file_name().unwrap().to_string_lossy().to_string();
    let stacked_dir = session.parent().and_then(|raw| raw.parent()).unwrap().join("stacked");
    let saved = std::fs::read_dir(&stacked_dir)
        .map(|dir| dir.flatten().any(|e| e.file_name().to_string_lossy() == format!("{name}.fits")))
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(session);
    saved
}

/// Where the imaging camera is once its capture has wound down: still warming up or gone
/// is the Disconnect carried out; `Idle` or `Precooling` is a Disconnect that only stopped.
async fn after_the_capture(state: &Arc<AppState>) -> (bool, CameraPhase, bool) {
    let idle = eventually(|| state.capture_state() == CaptureState::Idle, STOP_DRAIN + Duration::from_secs(10)).await;
    // The hand-back happens just before `Idle`; the warm-up it starts is the monitor's.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let warming = state.slot(CameraRole::Main).warmup().is_some();
    (idle, state.camera_phase(CameraRole::Main), warming)
}

async fn disconnect_now(state: &Arc<AppState>, device: &FakeDevice) {
    let _ = lifecycle::disconnect(state, &id_of(device), WarmupPolicy::Skip).await;
    teardown(state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_behind_a_slow_stacker_saves_within_the_drain_with_its_video_whole() {
    let (state, slow, _catalog) = behind_a_slow_stacker(&NEPTUNE, false).await;
    let session = state.disk_writer.session_dir().expect("a planetary session");
    let video = session.join("capture.ser");

    let stopped_at = Instant::now();
    CaptureService::stop_capture(&state).await;
    // The storage task ends at once; the stacker is still on its queue.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let while_stopping = (state.capture_state() == CaptureState::Stopping).then(|| ser_frames(&video)).flatten();
    let idle = eventually(|| state.capture_state() == CaptureState::Idle, STOP_DRAIN + Duration::from_secs(10)).await;
    let stopping_for = stopped_at.elapsed();
    teardown(&state).await;

    let ended = ser_frames(&video).expect("the video");
    let frames = ended.0;
    let stacked = slow.calls.load(Ordering::SeqCst);
    let saved_stack = saved_stack_then_clean(&session);

    assert!(u64::from(ended.0) == ended.1 && ended.0 > 0, "the finished video: {ended:?}");
    assert_eq!(while_stopping, Some(ended), "the SER was not whole while the stacker drained");
    assert!(idle, "still stopping {stopping_for:?} after Stop");
    assert!(stopping_for < STOP_DRAIN + Duration::from_secs(3), "Stop took {stopping_for:?}");
    assert!(saved_stack, "no stack saved");
    assert!((stacked as u32) + 10 < frames, "{stacked} of {frames} frames stacked: the queue drained whole");
}

/// Disconnect pressed during the capture answers after the short wait, not when the stack
/// is saved, and the warm-up starts once the capture hands the camera back.
#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_behind_a_slow_stacker_answers_warming_up_and_warms_up_after_the_save() {
    let (state, _slow, _catalog) = behind_a_slow_stacker(&ARES, true).await;
    let session = state.disk_writer.session_dir().expect("a planetary session");

    let pressed = Instant::now();
    let outcome = lifecycle::disconnect(&state, &id_of(&ARES), WarmupPolicy::WhenPossible).await;
    let answered_in = pressed.elapsed();
    let (idle, phase, warming) = after_the_capture(&state).await;
    disconnect_now(&state, &ARES).await;
    let saved_stack = saved_stack_then_clean(&session);

    assert!(matches!(outcome, Ok(DisconnectOutcome::WarmingUp { .. })), "{outcome:?}");
    assert!(answered_in < lifecycle::CAPTURE_STOP_WAIT + Duration::from_secs(1), "answered after {answered_in:?}");
    assert!(idle, "the capture never wound down");
    assert!(saved_stack, "no stack saved");
    assert!(
        matches!(phase, CameraPhase::WarmingUp | CameraPhase::Disconnected) && (warming || phase == CameraPhase::Disconnected),
        "after the capture the camera is {phase:?}, warm-up tracked: {warming}"
    );
}

/// 2026-10-24: the page that pressed Disconnect reloaded while the capture wound down, the
/// request was dropped with it, and the Disconnect had done no more than a Stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_whose_request_is_dropped_still_warms_up_and_disconnects() {
    let (state, _slow, _catalog) = behind_a_slow_stacker(&ARES, true).await;
    let session = state.disk_writer.session_dir().expect("a planetary session");

    let dropped = tokio::time::timeout(
        Duration::from_secs(1),
        lifecycle::disconnect(&state, &id_of(&ARES), WarmupPolicy::WhenPossible),
    )
    .await;
    let stopping = state.capture_state() == CaptureState::Stopping;
    let (idle, phase, warming) = after_the_capture(&state).await;
    disconnect_now(&state, &ARES).await;
    let saved_stack = saved_stack_then_clean(&session);

    assert!(dropped.is_err(), "the Disconnect answered before the request was dropped: {dropped:?}");
    assert!(stopping, "the Disconnect did not stop the capture");
    assert!(idle, "the capture never wound down");
    assert!(saved_stack, "no stack saved");
    assert!(
        matches!(phase, CameraPhase::WarmingUp | CameraPhase::Disconnected) && (warming || phase == CameraPhase::Disconnected),
        "the dropped Disconnect left the camera {phase:?}, warm-up tracked: {warming}"
    );
}
