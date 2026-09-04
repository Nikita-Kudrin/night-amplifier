//! Server-side Push-To bookkeeping: the solve latch, and the de-duplication that
//! keeps per-frame recomputation off the event bus.

use crate::server::api::settings::{optics_change, OpticsChange};
use crate::server::dto::UpdateSettingsRequest;
use crate::server::services::PushToState;
use crate::server::state::{CameraRole, CaptureSettings, TelescopeSettings};
use std::time::{Duration, Instant};

/// No cadence floor, so the latch tests exercise the latch alone.
const NO_THROTTLE: Duration = Duration::ZERO;

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

// ── Direction de-duplication ─────────────────────────────────────────────────

#[test]
fn an_unchanged_direction_is_only_announced_once() {
    // The direction is recomputed on every captured frame but only changes when the
    // position or target does. Broadcasting it regardless was one WebSocket message
    // per frame per client, all saying the same thing.
    let mut state = PushToState::default();

    assert!(state.direction_is_news(122.68, 4.512, false));
    assert!(!state.direction_is_news(122.68, 4.512, false));
    assert!(!state.direction_is_news(122.68, 4.512, false));
}

#[test]
fn floating_point_jitter_does_not_count_as_a_change() {
    // Spherical geometry recomputed from scratch each frame is not bit-stable, so an
    // exact comparison would let the per-frame spam straight back through.
    let mut state = PushToState::default();

    assert!(state.direction_is_news(122.680_000, 4.512_000, false));
    assert!(!state.direction_is_news(122.680_004, 4.512_000_2, false));
}

#[test]
fn a_direction_the_user_could_act_on_is_announced() {
    let mut state = PushToState::default();
    assert!(state.direction_is_news(122.6, 4.5, false));

    assert!(
        state.direction_is_news(124.0, 4.5, false),
        "a 1.4 degree heading change is real"
    );
    assert!(
        state.direction_is_news(124.0, 3.9, false),
        "closing half a degree of distance is real"
    );
    assert!(
        state.direction_is_news(124.0, 3.9, true),
        "crossing the 'close enough' threshold is real"
    );
}

#[test]
fn a_new_target_re_announces_even_an_identical_direction() {
    // Two targets can happen to lie in the same direction at the same distance. If
    // the record were not cleared, the client would keep showing numbers computed for
    // the target the user just moved away from.
    let mut state = PushToState::default();
    assert!(state.direction_is_news(90.0, 2.0, false));

    state.forget_direction();

    assert!(
        state.direction_is_news(90.0, 2.0, false),
        "a target change must force the direction to be sent again"
    );
}

// ── Blocker de-duplication ───────────────────────────────────────────────────

#[test]
fn a_blocker_is_announced_on_each_transition_and_not_between() {
    use crate::push_to::PushToBlocker;

    let mut state = PushToState::default();

    assert!(
        state.blocker_is_news(Some(PushToBlocker::SolverNotReady)),
        "the first report is always news"
    );
    assert!(!state.blocker_is_news(Some(PushToBlocker::SolverNotReady)));

    assert!(
        state.blocker_is_news(Some(PushToBlocker::NoTarget)),
        "a different reason is news"
    );
    assert!(
        state.blocker_is_news(None),
        "clearing the notice is news, so the UI can take it down"
    );
    assert!(!state.blocker_is_news(None));
}

#[test]
fn the_absence_of_a_blocker_is_reported_the_first_time() {
    // The outer Option exists to tell "never reported" from "reported that nothing is
    // wrong"; without it the very first healthy poll would be swallowed.
    let mut state = PushToState::default();
    assert!(state.blocker_is_news(None));
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

    assert_eq!(optics_change(&request, &current, CameraRole::Main), OpticsChange::default());
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

    let change = optics_change(&request, &current, CameraRole::Main);
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
        optics_change(&UpdateSettingsRequest::default(), &current, CameraRole::Main),
        OpticsChange::default()
    );
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
    assert!(!optics_change(&request, &current, CameraRole::Main).any());
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

    let change = optics_change(&request, &current, CameraRole::Main);
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

    assert_eq!(optics_change(&request, &current, CameraRole::Main), OpticsChange::default());
}
