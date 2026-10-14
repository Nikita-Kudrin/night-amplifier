//! Server-side Push-To bookkeeping: the solve and watch latches, and which settings
//! changes restart a solve. What clients are told is `PushToAnnouncer`'s, tested there.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::helpers::create_test_state;
use crate::push_to::{DirectionHint, PushToDirectionResponse, TelescopeSettings};
use crate::server::dto::UpdateSettingsRequest;
use crate::session::services::PushToState;
use crate::session::services::{optics_change, OpticsChange};
use crate::session::state::{CameraRole, CaptureSettings, SolverCamera};
use std::time::{Duration, Instant};

/// No cadence floor, so the latch tests exercise the latch alone.
const NO_THROTTLE: Duration = Duration::ZERO;

fn solving(role: CameraRole, name: &str) -> SolverCamera {
    SolverCamera { role, name: name.to_string() }
}

fn telescope(focal_length_mm: f32) -> TelescopeSettings {
    TelescopeSettings {
        focal_length_mm: Some(focal_length_mm),
        pixel_size_x_um: Some(2.9),
        pixel_size_y_um: Some(2.9),
        sensor_width_px: Some(3008),
        sensor_height_px: Some(3008),
        barlow_coeff: Some(1.0),
    }
}

// ── Solve latch ──────────────────────────────────────────────────────────────

// ── Watch latch ──────────────────────────────────────────────────────────────

#[test]
fn the_watch_slot_is_open_while_a_solve_holds_the_solve_slot() {
    // The whole point: a solve can run for minutes, and for all of that time frames
    // still have to reach the movement detector — otherwise a slew cannot arm the
    // gate, cannot abandon a search working on sky we have left, and cannot update
    // the status. The field log has a full-sky rung grinding for 223 s with every
    // frame in between skipped unread.
    let state = PushToState::default();
    let now = Instant::now();

    let solving = state.try_begin_solve(now, NO_THROTTLE).expect("solve claim");
    assert!(state.is_solving());
    assert!(
        state.try_begin_watch(now, NO_THROTTLE).is_some(),
        "the watch must be able to run while a solve does"
    );
    drop(solving);
}

#[test]
fn the_watch_latch_admits_one_frame_at_a_time() {
    let state = PushToState::default();
    let now = Instant::now();

    let first = state.try_begin_watch(now, NO_THROTTLE).expect("first claim");
    assert!(
        state.try_begin_watch(now, NO_THROTTLE).is_none(),
        "two frames must not both run the detector"
    );

    drop(first);
    assert!(state.try_begin_watch(now, NO_THROTTLE).is_some());
}

#[test]
fn the_watch_keeps_its_own_cadence_floor() {
    // Sharing `last_attempt` with the solve path would let the watch run flat out for
    // the minutes a full-sky search takes: that timestamp is stamped once per ladder,
    // so it stops moving the moment a solve begins. Each watch run costs a full
    // sensitive detection over the whole sensor, competing with the ASTAP it may be
    // about to abandon.
    let state = PushToState::default();
    let start = Instant::now();
    let floor = Duration::from_millis(1500);

    // A solve is claimed and stays claimed, exactly as during a long ladder.
    let _solving = state.try_begin_solve(start, floor).expect("solve claim");

    drop(state.try_begin_watch(start, floor).expect("first watch"));
    assert!(
        state.try_begin_watch(start + Duration::from_millis(500), floor).is_none(),
        "the watch must throttle itself, not ride the solve's timestamp"
    );
    assert!(
        state.try_begin_watch(start + floor, floor).is_some(),
        "and open again once its own floor has passed"
    );
}

