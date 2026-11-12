//! Who is connected to the image streams, for a kiosk's watchdog (`kiosk_start.sh` in the
//! manual): Chromium staying up says nothing about its page, which can crash ("Aw, Snap!")
//! or hang while the browser runs on, on a display nobody is there to reload.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

use crate::server::dto::ApiResponse;
use crate::session::state::{AppState, StreamClientInfo};

#[derive(Debug, Default, Deserialize)]
pub struct StreamViewersQuery {
    /// Only clients of this page (`/eyepiece_quality`); every page when absent.
    #[serde(default)]
    page: Option<String>,
    /// Only clients on this machine (`true`) or off it (`false`); both when absent.
    #[serde(default)]
    local: Option<bool>,
}

impl StreamViewersQuery {
    fn matches(&self, info: &StreamClientInfo) -> bool {
        self.page.as_deref().is_none_or(|page| page == info.page) && self.local.is_none_or(|local| local == info.local)
    }
}

/// Answer of `GET /api/streams/viewers`, over the clients the query names.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamViewersResponse {
    pub viewers: usize,
    /// Whole seconds since the freshest of them last sent anything (it pings every 30 s);
    /// `null` with none.
    pub heard_s: Option<u64>,
}

/// GET /api/streams/viewers?page=&local=
pub async fn get_stream_viewers(
    Query(query): Query<StreamViewersQuery>,
    State(state): State<Arc<AppState>>,
) -> Json<ApiResponse<StreamViewersResponse>> {
    ApiResponse::ok(summarize(&state.stream_clients.snapshot(), &query, Instant::now()))
}

fn summarize(clients: &[StreamClientInfo], query: &StreamViewersQuery, now: Instant) -> StreamViewersResponse {
    let matching: Vec<_> = clients.iter().filter(|info| query.matches(info)).collect();
    StreamViewersResponse {
        viewers: matching.len(),
        heard_s: matching
            .iter()
            .map(|info| now.saturating_duration_since(info.heard_at).as_secs())
            .min(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn client(page: &'static str, local: bool, heard_ago_s: u64, now: Instant) -> StreamClientInfo {
        let heard_at = now - Duration::from_secs(heard_ago_s);
        StreamClientInfo { page, local, connected_at: heard_at, heard_at }
    }

    fn query(page: Option<&str>, local: Option<bool>) -> StreamViewersQuery {
        StreamViewersQuery { page: page.map(str::to_owned), local }
    }

    #[test]
    fn counts_the_named_clients_and_the_freshest_of_them() {
        let now = Instant::now();
        let clients = [
            client("/eyepiece_quality", true, 95, now),
            client("/eyepiece_quality", true, 12, now),
            // A phone on the quality page must not keep a dead kiosk looking alive.
            client("/eyepiece_quality", false, 0, now),
            client("/eyepiece", true, 1, now),
        ];

        let kiosk = summarize(&clients, &query(Some("/eyepiece_quality"), Some(true)), now);
        assert_eq!(kiosk, StreamViewersResponse { viewers: 2, heard_s: Some(12) });

        let everyone = summarize(&clients, &query(None, None), now);
        assert_eq!(everyone, StreamViewersResponse { viewers: 4, heard_s: Some(0) });

        let remote = summarize(&clients, &query(None, Some(false)), now);
        assert_eq!(remote, StreamViewersResponse { viewers: 1, heard_s: Some(0) });
    }

    #[test]
    fn says_null_rather_than_zero_with_nobody_there() {
        let now = Instant::now();
        let clients = [client("/eyepiece", true, 0, now)];

        let none = summarize(&clients, &query(Some("/eyepiece_quality"), None), now);
        assert_eq!(none, StreamViewersResponse { viewers: 0, heard_s: None });
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, r#"{"viewers":0,"heard_s":null}"#);
    }
}
