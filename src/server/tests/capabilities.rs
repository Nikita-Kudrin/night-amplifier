//! Tests for capabilities endpoint

use axum::http::StatusCode;
use std::sync::Arc;

use super::helpers::*;

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
