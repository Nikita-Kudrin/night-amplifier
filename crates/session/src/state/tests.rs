use super::*;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

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
    assert_eq!(state.capture_state(), CaptureState::Idle);
    assert!(!state.is_cancelled());
}

#[tokio::test]
async fn test_app_state_capture_state() {
    let (state, _disk_writer) = AppState::new_for_testing();

    state.set_capture_state(CaptureState::Capturing);
    assert_eq!(state.capture_state(), CaptureState::Capturing);

    state.set_capture_state(CaptureState::Idle);
    assert_eq!(state.capture_state(), CaptureState::Idle);
}

#[test]
fn test_app_state_frame_tracking() {
    let (state, _disk_writer) = AppState::new_for_testing();
    state.reset_session();

    state.frame_captured(true, true, None);
    state.frame_captured(true, true, None);
    state.frame_captured(false, true, None);

    let counts = state.stats.counts();
    assert_eq!(counts.frames, 3);
    assert_eq!(counts.stacked, 2);
    assert!(state.stats.started_at().is_some());
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

/// The two streams are independent down to the counter. Sharing one would make each
/// camera's frames invalidate the other's payloads on every exposure.
#[tokio::test]
async fn guide_and_main_streams_do_not_share_a_counter_or_payloads() {
    let (state, _disk_writer) = AppState::new_for_testing();

    let main_counter = state.main_stream.begin_frame();
    state.main_stream.set_payload(StreamKind::Jpeg, main_counter, vec![1]);

    // Three guide frames must leave the main stream's payload readable.
    for _ in 0..3 {
        let guide_counter = state.guide_stream.begin_frame();
        state.guide_stream.set_payload(StreamKind::Jpeg, guide_counter, vec![2]);
    }

    assert_eq!(state.main_stream.frame_counter(), 1);
    assert_eq!(state.guide_stream.frame_counter(), 3);
    assert_eq!(
        state.main_stream.payload(StreamKind::Jpeg, main_counter).unwrap().as_ref(),
        &[1]
    );
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

    state.update_camera_status("Test Cam", status.clone(), Some(-10.0));

    let cached = state.get_camera_status("Test Cam").unwrap();
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

/// Every client counts a warm-up down, not only the one whose Disconnect started it and
/// read the time left from the answer.
#[tokio::test]
async fn a_warming_up_phase_event_carries_the_time_left() {
    let (state, _disk_writer) = AppState::new_for_testing();
    let mut subscriber = state.subscribe_events();
    state
        .slot(CameraRole::Main)
        .begin_warmup(Instant::now() + Duration::from_secs(120));

    state.set_camera_phase(CameraRole::Main, "Ares", CameraPhase::WarmingUp);
    state.set_camera_phase(CameraRole::Main, "Ares", CameraPhase::Idle);

    let left = |event| match event {
        ServerEvent::CameraPhaseChanged { warmup_remaining_s, .. } => warmup_remaining_s,
        other => panic!("expected a phase change, got {other:?}"),
    };
    let warming = left(subscriber.try_recv().unwrap()).expect("time left");
    assert!((110..=120).contains(&warming), "{warming}");
    assert_eq!(left(subscriber.try_recv().unwrap()), None, "only a warm-up counts down");
}

/// -300 °C from a just-reopened Ares-C PRO (2026-09-20) is a glitch, not a reading:
/// cached, it is what every ramp seed and the UI's temperature would have used.
#[tokio::test]
async fn an_implausible_temperature_is_neither_cached_nor_broadcast() {
    let (state, _disk_writer) = AppState::new_for_testing();
    let mut subscriber = state.subscribe_events();
    let glitch = CameraStatus {
        temperature_c: -300.0,
        ..Default::default()
    };

    state.update_camera_status("Ares", glitch, Some(0.0));

    assert!(state.get_camera_status("Ares").is_none());
    assert!(subscriber.try_recv().is_err(), "the glitch was broadcast");
}

#[tokio::test]
async fn test_get_camera_status_returns_none_for_unknown() {
    let (state, _disk_writer) = AppState::new_for_testing();
    assert!(state.get_camera_status("Unknown").is_none());
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