#[test]
fn an_offer_the_cadence_floor_would_drop_is_declined_before_the_frame_is_cloned() {
    // `plate_solve_available` runs on the stacking thread and its job is to avoid
    // paying for an offer that cannot do anything. Keeping a second handle on the
    // frame alive makes the render task's `Arc::try_unwrap` fail and copy a
    // full-resolution frame, so declining early is worth a duplicated read of the
    // clock — the claim itself is still the compare-and-swap.
    let state = PushToState::default();
    let start = Instant::now();
    let solve_floor = Duration::from_millis(1000);
    let watch_floor = Duration::from_millis(1500);

    assert!(
        state.offer_is_due(start, solve_floor, watch_floor),
        "a session that has never offered a frame must be due"
    );

    drop(state.try_begin_solve(start, solve_floor).expect("first offer"));
    assert!(!state.offer_is_due(start, solve_floor, watch_floor));
    assert!(state.offer_is_due(start + solve_floor, solve_floor, watch_floor));
}

#[test]
fn the_offer_check_follows_the_watch_cadence_while_a_solve_runs() {
    // The two floors differ on purpose — the watch is the slower of the two — so the
    // pre-check has to consult whichever one actually applies.
    let state = PushToState::default();
    let start = Instant::now();
    let solve_floor = Duration::from_millis(1000);
    let watch_floor = Duration::from_millis(1500);

    let _solving = state.try_begin_solve(start, solve_floor).expect("solve claim");
    drop(state.try_begin_watch(start, watch_floor).expect("first watch"));

    assert!(
        !state.offer_is_due(start + solve_floor, solve_floor, watch_floor),
        "the solve floor must not release an offer the watch floor still holds"
    );
    assert!(state.offer_is_due(start + watch_floor, solve_floor, watch_floor));
}

#[test]
fn the_two_latches_do_not_release_each_other() {
    let state = PushToState::default();
    let now = Instant::now();

    let watching = state.try_begin_watch(now, NO_THROTTLE).expect("watch claim");
    let solving = state.try_begin_solve(now, NO_THROTTLE).expect("solve claim");

    drop(watching);
    assert!(
        state.is_solving(),
        "releasing the watch must not clear the solve latch"
    );
    drop(solving);
    assert!(!state.is_solving());
    assert!(state.try_begin_watch(now, NO_THROTTLE).is_some());
}


#[test]
fn the_latch_admits_one_solve_at_a_time() {
    let state = PushToState::default();
    let now = Instant::now();

    let first = state
        .try_begin_solve(now, NO_THROTTLE)
        .expect("the first claim must succeed");
    assert!(state.is_solving());
    assert!(
        state.try_begin_solve(now, NO_THROTTLE).is_none(),
        "a second frame must not start a solve under one already running"
    );

    drop(first);
    assert!(!state.is_solving());
    assert!(
        state.try_begin_solve(now, NO_THROTTLE).is_some(),
        "the slot must be reusable once released"
    );
}

#[test]
fn frames_offered_faster_than_the_cadence_floor_are_dropped() {
    // Every offered frame pays a full sensitive star detection over the whole sensor
    // just to answer "has the view moved", and the answer is almost always no. The
    // floor bounds that cost without changing anything at deep-sky frame rates.
    let state = PushToState::default();
    let start = Instant::now();
    let floor = Duration::from_millis(1000);

    let first = state.try_begin_solve(start, floor).expect("first offer");
    drop(first);

    assert!(
        state
            .try_begin_solve(start + Duration::from_millis(200), floor)
            .is_none(),
        "a frame 200ms later must not be offered"
    );
    assert!(
        state
            .try_begin_solve(start + Duration::from_millis(1100), floor)
            .is_some(),
        "a frame past the floor must be offered"
    );
}

#[test]
fn the_cadence_floor_does_not_consume_the_slot_when_it_declines() {
    // A throttled offer must leave the latch untouched, or the floor would strand it.
    let state = PushToState::default();
    let start = Instant::now();
    let floor = Duration::from_millis(1000);

    drop(state.try_begin_solve(start, floor).expect("first offer"));
    assert!(state
        .try_begin_solve(start + Duration::from_millis(10), floor)
        .is_none());

    assert!(!state.is_solving(), "a declined offer must not raise the latch");
}

