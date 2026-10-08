//! Whether a plate-solve dispatch survives a rig switch after `plate_solve_available` approved the frame.
//!
//! Review of `656e367`/`1c8fa0e` (2026-09-05): the shared `MovementDetector` doesn't
//! know which camera's frame it's scoring, so a frame in flight during a guide
//! connect/disconnect read as the new rig having moved, aborting a fresh solve.
//! `solve_frame`/`watch_frame` now re-check `SolveSource::is_active` before dispatch,
//! not just once at the gate. The plugin is handed to each test state
//! (`AppState::plugins`), so nothing process-wide changes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use night_amplifier::detection::StarDetector;
use night_amplifier::frame::Frame;
use night_amplifier::plugins::Plugins;
use night_amplifier::push_to::{
    CatalogEntryResponse, FrameOutcome, PushToDirectionResponse, PushToResult, PushToSolverPlugin,
    PushToStatusResponse, TelescopeSettings,
};
use night_amplifier::session::capture::solving::{solve_frame, watch_frame, SolveSource};
use night_amplifier::session::services::PushToState;
use night_amplifier::session::state::AppState;

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
        messier: None,
        matched_name: None,
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

    async fn set_fov(&self, _fov: f32) -> PushToResult<()> {
        Ok(())
    }

    async fn set_telescope_settings(&self, _settings: TelescopeSettings) -> PushToResult<()> {
        Ok(())
    }

    async fn set_active_camera(&self, _camera: Option<String>) {}
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

/// A fresh app state with a `PushToState` claiming the solve slot, so `watch_frame`
/// finds a solve to watch.
async fn state_mid_solve(plugins: &Plugins) -> Arc<AppState> {
    // No raw frames are queued in this test, so the writer half can simply drop —
    // nothing needs it running.
    let (mut state, _disk_writer) = AppState::new();
    state.plugins = plugins.clone();

    let push_to = PushToState::default();
    let latch = push_to
        .try_begin_solve(Instant::now(), Duration::ZERO)
        .expect("a fresh state has nothing to contend with");
    // Leaked rather than bound to a variable this function keeps: `SolveLatch::drop`
    // releases the slot, and this state must read as "solving" for as long as the
    // test holds onto it, which outlives this function's own scope.
    std::mem::forget(latch);
    state.push_to = Some(push_to);
    Arc::new(state)
}

/// A fresh app state with nothing running yet, so `solve_frame` starts a solve.
async fn state_ready_to_solve(plugins: &Plugins) -> Arc<AppState> {
    let (mut state, _disk_writer) = AppState::new();
    state.plugins = plugins.clone();
    state.push_to = Some(PushToState::default());
    Arc::new(state)
}

/// Poll for up to a second for a dispatch count to be reached.
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

    let observe_calls = Arc::new(AtomicUsize::new(0));
    let process_calls = Arc::new(AtomicUsize::new(0));
    let on_get_status: Arc<Mutex<Option<Box<dyn Fn() + Send>>>> = Arc::new(Mutex::new(None));

    let plugins = Plugins::none()
        .with_push_to_solver(Arc::new(CountingPlugin {
            observe_frame_calls: Arc::clone(&observe_calls),
            process_new_frame_calls: Arc::clone(&process_calls),
            on_get_status: Arc::clone(&on_get_status),
        }))
        .always_licensed();

    let frame = tiny_frame();

    // ---- watch arm: the source is still active — dispatch reaches the plugin ------
    let state = state_mid_solve(&plugins).await;
    state.set_guide_loop_running(false); // Main is the active source

    watch_frame(&state, Arc::clone(&frame), SolveSource::Main).await;
    assert_eq!(
        observe_calls.load(Ordering::SeqCst),
        1,
        "the active source's frame must still reach the watch"
    );

    // ---- watch arm: the source went stale before watch_frame even started ---------
    let state = state_mid_solve(&plugins).await;
    // The guide camera connects in the gap between the caller's `plate_solve_available`
    // check and this call — exactly the race the fix closes.
    state.set_guide_loop_running(true);

    watch_frame(&state, Arc::clone(&frame), SolveSource::Main).await;
    assert_eq!(
        observe_calls.load(Ordering::SeqCst),
        1,
        "a frame from the outgoing camera must not reach the watch once the rig has \
         changed underneath it"
    );

    // ---- solve arm: the source is active throughout — dispatch reaches the plugin -
    let state = state_ready_to_solve(&plugins).await;
    state.set_guide_loop_running(false);

    solve_frame(&state, Arc::clone(&frame), SolveSource::Main).await;
    wait_for_count(&process_calls, 1).await;

    // ---- solve arm: the rig changes while `get_status` is in flight ---------------
    // The gap unique to this arm: claiming the slot and dispatching into
    // `process_new_frame` cross `get_status().await`, which can outlast a rig switch that
    // lands in between.
    let state = state_ready_to_solve(&plugins).await;
    state.set_guide_loop_running(false);
    let flip_state = Arc::clone(&state);
    *on_get_status.lock().unwrap() = Some(Box::new(move || {
        flip_state.set_guide_loop_running(true);
    }));

    solve_frame(&state, Arc::clone(&frame), SolveSource::Main).await;
    assert_eq!(
        process_calls.load(Ordering::SeqCst),
        1,
        "a rig change discovered while `get_status` was in flight must still drop the \
         frame instead of dispatching it"
    );
}
