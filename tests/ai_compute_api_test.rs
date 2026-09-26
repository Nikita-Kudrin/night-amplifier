//! `GET /api/ai-compute` and the capture gate, against a fake AI denoise plugin. The
//! plugin registry is process-wide, which is why this is its own binary with one test.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use night_amplifier::license::PRO_LICENSE_ACTIVE;
use night_amplifier::render::denoise::ai;
use night_amplifier::render::{
    AiComputePreference, AiComputeReport, AiDenoiseConfig, AiDenoisePlugin, BenchmarkState,
    ComputeRung, DenoiseScratch, RungReport, AI_DENOISE_PLUGIN,
};
use night_amplifier::server::state::DenoiseSettings;
use night_amplifier::server::{Server, ServerConfig};
use serde_json::Value;
use tower::ServiceExt;

static BENCHMARKING: AtomicBool = AtomicBool::new(true);
static STARTS: AtomicU32 = AtomicU32::new(0);
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

    fn compute_report(&self, preference: AiComputePreference) -> AiComputeReport {
        *ASKED_FOR.lock().unwrap() = Some(preference);
        AiComputeReport {
            generation: GENERATION.load(Ordering::SeqCst),
            state: if BENCHMARKING.load(Ordering::SeqCst) {
                BenchmarkState::Benchmarking
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

async fn call(app: &axum::Router, method: &str, uri: &str) -> (StatusCode, Option<String>, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().method(method).uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, retry_after, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_report_and_the_capture_gate_follow_the_plugin() {
    isolate_cwd();
    AI_DENOISE_PLUGIN.set(Box::new(FakePlugin)).ok().expect("nothing else registers a plugin here");
    PRO_LICENSE_ACTIVE.store(true, Ordering::SeqCst);

    let server = Server::new(ServerConfig::new());
    server.state().settings.write().await.denoise.ai_compute = AiComputePreference::IntegratedGpu;
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
    assert!(ai::benchmark_running());

    // Finished: the gate opens. No camera is connected, so the start fails on that instead.
    BENCHMARKING.store(false, Ordering::SeqCst);
    let (status, retry_after, _) = call(&app, "POST", "/api/capture/start").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(retry_after, None);

    // The watcher's counter and the startup hook reach the plugin.
    GENERATION.store(42, Ordering::SeqCst);
    assert_eq!(ai::compute_generation(), 42);
    let before = STARTS.load(Ordering::SeqCst);
    ai::start_benchmark();
    assert_eq!(STARTS.load(Ordering::SeqCst), before + 1);

    // Without a licence the plugin is not asked: no report, no gate, no benchmark.
    BENCHMARKING.store(true, Ordering::SeqCst);
    PRO_LICENSE_ACTIVE.store(false, Ordering::SeqCst);
    let (_, _, body) = call(&app, "GET", "/api/ai-compute").await;
    assert_eq!(body["data"]["state"], "unavailable");
    assert!(!ai::benchmark_running());
    let (status, _, _) = call(&app, "POST", "/api/capture/start").await;
    assert_ne!(status, StatusCode::SERVICE_UNAVAILABLE);
    let before = STARTS.load(Ordering::SeqCst);
    ai::start_benchmark();
    assert_eq!(STARTS.load(Ordering::SeqCst), before);
}