#[test]
fn the_latch_is_released_even_if_the_solve_panics() {
    // Regression: the flag was a set/clear pair straddling an `.await`. Any panic in
    // between left it raised for the life of the process, and plate solving was
    // silently dead from that point on with no error anywhere.
    let state = PushToState::default();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _latch = state
            .try_begin_solve(Instant::now(), NO_THROTTLE)
            .expect("claim");
        panic!("the plugin blew up mid-solve");
    }));

    assert!(result.is_err(), "the panic should have propagated");
    assert!(
        !state.is_solving(),
        "a panicking solve must still release the slot"
    );
}

// ── Settings comparison ──────────────────────────────────────────────────────

#[test]
fn a_telescope_block_that_changes_nothing_is_not_a_change() {
    // The bug behind issue 3's fix backfiring: the frontend posts the whole telescope
    // block on every debounced save, and testing presence rather than equality meant
    // each of those aborted the plate solve the user was waiting on.
    let current = CaptureSettings {
        telescope: telescope(1200.0),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        telescope: Some(telescope(1200.0)),
        ..Default::default()
    };

    assert_eq!(optics_change(&request, &current, CameraRole::Main, None), OpticsChange::default());
}

#[test]
fn a_different_focal_length_is_a_change() {
    let current = CaptureSettings {
        telescope: telescope(1200.0),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        telescope: Some(telescope(750.0)),
        ..Default::default()
    };

    let change = optics_change(&request, &current, CameraRole::Main, None);
    assert!(change.telescope);
    assert!(change.any());
}

#[test]
fn a_request_without_a_telescope_block_changes_nothing() {
    let current = CaptureSettings {
        telescope: telescope(1200.0),
        ..Default::default()
    };
    assert_eq!(
        optics_change(&UpdateSettingsRequest::default(), &current, CameraRole::Main, None),
        OpticsChange::default()
    );
}

/// The UI re-posts the selected camera's profile as the flat block on every camera
/// click; the solver reads its own camera's profile, so nothing it uses moved.
#[test]
fn rewriting_only_the_flat_block_is_not_a_change_for_a_profiled_camera() {
    let profiles: std::collections::HashMap<_, _> = [
        ("Imaging".to_string(), telescope(650.0)),
        ("Guiding".to_string(), telescope(200.0)),
    ]
    .into();
    let current = CaptureSettings {
        telescope: telescope(650.0),
        camera_telescope_profiles: profiles.clone(),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        telescope: Some(telescope(200.0)),
        camera_telescope_profiles: Some(profiles),
        last_camera_name: Some("Guiding".into()),
        ..Default::default()
    };

    for solver in [solving(CameraRole::Main, "Imaging"), solving(CameraRole::Guide, "Guiding")] {
        assert_eq!(
            optics_change(&request, &current, CameraRole::Main, Some(&solver)),
            OpticsChange::default(),
            "solving on {}", solver.name
        );
    }
}

/// A camera without a profile of its own falls back to the flat block, so there the
/// flat block is what the solver reads.
#[test]
fn the_flat_block_is_a_change_for_a_camera_without_a_profile() {
    let current = CaptureSettings {
        telescope: telescope(650.0),
        camera_telescope_profiles: [("Imaging".to_string(), telescope(650.0))].into(),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        telescope: Some(telescope(200.0)),
        ..Default::default()
    };

    assert!(optics_change(&request, &current, CameraRole::Main, Some(&solving(CameraRole::Main, "Simulator"))).telescope);
    assert!(!optics_change(&request, &current, CameraRole::Main, Some(&solving(CameraRole::Main, "Imaging"))).telescope);
}

