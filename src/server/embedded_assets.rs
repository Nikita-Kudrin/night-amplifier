//! Serves the Vue 3 frontend from assets embedded at compile time.
//!
//! The `web/dist/` directory (Vite production build output) is baked into the
//! binary via `rust_embed`. This lets us ship a single executable with no
//! external files required.
//!
//! In development the server can still serve from the filesystem (see
//! `ServerConfig::static_dir`); this module is the fallback when that
//! directory is absent.

use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;
use std::borrow::Cow;

#[derive(Embed)]
#[folder = "web/dist/"]
struct WebAssets;

/// Vite fingerprints everything under this prefix, so a URL under it can only ever
/// name one byte sequence and is safe to cache until the heat death of the browser.
const HASHED_ASSET_PREFIX: &str = "assets/";

/// A year, the maximum `max-age` HTTP defines a meaning for.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Store it, but check with us before using it. `index.html` names the hashed bundles,
/// so serving a stale one points the browser at files an upgraded binary no longer has.
const REVALIDATE_CACHE: &str = "no-cache";

/// One file out of a frontend bundle: its bytes and the hash the validator is built from.
struct Asset {
    data: Cow<'static, [u8]>,
    sha256: [u8; 32],
}

/// The bundle a request is answered from.
///
/// A port rather than a direct call into `WebAssets`, because the rules below — SPA
/// fallback, cache policy, revalidation — say nothing about what Vite emitted. Binding
/// them to `web/dist/` made all of them fail wherever `npm run build` had not run, CI
/// included; the tests supply an in-source bundle through this same port instead.
trait AssetBundle {
    fn get(path: &str) -> Option<Asset>;
}

impl AssetBundle for WebAssets {
    fn get(path: &str) -> Option<Asset> {
        <Self as Embed>::get(path).map(|file| Asset {
            sha256: file.metadata.sha256_hash(),
            data: file.data,
        })
    }
}

/// Axum handler that serves files from the embedded `web/dist/` bundle.
///
/// Unknown paths fall back to `index.html` so the Vue SPA handles client-side routes
/// like `/eyepiece_quality` — but only paths that could *be* a route. A request that
/// names a file (`/assets/index-OLD.js`) gets a 404 instead: answering it with HTML at
/// status 200 makes Chromium's strict MIME check silently refuse the module script,
/// leaving a black page with nothing in the log. See `spa_fallback_is_not_for_files`.
pub async fn serve_embedded(uri: Uri, headers: HeaderMap) -> Response {
    serve_from::<WebAssets>(&uri, &headers)
}

/// The routing and caching rules, over whichever bundle the caller names.
fn serve_from<B: AssetBundle>(uri: &Uri, headers: &HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');

    let (file, serve_path) = match B::get(path) {
        Some(content) => (content, path),
        None if names_a_file(path) => return StatusCode::NOT_FOUND.into_response(),
        None => match B::get("index.html") {
            Some(content) => (content, "index.html"),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };

    let cache_control = if serve_path.starts_with(HASHED_ASSET_PREFIX) {
        IMMUTABLE_CACHE
    } else {
        REVALIDATE_CACHE
    };
    let etag = etag_for(&file.sha256);

    if if_none_match_hit(headers, &etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, cache_control.to_string()),
            ],
        )
            .into_response();
    }

    let mime = mime_guess::from_path(serve_path).first_or_octet_stream();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.as_ref().to_string()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache_control.to_string()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

/// Whether a request path names a file rather than a client-side route.
///
/// The test is a dot in the last segment: SPA routes are word-shaped (`/eyepiece`,
/// `/eyepiece_quality`) and static files are not. A dotted directory earlier in the
/// path (`/v1.2/settings`) is still a route.
fn names_a_file(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|last| last.contains('.'))
}

/// A strong entity tag over the embedded bytes. Hex rather than the raw hash so the
/// value survives being a header, and quoted because RFC 9110 requires it.
fn etag_for(hash: &[u8; 32]) -> String {
    let mut tag = String::with_capacity(2 + 32);
    tag.push('"');
    // 16 hex characters of SHA-256 is a collision every 4 billion assets; the bundle
    // has four.
    for byte in &hash[..8] {
        tag.push_str(&format!("{:02x}", byte));
    }
    tag.push('"');
    tag
}

/// Whether the client already holds this exact entity.
///
/// `*` matches anything we have. Otherwise the header is a comma-separated list, which
/// a browser sends verbatim from its cache, so compare tags rather than the whole
/// string.
fn if_none_match_hit(headers: &HeaderMap, etag: &str) -> bool {
    let Some(value) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    value
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == etag)
}

