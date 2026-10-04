//! `GET /api/ai-compute`: the AI denoiser's hardware benchmark and where the network runs,
//! for the "Benchmarking hardware…" overlay and the "AI compute" selector.
//! `POST /api/ai-compute/benchmark`: Measure again.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

use crate::render::denoise::ai;
use crate::server::dto::ApiResponse;
use crate::server::error::ApiError;
use crate::server::state::{AppState, CaptureState};

/// Resolved for the saved "AI compute" choice, so `effective` and `notice` describe what the
/// observer has actually selected.
pub async fn get_ai_compute(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let preference = state.settings.read().await.denoise.ai_compute;
    (StatusCode::OK, ApiResponse::ok(ai::compute_report(&state.plugins, preference)))
}

/// Measure again, refused while a capture runs. Answers with the report as it stands; the
/// overlay follows the benchmark through `ai_compute_changed`.
pub async fn remeasure(State(state): State<Arc<AppState>>) -> axum::response::Response {
    if capture_running(state.capture_state().await) {
        let e = ApiError::BenchmarkDuringCapture;
        return (e.status_code(), ApiResponse::err::<()>(e.to_string())).into_response();
    }
    ai::remeasure(&state.plugins);
    let preference = state.settings.read().await.denoise.ai_compute;
    (StatusCode::OK, ApiResponse::ok(ai::compute_report(&state.plugins, preference))).into_response()
}

/// Whether a capture session is under way — paused for recovery included. The benchmark
/// never starts under one: it would take the CPU from stacking and store timings measured
/// under that load.
pub(crate) fn capture_running(state: CaptureState) -> bool {
    !matches!(state, CaptureState::Idle | CaptureState::Error)
}