/// Editing the solving camera's own profile is what moves the FOV hint.
#[test]
fn a_new_focal_length_in_the_solving_cameras_profile_is_a_change() {
    let current = CaptureSettings {
        telescope: telescope(650.0),
        camera_telescope_profiles: [("Guiding".to_string(), telescope(200.0))].into(),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        camera_telescope_profiles: Some([("Guiding".to_string(), telescope(240.0))].into()),
        ..Default::default()
    };

    assert!(optics_change(&request, &current, CameraRole::Guide, Some(&solving(CameraRole::Guide, "Guiding"))).telescope);
}

/// Profiles are upserted, so a request naming only another camera's profile leaves the
/// solving camera's in place: a map lacking it no longer drops it to the flat block.
#[test]
fn a_profile_map_without_the_solving_camera_is_not_a_change() {
    let current = CaptureSettings {
        telescope: telescope(650.0),
        camera_telescope_profiles: [
            ("Imaging".to_string(), telescope(650.0)),
            ("Guiding".to_string(), telescope(200.0)),
        ]
        .into(),
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        telescope: Some(telescope(650.0)),
        camera_telescope_profiles: Some([("Imaging".to_string(), telescope(700.0))].into()),
        ..Default::default()
    };
    let guiding = solving(CameraRole::Guide, "Guiding");

    assert!(!optics_change(&request, &current, CameraRole::Main, Some(&guiding)).telescope);
}

#[test]
fn resending_the_current_binning_is_not_a_change() {
    let current = CaptureSettings {
        bin: 2,
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        bin: Some(2),
        ..Default::default()
    };
    assert!(!optics_change(&request, &current, CameraRole::Main, None).any());
}

#[test]
fn a_different_binning_is_a_framing_change() {
    // Binning changes the effective field of view without touching the telescope
    // block, so it invalidates a solve just as surely as a new focal length.
    let current = CaptureSettings {
        bin: 1,
        ..Default::default()
    };
    let request = UpdateSettingsRequest {
        bin: Some(2),
        ..Default::default()
    };

    let change = optics_change(&request, &current, CameraRole::Main, None);
    assert!(!change.telescope);
    assert!(change.framing);
    assert!(change.any());
}

#[test]
fn an_unrelated_setting_is_never_an_optics_change() {
    // Exposure and gain arrive constantly during a session. If they counted, plate
    // solving would restart on every slider move.
    let current = CaptureSettings::default();
    let request = UpdateSettingsRequest {
        exposure_us: Some(5_000_000),
        gain: Some(120),
        ..Default::default()
    };

    assert_eq!(optics_change(&request, &current, CameraRole::Main, None), OpticsChange::default());
}

// ── Without Pro ──────────────────────────────────────────────────────────────

/// A missing plugin answers alike on every Push-To route: 403 and the Pro message. The
/// routes used to pick their own status — 404 for a target by name, 400 for a database,
/// 500 for the rest.
#[tokio::test]
async fn without_the_plugin_every_push_to_route_is_forbidden() {
    let app = crate::server::api::create_router().with_state(create_test_state());
    for (method, uri, body) in [
        ("POST", "/push-to/target", json!({ "name": "M31" })),
        ("POST", "/push-to/target", json!({ "ra_degrees": 10.7, "dec_degrees": 41.3 })),
        ("DELETE", "/push-to/target", json!({})),
        ("POST", "/push-to/cancel", json!({})),
        ("POST", "/push-to/config", json!({ "fov_degrees": 1.2 })),
        ("POST", "/push-to/config", json!({ "database_path": "/nowhere" })),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let answer: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri} {body}");
        assert_eq!(answer["success"], false);
        assert_eq!(
            answer["error"], "Push-To navigation requires Night Amplifier Pro",
            "{method} {uri} {body}"
        );
    }
}

// ── Status ───────────────────────────────────────────────────────────────────

/// A solver that reports one direction and does nothing else.
struct PointingSolver;

