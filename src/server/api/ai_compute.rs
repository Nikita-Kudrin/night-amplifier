//! `GET /api/ai-compute`: the AI denoiser's hardware benchmark and where the network runs,
//! for the "Benchmarking hardware…" overlay and the "AI compute" selector.

use axum::{extract::State, http::StatusCode, response::IntoResponse};
use std::sync::Arc;

use crate::render::denoise::ai;
use crate::server::dto::ApiResponse;
use crate::server::state::AppState;

/// Resolved for the saved "AI compute" choice, so `effective` and `notice` describe what the
/// observer has actually selected.
pub async fn get_ai_compute(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let preference = state.settings.read().await.denoise.ai_compute;
    (StatusCode::OK, ApiResponse::ok(ai::compute_report(preference)))
}
