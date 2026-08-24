//! Serves the VitePress manual from assets embedded at compile time.
//!
//! The `manual/.vitepress/dist/` directory is baked into the
//! binary via `rust_embed`.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "manual/.vitepress/dist/"]
struct ManualAssets;

/// Axum handler that serves files from the embedded `manual/.vitepress/dist/` bundle.
pub async fn serve_manual(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches("/night-amplifier");
    path = path.trim_start_matches('/');

    if path.is_empty() {
        path = "index.html";
    }

    // Try the exact path first
    let (data, serve_path) = match ManualAssets::get(path) {
        Some(content) => (content, path.to_string()),
        None => {
            // VitePress clean URLs: `/stacking` -> `stacking.html`
            let html_path = format!("{}.html", path);
            match ManualAssets::get(&html_path) {
                Some(content) => (content, html_path),
                None => {
                    // Try directory index
                    let index_path = format!("{}/index.html", path)
                        .trim_start_matches('/')
                        .to_string();
                    match ManualAssets::get(&index_path) {
                        Some(content) => (content, index_path),
                        None => match ManualAssets::get("404.html") {
                            Some(content) => (content, "404.html".to_string()),
                            None => return StatusCode::NOT_FOUND.into_response(),
                        },
                    }
                }
            }
        }
    };

    let mime = mime_guess::from_path(&serve_path).first_or_octet_stream();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime.as_ref().to_string())],
        data.data.to_vec(),
    )
        .into_response()
}