/// Whether `npm run build` had run when this binary was built. `web/dist/` is
/// git-ignored, so a checkout that never built the frontend embeds nothing and the
/// server 404s every page — a precondition the `frontend_serving` tests check before
/// asserting anything about the real bundle.
#[cfg(test)]
pub(crate) fn bundle_is_built() -> bool {
    <WebAssets as AssetBundle>::get("index.html").is_some()
}

/// The same routing rule for the filesystem (`static_dir`) branch, which otherwise
/// answers a missing `/assets/*.js` with `index.html` exactly as the embedded path did.
pub async fn serve_disk_spa_fallback(uri: Uri, index_path: std::path::PathBuf) -> Response {
    if names_a_file(uri.path().trim_start_matches('/')) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(&index_path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static(REVALIDATE_CACHE)),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::Router;
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;

    /// Stands in for the Vite build, shaped like it: an SPA shell naming one hashed
    /// bundle. It lives in source rather than on disk because every disk-backed version
    /// of it has gone missing — `web/dist/` is git-ignored and empty until `npm run
    /// build`, and a fixture directory is one `git clean` or one forgotten `git add`
    /// away from breaking the build for everyone. The hashes are real SHA-256 over
    /// these bytes, the same validator `rust_embed` computes, so the ETag tests below
    /// are not asserting against fabricated values.
    struct Fixture;

    const FIXTURE_ASSET: &str = "assets/index-TEST0001.js";
    const FIXTURE_INDEX_HTML: &[u8] = br#"<!doctype html>
<html lang="en">
  <head><script type="module" crossorigin src="/assets/index-TEST0001.js"></script></head>
  <body><div id="app"></div></body>
</html>
"#;
    const FIXTURE_ASSET_BODY: &[u8] = b"export const fixture = 'a hashed bundle';\n";

    impl AssetBundle for Fixture {
        fn get(path: &str) -> Option<Asset> {
            let data: &'static [u8] = match path {
                "index.html" => FIXTURE_INDEX_HTML,
                FIXTURE_ASSET => FIXTURE_ASSET_BODY,
                _ => return None,
            };
            Some(Asset {
                data: Cow::Borrowed(data),
                sha256: Sha256::digest(data).into(),
            })
        }
    }

    fn app() -> Router {
        Router::new().fallback(|uri: Uri, headers: HeaderMap| async move {
            serve_from::<Fixture>(&uri, &headers)
        })
    }

    /// The fixture must keep looking like a Vite build, or the tests below stop
    /// exercising the rules they name.
    #[test]
    fn the_fixture_bundle_is_shaped_like_a_build() {
        assert!(FIXTURE_ASSET.starts_with(HASHED_ASSET_PREFIX));
        assert!(Fixture::get("index.html").is_some());
        assert!(Fixture::get(FIXTURE_ASSET).is_some());
        assert_ne!(
            Fixture::get("index.html").unwrap().sha256,
            Fixture::get(FIXTURE_ASSET).unwrap().sha256
        );
    }

    async fn get(uri: &str, if_none_match: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = Request::builder().uri(uri);
        if let Some(tag) = if_none_match {
            request = request.header(header::IF_NONE_MATCH, tag);
        }
        let response = app()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, body.to_vec())
    }

    fn header(headers: &HeaderMap, name: header::HeaderName) -> String {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    /// The failure that took the kiosk display down: an upgraded binary no longer has
    /// the asset hash a cached `index.html` names, and the SPA fallback answered it with
    /// HTML at status 200. Chromium's strict MIME check then refuses the module script
    /// without a console error, leaving a black screen and nothing to diagnose. A 404 is
    /// the difference between a visible failure and an invisible one.
    #[tokio::test]
    async fn spa_fallback_is_not_for_files() {
        let (status, headers, _) = get("/assets/index-DEADBEEF.js", None).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            !header(&headers, header::CONTENT_TYPE).starts_with("text/html"),
            "a missing script was answered with HTML"
        );
    }

    /// The fallback still has to work for the routes it exists for — the kiosk opens one.
    #[tokio::test]
    async fn spa_routes_still_reach_index_html() {
        for route in ["/eyepiece_quality", "/eyepiece", "/"] {
            let (status, headers, body) = get(route, None).await;
            assert_eq!(status, StatusCode::OK, "{route}");
            assert!(header(&headers, header::CONTENT_TYPE).starts_with("text/html"), "{route}");
            assert!(
                String::from_utf8_lossy(&body).contains("<div id=\"app\">"),
                "{route} did not serve the SPA shell"
            );
        }
    }

    /// Hashed filenames can only ever name one byte sequence, so the kiosk should stop
    /// re-downloading the bundle on every restart.
    #[tokio::test]
    async fn hashed_assets_are_cached_forever() {
        let (status, headers, _) = get(&format!("/{FIXTURE_ASSET}"), None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(header(&headers, header::CACHE_CONTROL), IMMUTABLE_CACHE);
        assert!(!header(&headers, header::ETAG).is_empty());
    }

    /// `index.html` names the hashed bundles, so it is the one file that must never be
    /// used without checking — a stale copy is exactly what points the browser at assets
    /// an upgraded binary no longer has.
    #[tokio::test]
    async fn index_html_is_revalidated_and_never_immutable() {
        let (status, headers, _) = get("/", None).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(header(&headers, header::CACHE_CONTROL), REVALIDATE_CACHE);
        assert!(!header(&headers, header::ETAG).is_empty());
    }

    /// Revalidation has to actually be cheap, or `no-cache` just means "download it
    /// again every time" — which is what the handler did before, having no validator at
    /// all to offer.
    #[tokio::test]
    async fn a_matching_etag_returns_304_with_no_body() {
        let (_, headers, body) = get("/", None).await;
        let etag = header(&headers, header::ETAG);
        assert!(!body.is_empty());

        let (status, headers, body) = get("/", Some(&etag)).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert!(body.is_empty());
        assert_eq!(header(&headers, header::CACHE_CONTROL), REVALIDATE_CACHE);
    }

    /// A tag from a previous build must not suppress the new bundle.
    #[tokio::test]
    async fn a_stale_etag_returns_the_new_content() {
        let (status, _, body) = get("/", Some("\"0000000000000000\"")).await;

        assert_eq!(status, StatusCode::OK);
        assert!(!body.is_empty());
    }

    /// Different files must not share a tag, or a 304 would serve the wrong bytes.
    #[tokio::test]
    async fn etags_distinguish_files() {
        let (_, index_headers, _) = get("/", None).await;
        let (_, asset_headers, _) = get(&format!("/{FIXTURE_ASSET}"), None).await;

        assert_ne!(
            header(&index_headers, header::ETAG),
            header(&asset_headers, header::ETAG)
        );
    }

    /// The dev branch had the same hole: `ServeDir`'s `ServeFile` fallback answers a
    /// missing bundle with `index.html` too, so `npm run build` output that has moved on
    /// fails exactly as silently as it did in production.
    #[tokio::test]
    async fn the_disk_fallback_also_404s_files() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.html");
        std::fs::write(&index, FIXTURE_INDEX_HTML).unwrap();

        let response =
            serve_disk_spa_fallback("/assets/index-DEADBEEF.js".parse().unwrap(), index.clone())
                .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = serve_disk_spa_fallback("/eyepiece_quality".parse().unwrap(), index).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some(REVALIDATE_CACHE)
        );
    }

    #[test]
    fn spa_routes_are_not_files() {
        assert!(!names_a_file("eyepiece"));
        assert!(!names_a_file("eyepiece_quality"));
        assert!(!names_a_file(""));
        assert!(!names_a_file("v1.2/settings"));
    }

    #[test]
    fn asset_urls_are_files() {
        assert!(names_a_file("assets/index-Y2ej2Oeh.js"));
        assert!(names_a_file("assets/index-BfrbfdH8.css"));
        assert!(names_a_file("favicon.ico"));
        assert!(names_a_file("index.html"));
    }

    #[test]
    fn etag_is_quoted_hex_of_the_content_hash() {
        let tag = etag_for(&[0xab; 32]);
        assert_eq!(tag, "\"abababababababab\"");
        assert_ne!(etag_for(&[0x00; 32]), tag);
    }

    #[test]
    fn if_none_match_matches_a_tag_in_a_list() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("\"aaaa\", W/\"bbbb\""),
        );
        assert!(if_none_match_hit(&headers, "\"aaaa\""));
        assert!(if_none_match_hit(&headers, "\"bbbb\""));
        assert!(!if_none_match_hit(&headers, "\"cccc\""));
    }

    #[test]
    fn if_none_match_wildcard_and_absence() {
        let mut headers = HeaderMap::new();
        assert!(!if_none_match_hit(&headers, "\"aaaa\""));
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        assert!(if_none_match_hit(&headers, "\"aaaa\""));
    }
}
