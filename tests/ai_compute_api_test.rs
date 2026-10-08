//! `GET /api/ai-compute` and the capture gate, against a fake AI denoise plugin. The
//! server is built by `Server::new`, which takes the process's installed plugins and
//! licence flag, so this is its own binary and its tests run one at a time.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use night_amplifier::license::{LicenseDetails, LICENSE_UPDATER, PRO_LICENSE_ACTIVE};
use night_amplifier::plugins::{self, Plugins};
use night_amplifier::render::denoise::ai;
use night_amplifier::render::denoise::DenoiseSettings;
use night_amplifier::render::{
    AiComputePreference, AiComputeReport, AiDenoiseConfig, AiDenoisePlugin, BenchmarkState,
    ComputeRung, DenoiseScratch, RungReport,
};
use night_amplifier::session::state::CaptureState;
use night_amplifier::server::{Server, ServerConfig};
use serde_json::Value;
use serial_test::serial;
use tower::ServiceExt;

static BENCHMARKING: AtomicBool = AtomicBool::new(true);
static CHECKING: AtomicBool = AtomicBool::new(false);
static STARTS: AtomicU32 = AtomicU32::new(0);
static REMEASURES: AtomicU32 = AtomicU32::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(41);
static ASKED_FOR: Mutex<Option<AiComputePreference>> = Mutex::new(None);

struct FakePlugin;

impl AiDenoisePlugin for FakePlugin {
    fn config(&self, _settings: &DenoiseSettings) -> AiDenoiseConfig {
        AiDenoiseConfig::OFF
    }

    fn denoise_display_rgb(&self, _: &mut [f32], _: usize, _: usize, _: &AiDenoiseConfig, _: &mut DenoiseScratch) {}

    fn start_benchmark(&self) {
        STARTS.fetch_add(1, Ordering::SeqCst);
    }

    fn remeasure(&self) {
        REMEASURES.fetch_add(1, Ordering::SeqCst);
    }

    fn compute_report(&self, preference: AiComputePreference) -> AiComputeReport {
        *ASKED_FOR.lock().unwrap() = Some(preference);
        AiComputeReport {
            generation: GENERATION.load(Ordering::SeqCst),
            state: if BENCHMARKING.load(Ordering::SeqCst) {
                BenchmarkState::Benchmarking
            } else if CHECKING.load(Ordering::SeqCst) {
                BenchmarkState::Checking
            } else {
                BenchmarkState::Ready
            },
            rungs: vec![RungReport::absent(ComputeRung::Npu, "none found")],
            auto: Some(ComputeRung::Cpu),
            effective: Some(ComputeRung::Cpu),
            ..AiComputeReport::unavailable()
        }
    }

    fn compute_generation(&self) -> u64 {
        GENERATION.load(Ordering::SeqCst)
    }
}

/// `AppState::new_for_testing` is test-only in the library, so this binary runs from a
/// fresh directory instead: no real `settings.json` or `./captures` is touched.
fn isolate_cwd() {
    let dir = std::env::temp_dir().join(format!("night_amplifier_ai_compute_api_test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_current_dir(&dir).unwrap();
}

/// The fake plugin, a licence and the working directory, once per process.
fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        isolate_cwd();
        plugins::install(Plugins::none().with_ai_denoise(Arc::new(FakePlugin)));
    });
    PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);
}

async fn call(app: &axum::Router, method: &str, uri: &str) -> (StatusCode, Option<String>, Value) {
    call_with(app, method, uri, None).await
}

async fn call_with(app: &axum::Router, method: &str, uri: &str, json: Option<Value>) -> (StatusCode, Option<String>, Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match json {
        Some(json) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string())),
        None => request.body(Body::empty()),
    };
    let response = app.clone().oneshot(request.unwrap()).await.unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, retry_after, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
