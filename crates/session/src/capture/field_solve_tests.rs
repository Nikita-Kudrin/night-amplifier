use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use night_amplifier_core::detection::StarDetector;
use night_amplifier_core::plugins::Plugins;
use night_amplifier_core::push_to::{FieldObjectsPlugin, PushToResult, TelescopeSettings};

use crate::services::PushToState;
use crate::state::{CameraRole, ConnectedCameraInfo};

fn region() -> ProtectedRegion {
    ProtectedRegion { cx: 100.0, cy: 80.0, semi_major: 30.0, semi_minor: 10.0, angle: 0.5 }
}

fn too_few_stars() -> PushToError {
    PushToError::NotEnoughStars { found: 3, required: 10 }
}

#[test]
fn nothing_is_claimed_before_the_stack_is_deep_enough() {
    let solve = FieldSolve::default();
    assert!(solve.claim(FIRST_DEPTH - 1).is_none());
    assert!(solve.claim(FIRST_DEPTH).is_some());
    assert!(solve.claim(FIRST_DEPTH + 5).is_none(), "one attempt at a time");
}

#[test]
fn a_solve_protects_its_stack_until_the_next_one() {
    let solve = FieldSolve::default();
    let ticket = solve.claim(FIRST_DEPTH).unwrap();
    solve.finish(ticket, Ok(vec![region()]), (3008, 3008), FIRST_DEPTH);
    let protection = solve.protection().expect("a landed solve protects");
    assert_eq!((protection.width, protection.regions.len()), (3008, 1));
    assert!(solve.claim(100).is_none(), "one solve per stack");

    solve.reset();
    assert!(solve.protection().is_none(), "a new stack must not inherit the old field's footprints");
    let next = solve.claim(FIRST_DEPTH).expect("a new stack solves again");
    solve.finish(next, Ok(vec![region()]), (3008, 3008), FIRST_DEPTH);
    assert_ne!(solve.protection().unwrap().generation, protection.generation, "the cache must see a new solve");
}

#[test]
fn a_result_for_a_replaced_stack_is_dropped() {
    let solve = FieldSolve::default();
    let stale = solve.claim(FIRST_DEPTH).unwrap();
    solve.reset();
    let current = solve.claim(FIRST_DEPTH).unwrap();
    solve.finish(stale, Ok(vec![region()]), (3008, 3008), FIRST_DEPTH);
    assert!(solve.protection().is_none(), "the old stack's solve landed on the new one");
    solve.finish(current, Ok(vec![]), (3008, 3008), FIRST_DEPTH);
    assert!(solve.protection().is_some());
}

/// A thin stack of a zoomed-in field has too few stars: try deeper, then give up.
#[test]
fn too_few_stars_retries_deeper_then_gives_up() {
    let solve = FieldSolve::default();
    let mut depth = FIRST_DEPTH;
    for attempt in 1..=MAX_ATTEMPTS {
        let ticket = solve.claim(depth).unwrap_or_else(|| panic!("attempt {attempt} not claimed at {depth}"));
        solve.finish(ticket, Err(too_few_stars()), (3008, 3008), depth);
        assert!(solve.claim(depth + RETRY_STEP - 1).is_none(), "retried before going deeper");
        depth += RETRY_STEP;
    }
    assert!(solve.claim(depth * 10).is_none(), "gave up after {MAX_ATTEMPTS} attempts");
}

#[test]
fn a_failed_solve_gives_up_for_the_stack() {
    let solve = FieldSolve::default();
    let ticket = solve.claim(FIRST_DEPTH).unwrap();
    solve.finish(ticket, Err(PushToError::SolveFailed("timed out".into())), (3008, 3008), FIRST_DEPTH);
    assert!(solve.claim(1000).is_none());
    assert!(solve.protection().is_none());
}

#[test]
fn a_released_claim_is_retried_on_the_next_frame() {
    let solve = FieldSolve::default();
    let ticket = solve.claim(FIRST_DEPTH).unwrap();
    solve.release(ticket, FIRST_DEPTH + 1);
    assert!(solve.claim(FIRST_DEPTH).is_none());
    assert!(solve.claim(FIRST_DEPTH + 1).is_some());
}

// ---------------------------------------------------------------------------
// `offer`, through AppState
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeField {
    calls: AtomicUsize,
    cancels: AtomicUsize,
    /// What the last call was handed: the camera, the focal length, the frame's channels.
    seen: Mutex<Option<(Option<String>, Option<f32>, usize)>>,
}

#[async_trait]
impl FieldObjectsPlugin for FakeField {
    async fn objects_in_field(
        &self,
        frame: &Frame,
        camera: Option<&str>,
        telescope: &TelescopeSettings,
        _: &StarDetector,
        budget: Duration,
    ) -> PushToResult<Vec<ProtectedRegion>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(budget, BUDGET);
        *self.seen.lock().unwrap() = Some((camera.map(str::to_owned), telescope.focal_length_mm, frame.channels()));
        Ok(vec![ProtectedRegion { cx: frame.width() as f32 / 2.0, ..region() }])
    }

    fn cancel(&self) -> bool {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        true
    }
}

