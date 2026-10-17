//! One log line for every API request answered with an error.
//!
//! Handlers return their errors to the client and log nothing, so on 2026-09-20 the
//! observer's "The guide camera is already running" and every refused Disconnect left no
//! trace to match the report against. Only error responses are read, and they are small.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use tracing::{debug, info, warn};

/// Larger than any error body a handler builds; a body past it is logged unread.
const MAX_ERROR_BODY: usize = 64 * 1024;

pub async fn log_failures(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(%method, path, status = status.as_u16(), error = %e, "API request failed; its body could not be read");
            return Response::from_parts(parts, Body::empty());
        }
    };
    let error = error_message(&bytes);
    let status_code = status.as_u16();
    // A response asking to be retried belongs to a polling protocol (the eyepiece
    // snapshot, the start-up benchmark): one line per poll would bury everything else.
    if parts.headers.contains_key(header::RETRY_AFTER) {
        debug!(%method, path, status = status_code, error, "API request deferred");
    } else if status.is_server_error() {
        warn!(%method, path, status = status_code, error, "API request failed");
    } else {
        info!(%method, path, status = status_code, error, "API request refused");
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// The `error` field of an `ApiResponse` body, or the start of whatever else it was.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json.get("error")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    #[test]
    fn the_message_is_the_error_field_of_an_api_response() {
        assert_eq!(
            error_message(br#"{"success":false,"error":"Camera 'Ares' is busy"}"#),
            "Camera 'Ares' is busy"
        );
        assert_eq!(error_message(b"plain text"), "plain text");
    }

    /// The middleware reads the body to log it; the client must still get every byte.
    #[tokio::test]
    async fn an_error_response_reaches_the_client_unchanged() {
        let body = r#"{"success":false,"error":"refused"}"#;
        let app = Router::new()
            .route("/refused", get(move || async move { (StatusCode::CONFLICT, body) }))
            .layer(axum::middleware::from_fn(log_failures));

        let response = app
            .oneshot(Request::builder().uri("/refused").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], body.as_bytes());
    }
}
