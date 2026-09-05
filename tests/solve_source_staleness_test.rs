//! Whether a plate-solve dispatch survives a rig switch that happens after
//! `plate_solve_available` already approved the frame.
//!
//! Found in code review of `656e367`/`1c8fa0e` (2026-09-05): `PushToSolverPlugin::
//! observe_frame`/`process_new_frame` are told nothing about which camera captured
//! their frame, so `ProPushToPlugin::look()` scales and mutates the one shared
//! `MovementDetector` for whichever frame reaches it. A frame from the outgoing camera
//! that is still in flight when a guide camera connects or disconnects — offered
//! before the switch, dispatched after it — used to reach the plugin anyway, reading
//! as the *new* rig's telescope having moved and aborting a solve that had just
//! started. `try_plate_solve` now re-checks `SolveSource::is_active` immediately
//! before every dispatch, not only once at the `plate_solve_available` gate the
//! caller checks first.
//!
//! One test function, deliberately: it registers a fake plugin into the
//! process-global `PUSH_TO_PLUGIN` and flips the process-global `PRO_LICENSE_ACTIVE`
//! — both one-shot/global state that every other test in the crate assumes untouched.
//! A dedicated top-level binary keeps that off the shared `cargo test --lib` process;
//! one test keeps it off this binary's own parallel test scheduling.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use night_amplifier::detection::StarDetector;
use night_amplifier::frame::Frame;
use night_amplifier::push_to::{
    FrameOutcome, PushToCatalogPlugin, PushToInstallerPlugin, PushToResult, PushToSolverPlugin,
    PUSH_TO_PLUGIN,
};
use night_amplifier::server::capture::solving::{try_plate_solve, SolveSource};
use night_amplifier::server::services::PushToState;
use night_amplifier::server::state::AppState;
use night_amplifier::server::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, CoordinateResponse,
    DatabaseTypeResponse, PushToDirectionResponse, PushToStatusResponse, ServerEvent,
    TelescopeSettings,
};

/// Counts dispatches into the plugin. Every gating decision under test lives in
/// `solving.rs`, not here — this only records whether it was reached.
struct CountingPlugin {
    observe_frame_calls: Arc<AtomicUsize>,
    process_new_frame_calls: Arc<AtomicUsize>,
    /// Run synchronously inside `get_status`, so a test can simulate the rig changing
    /// while that exact `.await` is suspended — the gap the solve arm's second
    /// re-check exists for.
    on_get_status: Arc<Mutex<Option<Box<dyn Fn() + Send>>>>,
}

fn fake_target() -> CatalogEntryResponse {
    CatalogEntryResponse {
        designation: "Test Target".to_string(),
        name: None,
        catalog_type: String::new(),
        ra_degrees: 10.0,
        dec_degrees: 20.0,
        ra_string: String::new(),
        dec_string: String::new(),
        object_type: String::new(),
        magnitude: None,
        constellation: String::new(),
    }
}

#[async_trait]
impl PushToSolverPlugin for CountingPlugin {
    async fn process_new_frame(
        &self,
        _frame: &Frame,
        _detector: &StarDetector,
        _wanderer_mode: bool,
    ) -> PushToResult<FrameOutcome> {
        self.process_new_frame_calls.fetch_add(1, Ordering::SeqCst);
        Ok(FrameOutcome::idle())
    }

    async fn observe_frame(
        &self,
        _frame: &Frame,
        _detector: &StarDetector,
        _wanderer_mode: bool,
    ) -> PushToResult<FrameOutcome> {
        self.observe_frame_calls.fetch_add(1, Ordering::SeqCst);
        Ok(FrameOutcome::idle())
    }

    async fn get_status(&self) -> PushToStatusResponse {
        if let Some(hook) = self.on_get_status.lock().unwrap().as_ref() {
            hook();
        }
        PushToStatusResponse {
            solver_ready: true,
            is_solving: false,
            current_target: Some(fake_target()),
            last_position: None,
            direction: None,
        }
    }

    async fn cancel_solve(&self) -> PushToResult<bool> {
        Ok(false)
    }

    async fn restart_solve(&self) -> PushToResult<()> {
        Ok(())
    }

    async fn get_direction(&self) -> Option<PushToDirectionResponse> {
        None
    }

    async fn set_fov(&self, _fov: f32) -> Result<(), String> {
        Ok(())
    }

    async fn set_telescope_settings(&self, _settings: TelescopeSettings) -> Result<(), String> {
        Ok(())
    }

    async fn set_active_camera(&self, _camera: Option<String>) {}
}

/// None of these are exercised by this test — `try_plate_solve` only ever reaches
/// `PushToSolverPlugin` methods — but `PushToSystemPlugin` requires all three traits.
#[async_trait]
impl PushToCatalogPlugin for CountingPlugin {
    async fn search_catalog(&self, _query: &str, _limit: usize) -> Vec<CatalogEntryResponse> {
        unreachable!("not exercised by this test")
    }
    async fn get_catalog_by_type(&self, _catalog_type: &str) -> Vec<CatalogEntryResponse> {
        unreachable!("not exercised by this test")
    }
    async fn set_target_by_name(&self, _name: &str) -> Result<CatalogEntryResponse, String> {
        unreachable!("not exercised by this test")
    }
    async fn set_target_by_coords(
        &self,
        _ra: f64,
        _dec: f64,
    ) -> Result<CoordinateResponse, String> {
        unreachable!("not exercised by this test")
    }
    async fn clear_target(&self) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
    async fn load_database(&self, _path: &str) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
}

