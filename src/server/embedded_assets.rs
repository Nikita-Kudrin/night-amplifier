//! Serves the Vue 3 frontend from assets embedded at compile time.
//!
//! The `web/dist/` directory (Vite production build output) is baked into the
//! binary via `rust_embed`. This lets us ship a single executable with no
//! external files required.
//!
//! In development the server can still serve from the filesystem (see
//! `ServerConfig::static_dir`); this module is the fallback when that
//! directory is absent.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/dist/"]
struct WebAssets;

/// Axum handler that serves files from the embedded `web/dist/` bundle.
///
/// Unknown paths fall back to `index.html` so that the Vue SPA router
/// handles client-side navigation correctly.
pub async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try the exact path first; fall back to index.html for SPA routing.
    let (data, serve_path) = match WebAssets::get(path) {
        Some(content) => (content, path),
        None => match WebAssets::get("index.html") {
            Some(content) => (content, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };

    let mime = mime_guess::from_path(serve_path).first_or_octet_stream();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime.as_ref().to_string())],
        data.data.to_vec(),
    )
        .into_response()
}