#[async_trait::async_trait]
impl crate::push_to::PushToSolverPlugin for PointingSolver {
    async fn process_new_frame(
        &self,
        _frame: &crate::frame::Frame,
        _detector: &crate::detection::StarDetector,
        _wanderer_mode: bool,
    ) -> crate::push_to::PushToResult<crate::push_to::FrameOutcome> {
        unreachable!("no frames in this test")
    }

    async fn observe_frame(
        &self,
        _frame: &crate::frame::Frame,
        _detector: &crate::detection::StarDetector,
        _wanderer_mode: bool,
    ) -> crate::push_to::PushToResult<crate::push_to::FrameOutcome> {
        unreachable!("no frames in this test")
    }

    async fn get_status(&self) -> crate::push_to::PushToStatusResponse {
        crate::push_to::PushToStatusResponse {
            solver_ready: true,
            is_solving: false,
            current_target: None,
            last_position: None,
            direction: Some(pointing()),
        }
    }

    async fn cancel_solve(&self) -> crate::push_to::PushToResult<bool> {
        Ok(false)
    }

    async fn restart_solve(&self) -> crate::push_to::PushToResult<()> {
        Ok(())
    }

    async fn get_direction(&self) -> Option<PushToDirectionResponse> {
        Some(pointing())
    }

    async fn set_fov(&self, _fov: f32) -> crate::push_to::PushToResult<()> {
        Ok(())
    }

    async fn set_telescope_settings(&self, _settings: TelescopeSettings) -> crate::push_to::PushToResult<()> {
        Ok(())
    }

    async fn set_active_camera(&self, _camera: Option<String>) {}
}

fn pointing() -> PushToDirectionResponse {
    PushToDirectionResponse {
        angle_deg: 37.0,
        distance_deg: 2.5,
        is_close: false,
        direction_hint: DirectionHint::NorthEast,
        current_position: None,
        target: None,
        solved_camera: Some("Guide Cam".into()),
    }
}

async fn status(state: std::sync::Arc<crate::session::state::AppState>) -> Value {
    let app = crate::server::api::create_router().with_state(state);
    let request = Request::builder().uri("/push-to/status").body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["data"].clone()
}

/// The status says what the sync snapshot does: the solve, the blocker, and the direction's
/// field of view, which only its event used to carry.
#[tokio::test]
async fn the_status_carries_the_solve_the_blocker_and_the_direction_fov() {
    use crate::push_to::PushToBlocker;
    use crate::session::events::ServerEvent;

    let (mut state, _disk_writer) = crate::session::state::AppState::new_for_testing();
    state.plugins = crate::plugins::Plugins::none()
        .with_push_to_solver(std::sync::Arc::new(PointingSolver))
        .always_licensed();
    let announcer = &state.push_to_announcer;
    announcer.announce_direction(crate::session::events::PushDirection::new(pointing(), Some(1.45)));
    announcer.announce(ServerEvent::PlateSolvingStarted { target_name: Some("M31".into()) });
    announcer.announce(ServerEvent::position_solve_failed("no match"));
    announcer.announce_blocker(Some(PushToBlocker::BackingOff));

    let status = status(std::sync::Arc::new(state)).await;

    assert_eq!(status["solver_ready"], true);
    assert_eq!(status["direction"]["angle_deg"], 37.0);
    assert_eq!(status["direction"]["direction_hint"], "NE");
    assert_eq!(status["direction"]["solved_camera"], "Guide Cam");
    assert_eq!(status["direction"]["fov_deg"], 1.45);
    assert_eq!(
        status["solve"],
        json!({"in_progress": false, "target_name": "M31", "stage": null, "last_result": "failed"})
    );
    assert_eq!(status["blocker"], "Waiting before the next solve attempt");
}

#[tokio::test]
async fn without_the_plugin_the_status_says_nothing_is_happening() {
    let status = status(create_test_state()).await;

    assert_eq!(status["solver_ready"], false);
    assert_eq!(status["direction"], Value::Null);
    assert_eq!(status["solve"]["in_progress"], false);
    assert_eq!(status["blocker"], Value::Null);
}
