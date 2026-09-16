//! Push-To's work must not run on the server's runtime.
//!
//! 2026-09-14: star detection, the luminance projection and the FITS write ran synchronously
//! inside the plugin's async calls, on the runtime workers every lock and socket of the
//! server waits on — and the guide camera's stalls began during bursts of solves.
//! `solving::offer_plate_solve` hands it to Push-To's own task threads (`push_to_tasks`).
//! The fake plugin holds its thread the way that work does; the test watches whether the
//! server notices.
//!
//! One test in its own binary: it registers into the process-global `PUSH_TO_PLUGIN` and
//! flips `PRO_LICENSE_ACTIVE`, as `solve_source_staleness_test` does.

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
use night_amplifier::server::capture::solving::{offer_plate_solve, SolveSource};
use night_amplifier::server::services::PushToState;
use night_amplifier::server::state::AppState;
use night_amplifier::server::{
    AstapStatusResponse, CatalogEntryResponse, CatalogStatusResponse, CoordinateResponse,
    DatabaseTypeResponse, PushToDirectionResponse, PushToStatusResponse, ServerEvent,
    TelescopeSettings,
};

/// How long the fake plugin holds its thread: a detection plus a FITS write on the Pi.
const BLOCK: Duration = Duration::from_millis(600);

/// The server-side heartbeat's period.
const TICK: Duration = Duration::from_millis(20);

/// Holds the thread it runs on, and records which thread that was.
struct BlockingPlugin {
    offers: Arc<AtomicUsize>,
    threads: Arc<Mutex<Vec<String>>>,
}

impl BlockingPlugin {
    fn work(&self) -> PushToResult<FrameOutcome> {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        self.threads.lock().unwrap().push(thread);
        std::thread::sleep(BLOCK);
        self.offers.fetch_add(1, Ordering::SeqCst);
        Ok(FrameOutcome::idle())
    }
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
impl PushToSolverPlugin for BlockingPlugin {
    async fn process_new_frame(
        &self,
        _frame: &Frame,
        _detector: &StarDetector,
        _wanderer_mode: bool,
    ) -> PushToResult<FrameOutcome> {
        self.work()
    }

    async fn observe_frame(
        &self,
        _frame: &Frame,
        _detector: &StarDetector,
        _wanderer_mode: bool,
    ) -> PushToResult<FrameOutcome> {
        self.work()
    }

    async fn get_status(&self) -> PushToStatusResponse {
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
}

/// Not reached by a solve offer, but `PushToSystemPlugin` requires all three traits.
#[async_trait]
impl PushToCatalogPlugin for BlockingPlugin {
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
impl PushToInstallerPlugin for BlockingPlugin {
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

/// `AppState::new` reads `settings.json` and `./captures` from the working directory, so the
/// test runs from a fresh one — see `solve_source_staleness_test`.
fn isolate_cwd() {
    let dir = std::env::temp_dir().join(format!(
        "night_amplifier_push_to_runtime_isolation_test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create an isolated working directory");
    std::env::set_current_dir(&dir).expect("switch into the isolated working directory");
}

/// One worker on the server's side, so a single blocked task would show as a heartbeat
/// falling behind by the whole block.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn push_to_work_never_holds_the_servers_runtime() {
    isolate_cwd();
    night_amplifier::license::PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);

    let offers = Arc::new(AtomicUsize::new(0));
    let threads = Arc::new(Mutex::new(Vec::new()));
    PUSH_TO_PLUGIN
        .set(Box::new(BlockingPlugin {
            offers: Arc::clone(&offers),
            threads: Arc::clone(&threads),
        }))
        .ok()
        .expect("this binary registers the plugin exactly once");

    let (state, _disk_writer) = AppState::new();
    let state = Arc::new(state);
    *state.push_to.write().await = Some(PushToState::default());

    let heartbeat = tokio::spawn(async {
        let mut worst = Duration::ZERO;
        let mut due = tokio::time::Instant::now();
        for _ in 0..(BLOCK * 2).as_millis() / TICK.as_millis() {
            due += TICK;
            tokio::time::sleep_until(due).await;
            worst = worst.max(tokio::time::Instant::now().saturating_duration_since(due));
        }
        worst
    });

    let frame = Arc::new(Frame::from_f32_vec(vec![0.1f32; 16], 4, 4, 1).unwrap());
    assert!(
        offer_plate_solve(&state, &tokio::runtime::Handle::current(), frame, SolveSource::Main),
        "an idle Push-To task must take the frame"
    );

    let worst = heartbeat.await.expect("the heartbeat task");
    let deadline = Instant::now() + Duration::from_secs(5);
    while offers.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert_eq!(offers.load(Ordering::SeqCst), 1, "the offer must still reach the plugin");
    let threads = threads.lock().unwrap().clone();
    assert!(
        threads.iter().all(|name| name.starts_with("push-to")),
        "Push-To's work ran on {threads:?}"
    );
    assert!(
        worst < BLOCK / 2,
        "the server's runtime fell {worst:?} behind while Push-To worked"
    );
}
