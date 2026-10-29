//! Tests for capabilities endpoint

use std::sync::Arc;

use axum::http::StatusCode;

use super::helpers::*;
use crate::session::state::AppState;

#[tokio::test]
async fn test_get_capabilities_planetary_always_enabled() {
    let state = create_test_state();
    let app = create_test_router(state);

    let (status, json) = get_json(&app, "/api/capabilities").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["success"], true);

    let data = &json["data"];

    // has_pro should be false in basic community version
    assert_eq!(data["has_pro"], false);

    // planetary should be present and advanced_stacking should be true
    assert!(data["planetary"]["advanced_stacking"].is_boolean());
    assert_eq!(data["planetary"]["advanced_stacking"], true);
}

/// The UI shows its diagnostics, and offers guide-frame saving, on this flag; the server
/// saves guide frames on `AppState::debug_mode`. Reported from that one value, the two
/// cannot disagree the way two `tracing::enabled!` call sites in different crates could.
#[tokio::test]
async fn debug_logging_reports_the_state_s_debug_mode() {
    for debug_mode in [false, true] {
        let (mut state, _disk_writer) = AppState::new_for_testing();
        state.set_debug_mode(debug_mode);
        let app = create_test_router(Arc::new(state));

        let (status, json) = get_json(&app, "/api/capabilities").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["debug_logging"], debug_mode);
    }
}
