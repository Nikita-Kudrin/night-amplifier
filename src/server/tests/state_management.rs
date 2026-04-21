//! Tests for state management

use std::sync::Arc;

use super::helpers::*;
use crate::disk_writer::DiskWriterConfig;
use crate::server::events::ServerEvent;
use crate::server::state::*;

// ============================================================================
// State Management Tests
// ============================================================================

#[tokio::test]
async fn test_app_state_event_subscription() {
    let state = create_test_state();

    let mut rx1 = state.subscribe_events();
    let mut rx2 = state.subscribe_events();

    state.set_capture_state(CaptureState::Capturing).await;

    // Both receivers should get the event
    let event1 = rx1.recv().await.unwrap();
    let event2 = rx2.recv().await.unwrap();

    assert!(matches!(event1, ServerEvent::StateChanged { .. }));
    assert!(matches!(event2, ServerEvent::StateChanged { .. }));
}

#[tokio::test]
async fn test_app_state_frame_counter() {
    let state = create_test_state();

    let initial = state
        .frame_counter
        .load(std::sync::atomic::Ordering::SeqCst);
    state.set_latest_frame(vec![1, 2, 3]).await;
    let after = state
        .frame_counter
        .load(std::sync::atomic::Ordering::SeqCst);

    assert_eq!(after, initial + 1);
}

#[tokio::test]
async fn test_app_state_reset_session() {
    let state = create_test_state();

    // Add some data
    state.frame_captured(true).await;
    state.frame_captured(true).await;
    state.frame_rejected("test".to_string()).await;

    // Reset
    state.reset_session().await;

    let session = state.session.read().await;
    assert_eq!(session.frame_count, 0);
    assert_eq!(session.stacked_count, 0);
    assert_eq!(session.rejected_count, 0);
    assert!(session.started_at.is_some());
}

// ============================================================================
// Concurrent Access Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_settings_updates() {
    use axum::http::StatusCode;
    use serde_json::json;

    let state = create_test_state();
    let app = create_test_router(Arc::clone(&state));

    // Spawn multiple concurrent updates
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let app = app.clone();
            tokio::spawn(async move { post_json(&app, "/api/settings", json!({"gain": i})).await })
        })
        .collect();

    // Wait for all to complete
    for handle in handles {
        let (status, _) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK);
    }

    // Verify state is consistent (one of the values should have won)
    let settings = state.settings.read().await;
    assert!(settings.gain >= 0 && settings.gain < 10);
}

#[tokio::test]
async fn test_concurrent_status_reads() {
    use axum::http::StatusCode;

    let state = create_test_state();
    let app = create_test_router(state);

    // Spawn multiple concurrent reads
    let handles: Vec<_> = (0..20)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { get_json(&app, "/api/capture/status").await })
        })
        .collect();

    // All should succeed
    for handle in handles {
        let (status, json) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["success"], true);
    }
}

// ============================================================================
// Capture State Transition Tests
// ============================================================================

#[tokio::test]
async fn test_capture_state_transitions() {
    let state = create_test_state();

    // Initial state
    assert_eq!(state.capture_state().await, CaptureState::Idle);

    // Transition to Starting
    state.set_capture_state(CaptureState::Starting).await;
    assert_eq!(state.capture_state().await, CaptureState::Starting);

    // Transition to Capturing
    state.set_capture_state(CaptureState::Capturing).await;
    assert_eq!(state.capture_state().await, CaptureState::Capturing);

    // Transition to Stopping
    state.set_capture_state(CaptureState::Stopping).await;
    assert_eq!(state.capture_state().await, CaptureState::Stopping);

    // Transition to Idle
    state.set_capture_state(CaptureState::Idle).await;
    assert_eq!(state.capture_state().await, CaptureState::Idle);

    // Transition to Error
    state.set_capture_state(CaptureState::Error).await;
    assert_eq!(state.capture_state().await, CaptureState::Error);
}

#[tokio::test]
async fn test_capture_state_broadcasts_events() {
    let state = create_test_state();
    let mut events_rx = state.subscribe_events();

    // Set state
    state.set_capture_state(CaptureState::Capturing).await;

    // Should receive event
    let event = events_rx.recv().await.unwrap();
    assert!(matches!(event, ServerEvent::StateChanged { .. }));
}

// ============================================================================
// Frame Tracking Tests
// ============================================================================

#[tokio::test]
async fn test_frame_captured_increments_counts() {
    let state = create_test_state();
    state.reset_session().await;

    // Capture stacked frame
    state.frame_captured(true).await;
    let session = state.session.read().await;
    assert_eq!(session.frame_count, 1);
    assert_eq!(session.stacked_count, 1);
    drop(session);

    // Capture non-stacked frame
    state.frame_captured(false).await;
    let session = state.session.read().await;
    assert_eq!(session.frame_count, 2);
    assert_eq!(session.stacked_count, 1);
}

#[tokio::test]
async fn test_frame_rejected_increments_counts() {
    let state = create_test_state();
    state.reset_session().await;

    state.frame_rejected("Test reason".to_string()).await;

    let session = state.session.read().await;
    assert_eq!(session.frame_count, 1);
    assert_eq!(session.rejected_count, 1);
    assert_eq!(session.stacked_count, 0);
}