fn state_with(plugin: Option<Arc<FakeField>>) -> Arc<AppState> {
    let (mut state, _writer) = AppState::new_for_testing();
    state.push_to = Some(PushToState::default());
    state.plugins = match plugin {
        Some(p) => Plugins::none().with_field_objects(p).always_licensed(),
        None => Plugins::none().always_licensed(),
    };
    Arc::new(state)
}

fn stack() -> Arc<Frame> {
    Arc::new(Frame::filled(64, 48, 3, 0.1).unwrap())
}

fn wait_for_protection(state: &AppState) -> Option<Arc<FieldProtection>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(p) = state.field_solve.protection() {
            return Some(p);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

#[test]
fn an_offered_stack_is_solved_once_off_the_calling_thread() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let plugin = Arc::new(FakeField::default());
    let state = state_with(Some(Arc::clone(&plugin)));

    offer(&state, rt.handle(), &stack(), FIRST_DEPTH - 1);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(plugin.calls.load(Ordering::SeqCst), 0, "solved before the stack was deep enough");

    offer(&state, rt.handle(), &stack(), FIRST_DEPTH);
    let protection = wait_for_protection(&state).expect("the solve never landed");
    assert_eq!((protection.width, protection.height), (64, 48));
    assert_eq!(protection.regions[0].cx, 32.0);

    offer(&state, rt.handle(), &stack(), FIRST_DEPTH + 10);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(plugin.calls.load(Ordering::SeqCst), 1, "one solve per stack");
}

/// A Push-To solve in flight defers the field solve: two ASTAPs at once halve both.
#[test]
fn a_push_to_solve_in_flight_defers_the_field_solve() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let plugin = Arc::new(FakeField::default());
    let state = state_with(Some(Arc::clone(&plugin)));

    let push_to = state.push_to.as_ref().unwrap().try_begin_solve(Instant::now(), Duration::ZERO).unwrap();
    offer(&state, rt.handle(), &stack(), FIRST_DEPTH);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(plugin.calls.load(Ordering::SeqCst), 0, "ran beside a Push-To solve");

    drop(push_to);
    offer(&state, rt.handle(), &stack(), FIRST_DEPTH + 1);
    assert!(wait_for_protection(&state).is_some(), "the deferred solve never ran");
}

#[test]
fn community_never_claims_a_solve() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let state = state_with(None);
    offer(&state, rt.handle(), &stack(), FIRST_DEPTH);
    assert!(state.field_solve.claim(FIRST_DEPTH).is_some(), "the job must still be waiting");
}

/// A Stop cancels the solve in flight (`abandon_solve_on_shutdown`), as does Push-To's
/// movement watch; neither says anything about the stack. A resumed capture keeps the
/// stack (and this job), so a cancel must leave it solvable, not given up.
#[test]
fn a_cancelled_attempt_is_retried_not_given_up() {
    let solve = FieldSolve::default();
    let ticket = solve.claim(FIRST_DEPTH).unwrap();
    solve.finish(ticket, Err(PushToError::Cancelled), (3008, 3008), FIRST_DEPTH);
    assert!(solve.claim(FIRST_DEPTH + 1).is_some(), "a cancelled solve gave the stack up");
}

/// The stack is the imaging camera's, through its own optics — not the solver's rig,
/// which is the guide's while one holds Push-To. One plane reaches the plugin.
#[test]
fn the_stack_is_solved_on_the_imaging_cameras_rig_as_one_plane() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let plugin = Arc::new(FakeField::default());
    let state = state_with(Some(Arc::clone(&plugin)));
    state.roster.install(
        ConnectedCameraInfo {
            id: "mock_0".into(),
            provider: "Mock".into(),
            index: 0,
            role: CameraRole::Main,
            info: night_amplifier_core::camera::CameraInfo { name: "Ares-C PRO".into(), ..Default::default() },
        },
        false,
    );
    state.settings.update(|s| {
        let imaging = TelescopeSettings { focal_length_mm: Some(750.0), ..Default::default() };
        s.camera_telescope_profiles.insert("Ares-C PRO".into(), imaging);
        s.telescope = TelescopeSettings { focal_length_mm: Some(200.0), ..Default::default() };
    });

    offer(&state, rt.handle(), &stack(), FIRST_DEPTH);
    wait_for_protection(&state).expect("the solve never landed");
    let seen = plugin.seen.lock().unwrap().clone();
    assert_eq!(seen, Some((Some("Ares-C PRO".into()), Some(750.0), 1)));
}

#[test]
fn a_stop_cancels_the_stack_solve() {
    let plugin = Arc::new(FakeField::default());
    let state = state_with(Some(Arc::clone(&plugin)));
    cancel(&state);
    assert_eq!(plugin.cancels.load(Ordering::SeqCst), 1);
}