#[async_trait]
impl PushToInstallerPlugin for CountingPlugin {
    async fn get_astap_status(&self) -> AstapStatusResponse {
        unreachable!("not exercised by this test")
    }
    async fn get_astap_databases(&self) -> Vec<DatabaseTypeResponse> {
        unreachable!("not exercised by this test")
    }
    async fn install_astap(
        &self,
        _database_types: &[String],
        _events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
    async fn get_catalog_status(&self) -> CatalogStatusResponse {
        unreachable!("not exercised by this test")
    }
    async fn install_catalog(
        &self,
        _include_stars: bool,
        _events: tokio::sync::broadcast::Sender<ServerEvent>,
    ) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
}

fn tiny_frame() -> Arc<Frame> {
    Arc::new(Frame::from_f32_vec(vec![0.1f32; 4 * 4], 4, 4, 1).unwrap())
}

/// `AppState::new_for_testing` is `#[cfg(test)]`-only, so it does not exist in the
/// normal build this external binary links against. Its whole point — never touching
/// a real `settings.json` or `./captures` — is reproduced here by running from a fresh
/// temp directory instead: `SettingsPersistence::default()` and `DiskWriterConfig::default()`
/// both resolve relative paths, and a directory this test just created has neither.
/// Process-wide, but harmless: this binary has exactly one test.
fn isolate_cwd() {
    let dir = std::env::temp_dir().join(format!(
        "night_amplifier_solve_source_staleness_test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create an isolated working directory");
    std::env::set_current_dir(&dir).expect("switch into the isolated working directory");
}

/// A fresh app state with a `PushToState` claiming the solve slot, so `try_plate_solve`
/// routes to the watch arm rather than starting a new solve.
async fn state_mid_solve() -> Arc<AppState> {
    // No raw frames are queued in this test, so the writer half can simply drop —
    // nothing needs it running.
    let (state, _disk_writer) = AppState::new();
    let state = Arc::new(state);

    let push_to = PushToState::default();
    let latch = push_to
        .try_begin_solve(Instant::now(), Duration::ZERO)
        .expect("a fresh state has nothing to contend with");
    // Leaked rather than bound to a variable this function keeps: `SolveLatch::drop`
    // releases the slot, and this state must read as "solving" for as long as the
    // test holds onto it, which outlives this function's own scope.
    std::mem::forget(latch);
    *state.push_to.write().await = Some(push_to);
    state
}

/// A fresh app state with nothing running yet, so `try_plate_solve` starts a solve.
async fn state_ready_to_solve() -> Arc<AppState> {
    let (state, _disk_writer) = AppState::new();
    let state = Arc::new(state);
    *state.push_to.write().await = Some(PushToState::default());
    state
}

/// Poll for up to a second: `process_new_frame` runs on a detached `tokio::spawn`, so
/// a caller that only awaits `try_plate_solve` cannot see it finish.
async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    for _ in 0..100 {
        if counter.load(Ordering::SeqCst) >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "expected at least {expected} call(s), saw {}",
        counter.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn a_rig_switch_between_the_gate_and_the_dispatch_drops_the_stale_frame() {
    isolate_cwd();
    night_amplifier::license::PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);

    let observe_calls = Arc::new(AtomicUsize::new(0));
    let process_calls = Arc::new(AtomicUsize::new(0));
    let on_get_status: Arc<Mutex<Option<Box<dyn Fn() + Send>>>> = Arc::new(Mutex::new(None));

    PUSH_TO_PLUGIN
        .set(Box::new(CountingPlugin {
            observe_frame_calls: Arc::clone(&observe_calls),
            process_new_frame_calls: Arc::clone(&process_calls),
            on_get_status: Arc::clone(&on_get_status),
        }))
        .ok()
        .expect("this binary registers the plugin exactly once, in this one test");

    let frame = tiny_frame();

    // ---- watch arm: the source is still active — dispatch reaches the plugin ------
    let state = state_mid_solve().await;
    state.set_guide_loop_running(false); // Main is the active source

    try_plate_solve(&state, Arc::clone(&frame), SolveSource::Main).await;
    assert_eq!(
        observe_calls.load(Ordering::SeqCst),
        1,
        "the active source's frame must still reach the watch"
    );

    // ---- watch arm: the source went stale before try_plate_solve even started -----
    let state = state_mid_solve().await;
    // The guide camera connects in the gap between the caller's `plate_solve_available`
    // check and this call — exactly the race the fix closes.
    state.set_guide_loop_running(true);

    try_plate_solve(&state, Arc::clone(&frame), SolveSource::Main).await;
    assert_eq!(
        observe_calls.load(Ordering::SeqCst),
        1,
        "a frame from the outgoing camera must not reach the watch once the rig has \
         changed underneath it"
    );

    // ---- solve arm: the source is active throughout — dispatch reaches the plugin -
    let state = state_ready_to_solve().await;
    state.set_guide_loop_running(false);

    try_plate_solve(&state, Arc::clone(&frame), SolveSource::Main).await;
    wait_for_count(&process_calls, 1).await;

    // ---- solve arm: the rig changes while `get_status` is in flight ---------------
    // The gap unique to this arm: claiming the slot and dispatching into
    // `process_new_frame` cross `get_status().await` and a `tokio::spawn`, either of
    // which can outlast a rig switch that lands in between.
    let state = state_ready_to_solve().await;
    state.set_guide_loop_running(false);
    let flip_state = Arc::clone(&state);
    *on_get_status.lock().unwrap() = Some(Box::new(move || {
        flip_state.set_guide_loop_running(true);
    }));

    try_plate_solve(&state, Arc::clone(&frame), SolveSource::Main).await;
    // No count to poll *up* to — this asserts the detached task never runs, so give it
    // a real window to have done so before checking it did not.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        process_calls.load(Ordering::SeqCst),
        1,
        "a rig change discovered while `get_status` was in flight must still drop the \
         frame instead of dispatching it"
    );
}
