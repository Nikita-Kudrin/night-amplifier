//! An all-optional JSON request body.
//!
//! Extracting one as `Option<Json<T>>` rejects a request that declares
//! `application/json` and then sends nothing — which is what a client does when it has
//! nothing to say, and what the web client's own Stop does. Every field of these bodies
//! is `#[serde(default)]`, so an absent body *is* the default rather than a malformed
//! one, and only bytes that are present and unparseable are an error.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use super::super::dto::ApiResponse;

pub struct OptionalBody<T>(pub T);

impl<T, S> FromRequest<S> for OptionalBody<T>
where
    T: DeserializeOwned + Default,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state)
            .await
            .map_err(|e| bad_request(e.body_text()))?;
        if bytes.is_empty() {
            return Ok(Self(T::default()));
        }
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|e| bad_request(e.to_string()))
    }
}

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, ApiResponse::err::<()>(message)).into_response()
}