#[tokio::test]
async fn test_frame_captured_broadcasts_event() {
    let state = create_test_state();
    let mut events_rx = state.subscribe_events();
    state.reset_session().await;

    state.frame_captured(true).await;

    let event = events_rx.recv().await.unwrap();
    match event {
        ServerEvent::FrameCaptured {
            frame_number,
            stacked_count,
        } => {
            assert_eq!(frame_number, 1);
            assert_eq!(stacked_count, 1);
        }
        _ => panic!("Expected FrameCaptured event"),
    }
}

#[tokio::test]
async fn test_frame_rejected_broadcasts_event() {
    let state = create_test_state();
    let mut events_rx = state.subscribe_events();
    state.reset_session().await;

    state.frame_rejected("Test reason".to_string()).await;

    let event = events_rx.recv().await.unwrap();
    match event {
        ServerEvent::FrameRejected {
            frame_number,
            stacked_count,
            reason,
        } => {
            assert_eq!(frame_number, 1);
            assert_eq!(stacked_count, 0);
            assert_eq!(reason, "Test reason");
        }
        _ => panic!("Expected FrameRejected event"),
    }
}

// ============================================================================
// Latest Frame Storage Tests
// ============================================================================

#[tokio::test]
async fn test_latest_frame_storage() {
    let state = create_test_state();

    // Initially no frame
    assert!(state.get_latest_frame().await.is_none());

    // Store a frame
    let frame_data = vec![1, 2, 3, 4, 5];
    state.set_latest_frame(frame_data.clone()).await;

    // Retrieve frame
    let retrieved = state.get_latest_frame().await.unwrap();
    assert_eq!(retrieved.as_ref(), &frame_data);
}

#[tokio::test]
async fn test_latest_frame_overwrites_previous() {
    let state = create_test_state();

    state.set_latest_frame(vec![1, 2, 3]).await;
    state.set_latest_frame(vec![4, 5, 6]).await;

    let retrieved = state.get_latest_frame().await.unwrap();
    assert_eq!(retrieved.as_ref(), &[4, 5, 6]);
}

#[tokio::test]
async fn test_frame_counter_increments() {
    let state = create_test_state();

    let initial = state
        .frame_counter
        .load(std::sync::atomic::Ordering::SeqCst);

    state.set_latest_frame(vec![1]).await;
    let after_first = state
        .frame_counter
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(after_first, initial + 1);

    state.set_latest_frame(vec![2]).await;
    let after_second = state
        .frame_counter
        .load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(after_second, initial + 2);
}

// ============================================================================
// Cancellation Tests
// ============================================================================

#[tokio::test]
async fn test_cancellation_flag() {
    let state = create_test_state();

    // Initially not cancelled
    assert!(!state.is_cancelled());

    // Request cancellation
    state.request_cancel();
    assert!(state.is_cancelled());

    // Reset cancellation
    state.reset_cancel();
    assert!(!state.is_cancelled());
}

// ============================================================================
// Session Reset Tests
// ============================================================================

#[tokio::test]
async fn test_session_reset_clears_all_counts() {
    let state = create_test_state();

    // Add some data
    state.frame_captured(true).await;
    state.frame_captured(true).await;
    state.frame_rejected("test".to_string()).await;

    // Verify data exists
    {
        let session = state.session.read().await;
        assert!(session.frame_count > 0);
    }

    // Reset session
    state.reset_session().await;

    // Verify all cleared
    let session = state.session.read().await;
    assert_eq!(session.frame_count, 0);
    assert_eq!(session.stacked_count, 0);
    assert_eq!(session.rejected_count, 0);
    assert!(session.last_error.is_none());
}

#[tokio::test]
async fn test_session_reset_sets_started_at() {
    let state = create_test_state();

    // Initially no started_at
    {
        let session = state.session.read().await;
        assert!(session.started_at.is_none());
    }

    // Reset sets timestamp
    state.reset_session().await;

    let session = state.session.read().await;
    assert!(session.started_at.is_some());

    // Should be a reasonable timestamp (within last minute)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let started = session.started_at.unwrap();
    assert!(started <= now);
    assert!(started > now - 60_000); // Within last minute
}

// ============================================================================
// Error Event Tests
// ============================================================================

#[tokio::test]
async fn test_send_error_broadcasts_event() {
    let state = create_test_state();
    let mut events_rx = state.subscribe_events();

    state.send_error("Test error message".to_string());

    let event = events_rx.recv().await.unwrap();
    match event {
        ServerEvent::Error { message } => {
            assert_eq!(message, "Test error message");
        }
        _ => panic!("Expected Error event"),
    }
}

// ============================================================================
// Disk Writer State Tests
// ============================================================================

#[tokio::test]
async fn test_app_state_has_disk_writer() {
    let (state, _disk_writer) = AppState::new();

    // Disk writer handle should be accessible
    assert!(state.disk_writer.is_enabled()); // Default enabled

    // Should be able to toggle
    state.disk_writer.set_enabled(false);
    assert!(!state.disk_writer.is_enabled());
}

#[tokio::test]
async fn test_app_state_with_disk_writer_config() {
    let config = DiskWriterConfig::new("/tmp/test_captures")
        .with_enabled(false)
        .with_max_queue_size(50);

    let (state, _disk_writer) = AppState::with_disk_writer_config(config);

    // Should use custom config
    assert!(!state.disk_writer.is_enabled());
}