#[serial]
async fn the_report_and_the_capture_gate_follow_the_plugin() {
    install();
    BENCHMARKING.store(true, Ordering::SeqCst);
    GENERATION.store(41, Ordering::SeqCst);

    let server = Server::new(ServerConfig::new());
    server.state().settings.update(|s| s.denoise.ai_compute = AiComputePreference::IntegratedGpu);
    let app = server.build_router();

    // The report is resolved for the saved choice.
    let (status, _, body) = call(&app, "GET", "/api/ai-compute").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["state"], "benchmarking");
    assert_eq!(body["data"]["generation"], 41);
    assert_eq!(body["data"]["rungs"][0]["rung"], "npu");
    assert_eq!(*ASKED_FOR.lock().unwrap(), Some(AiComputePreference::IntegratedGpu));

    // A capture waits for the benchmark, and says how long to wait.
    let (status, retry_after, body) = call(&app, "POST", "/api/capture/start").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(retry_after.as_deref(), Some("5"));
    assert!(body["error"].as_str().unwrap().contains("Benchmarking"), "{body}");
    assert!(ai::benchmark_pending(&Plugins::installed()));

    // Finished: the gate opens. No camera is connected, so the start fails on that instead.
    BENCHMARKING.store(false, Ordering::SeqCst);
    let (status, retry_after, _) = call(&app, "POST", "/api/capture/start").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(retry_after, None);

    // The watcher's counter and the startup hook reach the plugin.
    GENERATION.store(42, Ordering::SeqCst);
    assert_eq!(ai::compute_generation(&Plugins::installed()), 42);
    let before = STARTS.load(Ordering::SeqCst);
    ai::start_benchmark(&Plugins::installed());
    assert_eq!(STARTS.load(Ordering::SeqCst), before + 1);

    // Without a licence the plugin is not asked: no report, no gate, no benchmark.
    BENCHMARKING.store(true, Ordering::SeqCst);
    PRO_LICENSE_ACTIVE.store(false, Ordering::SeqCst);
    let (_, _, body) = call(&app, "GET", "/api/ai-compute").await;
    assert_eq!(body["data"]["state"], "unavailable");
    assert!(!ai::benchmark_pending(&Plugins::installed()));
    let (status, _, _) = call(&app, "POST", "/api/capture/start").await;
    assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
    let before = (STARTS.load(Ordering::SeqCst), REMEASURES.load(Ordering::SeqCst));
    ai::start_benchmark(&Plugins::installed());
    ai::remeasure(&Plugins::installed());
    assert_eq!((STARTS.load(Ordering::SeqCst), REMEASURES.load(Ordering::SeqCst)), before);
}

/// Discovery (`Checking`) is the benchmark's first step, not its absence: a capture started
/// then is still running when `Benchmarking` begins — measured under its load, with the
/// overlay making the UI inert over a running capture. Captures wait for both.
#[tokio::test]
#[serial]
async fn a_capture_waits_while_the_hardware_is_still_being_checked() {
    install();
    BENCHMARKING.store(false, Ordering::SeqCst);
    CHECKING.store(true, Ordering::SeqCst);
    let app = Server::new(ServerConfig::new()).build_router();

    let (status, retry_after, _) = call(&app, "POST", "/api/capture/start").await;
    CHECKING.store(false, Ordering::SeqCst);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "a capture started while the hardware was being checked");
    assert_eq!(retry_after.as_deref(), Some("5"));
}

/// A licence activated mid-session must not start the benchmark under the running
/// capture: it would compete with stacking for the CPU (dropped frames), store timings
/// taken under that load for good, and the overlay would make the whole UI — Stop
/// included — inert until it ended. It starts when the capture ends instead.
#[tokio::test]
#[serial]
async fn activating_a_licence_during_a_capture_benchmarks_when_the_capture_ends() {
    install();
    BENCHMARKING.store(false, Ordering::SeqCst);
    LICENSE_UPDATER
        .set(Box::new(|_| {
            Ok(LicenseDetails {
                name: "Observer".into(),
                email: "observer@example.com".into(),
                issued_at: "2026-09-26".into(),
                expires_at: "2027-09-26".into(),
            })
        }))
        .ok();
    let server = Server::new(ServerConfig::new());
    server.state().set_capture_state(CaptureState::Capturing);
    let app = server.build_router();

    let before = STARTS.load(Ordering::SeqCst);
    let (status, _, body) = call_with(&app, "POST", "/api/about/license", Some(serde_json::json!({ "token": "t" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(STARTS.load(Ordering::SeqCst), before, "the benchmark started under a running capture");

    server.state().end_capture_state();
    assert_eq!(STARTS.load(Ordering::SeqCst), before + 1, "the capture ended and the benchmark never started");

    // With nothing running it starts at once.
    let (status, _, _) = call_with(&app, "POST", "/api/about/license", Some(serde_json::json!({ "token": "t" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(STARTS.load(Ordering::SeqCst), before + 2);
}

/// Measure again reaches the plugin only with no capture running, for the same reasons the
/// first benchmark waits for one.
#[tokio::test]
#[serial]
async fn measure_again_waits_for_the_capture_to_end() {
    install();
    BENCHMARKING.store(false, Ordering::SeqCst);
    let server = Server::new(ServerConfig::new());
    server.state().set_capture_state(CaptureState::Recovering);
    let app = server.build_router();

    let before = REMEASURES.load(Ordering::SeqCst);
    let (status, _, body) = call(&app, "POST", "/api/ai-compute/benchmark").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Stop the capture"), "{body}");
    assert_eq!(REMEASURES.load(Ordering::SeqCst), before);

    server.state().set_capture_state(CaptureState::Idle);
    let (status, _, body) = call(&app, "POST", "/api/ai-compute/benchmark").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["state"], "ready");
    assert_eq!(REMEASURES.load(Ordering::SeqCst), before + 1);
}
