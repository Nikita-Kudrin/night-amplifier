//! Frames offered to Push-To must never queue, and a long solve must not starve the watch.
//!
//! Review of a2f89e5 (2026-09-16): offers were spawned as tasks on a two-worker runtime,
//! and the cadence floors are stamped only when an offer runs `try_begin_*`. With a solve
//! and a watch both inside synchronous detection, every offer past a floor queued with a
//! full frame — 2026-09-14's guide frames are ~50 MB — 27 of 32 alive at once, the oldest
//! reaching the plugin 5.7 s stale. `push_to_tasks` hands a frame only to an idle consumer.
//!
//! One test in its own binary: it registers into the process-global `PUSH_TO_PLUGIN`, as
//! `push_to_runtime_isolation_test` does.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use night_amplifier::detection::StarDetector;
use night_amplifier::frame::Frame;
use night_amplifier::push_to::{
    FrameOutcome, PushToCatalogPlugin, PushToInstallerPlugin, PushToResult, PushToSolverPlugin,
    PUSH_TO_PLUGIN,
};
use night_amplifier::server::capture::solving::{
    offer_plate_solve, plate_solve_available, SolveSource,
};
use night_amplifier::server::services::PushToState;
use night_amplifier::server::state::AppState;
use night_amplifier::server::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, CoordinateResponse,
    DatabaseTypeResponse, PushToDirectionResponse, PushToStatusResponse, ServerEvent,
    TelescopeSettings,
};

/// Slow detection on a busy board: longer than both cadence floors.
const BLOCK: Duration = Duration::from_millis(2500);

/// A 0.1 s guide exposure.
const FRAME_INTERVAL: Duration = Duration::from_millis(100);

const RUN_FOR: Duration = Duration::from_secs(8);

/// Offer times, indexed by the value every pixel of that offer's frame carries.
type OfferTimes = Arc<Mutex<Vec<Instant>>>;

struct BlockingPlugin {
    offered_at: OfferTimes,
    worst_age: Arc<Mutex<Duration>>,
    watches: Arc<AtomicUsize>,
}

impl BlockingPlugin {
    fn work(&self, frame: &Frame) -> PushToResult<FrameOutcome> {
        let index = frame.data()[0] as usize;
        let age = self.offered_at.lock().unwrap()[index].elapsed();
        let mut worst = self.worst_age.lock().unwrap();
        *worst = (*worst).max(age);
        drop(worst);
        std::thread::sleep(BLOCK);
        Ok(FrameOutcome::idle())
    }
}

#[async_trait]
impl PushToSolverPlugin for BlockingPlugin {
    async fn process_new_frame(&self, frame: &Frame, _: &StarDetector, _: bool) -> PushToResult<FrameOutcome> {
        self.work(frame)
    }
    async fn observe_frame(&self, frame: &Frame, _: &StarDetector, _: bool) -> PushToResult<FrameOutcome> {
        self.watches.fetch_add(1, Ordering::SeqCst);
        self.work(frame)
    }
    async fn get_status(&self) -> PushToStatusResponse {
        PushToStatusResponse {
            solver_ready: true,
            is_solving: false,
            current_target: Some(CatalogEntryResponse {
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
            }),
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
    async fn set_fov(&self, _: f32) -> Result<(), String> {
        Ok(())
    }
    async fn set_telescope_settings(&self, _: TelescopeSettings) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl PushToCatalogPlugin for BlockingPlugin {
    async fn search_catalog(&self, _: &str, _: usize) -> Vec<CatalogEntryResponse> {
        unreachable!("not exercised by this test")
    }
    async fn get_catalog_by_type(&self, _: &str) -> Vec<CatalogEntryResponse> {
        unreachable!("not exercised by this test")
    }
    async fn set_target_by_name(&self, _: &str) -> Result<CatalogEntryResponse, String> {
        unreachable!("not exercised by this test")
    }
    async fn set_target_by_coords(&self, _: f64, _: f64) -> Result<CoordinateResponse, String> {
        unreachable!("not exercised by this test")
    }
    async fn clear_target(&self) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
    async fn load_database(&self, _: &str) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
}

#[async_trait]
impl PushToInstallerPlugin for BlockingPlugin {
    async fn get_astap_status(&self) -> AstapStatusResponse {
        unreachable!("not exercised by this test")
    }
    async fn get_astap_databases(&self) -> Vec<DatabaseTypeResponse> {
        unreachable!("not exercised by this test")
    }
    async fn install_astap(&self, _: &[String], _: tokio::sync::broadcast::Sender<ServerEvent>) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
    async fn get_catalog_status(&self) -> CatalogStatusResponse {
        unreachable!("not exercised by this test")
    }
    async fn install_catalog(&self, _: bool, _: tokio::sync::broadcast::Sender<ServerEvent>) -> Result<(), String> {
        unreachable!("not exercised by this test")
    }
}

/// `AppState::new` reads `settings.json` and `./captures` from the working directory.
fn isolate_cwd() {
    let dir = std::env::temp_dir().join(format!(
        "night_amplifier_push_to_offer_backlog_test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create an isolated working directory");
    std::env::set_current_dir(&dir).expect("switch into the isolated working directory");
}

/// The guide loop's offer, exactly: check availability, then offer a fresh frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offers_never_queue_behind_busy_push_to_tasks() {
    isolate_cwd();
    night_amplifier::license::PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);

    let offered_at: OfferTimes = Arc::default();
    let worst_age = Arc::new(Mutex::new(Duration::ZERO));
    let watches = Arc::new(AtomicUsize::new(0));
    PUSH_TO_PLUGIN
        .set(Box::new(BlockingPlugin {
            offered_at: Arc::clone(&offered_at),
            worst_age: Arc::clone(&worst_age),
            watches: Arc::clone(&watches),
        }))
        .ok()
        .expect("this binary registers the plugin exactly once");

    let (state, _disk_writer) = AppState::new();
    let state = Arc::new(state);
    *state.push_to.write().await = Some(PushToState::default());
    state.set_push_to_has_target(true).await;

    let rt = tokio::runtime::Handle::current();
    let mut live: Vec<Weak<Frame>> = Vec::new();
    let mut peak_live = 0;
    let started = Instant::now();
    while started.elapsed() < RUN_FOR {
        if plate_solve_available(&state, SolveSource::Main) {
            let index = {
                let mut offered = offered_at.lock().unwrap();
                offered.push(Instant::now());
                offered.len() - 1
            };
            let frame = Arc::new(Frame::from_f32_vec(vec![index as f32; 16], 4, 4, 1).unwrap());
            live.push(Arc::downgrade(&frame));
            offer_plate_solve(&state, &rt, frame, SolveSource::Main);
        }
        live.retain(|frame| frame.strong_count() > 0);
        peak_live = peak_live.max(live.len());
        tokio::time::sleep(FRAME_INTERVAL).await;
    }

    let worst_age = *worst_age.lock().unwrap();
    eprintln!(
        "offers={} peak live frames={peak_live} worst age at the plugin={worst_age:?}",
        offered_at.lock().unwrap().len()
    );
    // One frame in the solve, one in the watch.
    assert!(peak_live <= 2, "{peak_live} offered frames were alive at once");
    assert!(
        worst_age < Duration::from_secs(1),
        "the plugin was handed a frame {worst_age:?} after it was offered"
    );
    assert!(
        watches.load(Ordering::SeqCst) >= 1,
        "the movement watch never saw a frame while the solve held its task"
    );
}
