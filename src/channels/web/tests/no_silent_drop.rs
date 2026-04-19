//! Regression tests: the gateway channel must never silently drop messages.
//!
//! Previously, `respond()` and `broadcast()` returned `Ok(())` when thread_id
//! was missing, making callers believe the message was delivered when it wasn't.
//! These tests ensure that missing routing info produces an explicit error.

use crate::channels::channel::{Channel, IncomingMessage, OutgoingResponse};
use crate::channels::web::GatewayChannel;
use crate::channels::web::sse::DEFAULT_BROADCAST_BUFFER;
use crate::config::GatewayConfig;
use crate::error::ChannelError;

fn test_gateway() -> GatewayChannel {
    GatewayChannel::new(
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("test-token".to_string()),
            max_connections: 100,
            broadcast_buffer: DEFAULT_BROADCAST_BUFFER,
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
        },
        "test-user".to_string(),
    )
}

#[tokio::test]
async fn gateway_respond_without_thread_id_returns_error() {
    let gw = test_gateway();
    let msg = IncomingMessage::new("gateway", "test-user", "hello");
    // msg has no thread_id by default
    assert!(msg.thread_id.is_none());

    let response = OutgoingResponse::text("reply");
    let result = gw.respond(&msg, response).await;

    assert!(
        result.is_err(),
        "respond() must not silently succeed without thread_id"
    );
    assert!(
        matches!(result, Err(ChannelError::MissingRoutingTarget { .. })),
        "Expected MissingRoutingTarget, got: {:?}",
        result
    );
}

#[tokio::test]
async fn gateway_respond_with_thread_id_succeeds() {
    let gw = test_gateway();
    let mut msg = IncomingMessage::new("gateway", "test-user", "hello");
    msg.thread_id = Some(ironclaw_common::ExternalThreadId::from_trusted(
        "thread-123".to_string(),
    ));

    let response = OutgoingResponse::text("reply");
    let result = gw.respond(&msg, response).await;

    assert!(
        result.is_ok(),
        "respond() should succeed with thread_id: {:?}",
        result
    );
}

#[tokio::test]
async fn gateway_broadcast_without_thread_id_and_no_store_returns_error() {
    let gw = test_gateway();
    let response = OutgoingResponse::text("notification");
    // response has no thread_id by default, gateway has no store

    let result = gw.broadcast("test-user", response).await;

    assert!(
        result.is_err(),
        "broadcast() without thread_id and no store should error"
    );
    assert!(
        matches!(result, Err(ChannelError::MissingRoutingTarget { .. })),
        "Expected MissingRoutingTarget, got: {:?}",
        result
    );
}

/// When a store IS available, broadcast() without thread_id falls back to
/// the user's assistant conversation instead of erroring.
/// Verifies the SSE event carries the correct resolved thread_id and that the
/// DB conversation row exists.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn gateway_broadcast_without_thread_id_falls_back_to_assistant_thread() {
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_broadcast_fallback.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());

    // Subscribe to SSE before broadcasting so we capture the event
    let mut stream = gw
        .state
        .sse
        .subscribe_raw(Some("test-user".to_string()))
        .expect("subscribe should succeed");

    let response = OutgoingResponse::text("mission notification");
    let result = gw.broadcast("test-user", response).await;

    assert!(
        result.is_ok(),
        "broadcast() without thread_id should fall back to assistant thread: {:?}",
        result
    );

    // Verify SSE event has the correct thread_id
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    // The thread_id should be a valid UUID from get_or_create_assistant_conversation
    let resolved_uuid =
        uuid::Uuid::parse_str(&thread_id).expect("thread_id should be a valid UUID");

    // Verify the DB conversation row exists
    let db_conv_id = store
        .get_or_create_assistant_conversation("test-user", "gateway")
        .await
        .expect("assistant conversation should exist");
    assert_eq!(
        resolved_uuid, db_conv_id,
        "SSE thread_id should match the DB assistant conversation UUID"
    );
}

#[tokio::test]
async fn gateway_broadcast_with_thread_id_succeeds() {
    let gw = test_gateway();
    let response = OutgoingResponse::text("notification").in_thread("thread-456".to_string());

    let result = gw.broadcast("test-user", response).await;

    assert!(
        result.is_ok(),
        "broadcast() should succeed with thread_id: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Cross-user thread_id guard tests
//
// Regression coverage for the security guard in handle_mission_notification
// that prevents leaking the mission owner's thread_id to a different
// recipient (notify_user). Tests exercise the caller, not just broadcast().
// ---------------------------------------------------------------------------

/// When notify_user routes to a different user, the owner's mission
/// thread_id must NOT be attached to the channel broadcast.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_cross_user_does_not_leak_owner_thread_id() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use std::sync::Arc;

    // Set up DB for broadcast fallback
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_cross_user.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    // Subscribe as the recipient ("other-user") to capture what they receive
    let mut stream = sse
        .subscribe_raw(Some("other-user".to_string()))
        .expect("subscribe should succeed");

    // Register the gateway channel with the channel manager
    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        user_id: "owner-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: Some("other-user".to_string()),
        response: Some("mission result".to_string()),
        is_error: false,
    };

    let owner_thread_id = notif.thread_id.to_string();

    crate::bridge::handle_mission_notification(&notif, &channels, Some(&sse), Some(&store), None)
        .await;

    // The recipient should get an event with a thread_id that is NOT the
    // owner's mission thread — it should be their own assistant thread.
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_ne!(
        thread_id, owner_thread_id,
        "cross-user broadcast must NOT carry the owner's thread_id"
    );

    // The thread_id should be the recipient's own assistant conversation
    let recipient_conv = store
        .get_or_create_assistant_conversation("other-user", "gateway")
        .await
        .expect("recipient assistant conversation should exist");
    assert_eq!(
        thread_id,
        recipient_conv.to_string(),
        "cross-user broadcast should resolve to recipient's assistant thread"
    );
}

/// When notify_user is None (owner IS the recipient), the mission
/// thread_id SHOULD be attached to the broadcast.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_same_user_attaches_owner_thread_id() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_same_user.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    // Subscribe as the owner to capture channel broadcast events
    let mut stream = sse
        .subscribe_raw(Some("test-user".to_string()))
        .expect("subscribe should succeed");

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None, // owner IS the recipient
        response: Some("mission result".to_string()),
        is_error: false,
    };

    let owner_thread_id = notif.thread_id.to_string();

    crate::bridge::handle_mission_notification(&notif, &channels, Some(&sse), Some(&store), None)
        .await;

    // The owner should receive two events:
    // 1. From GatewayChannel::broadcast() (channel path)
    // 2. From direct SSE broadcast_for_user (SSE path)
    // Both should carry the owner's thread_id.
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_eq!(
        thread_id, owner_thread_id,
        "same-user broadcast should carry the owner's mission thread_id"
    );
}

/// Edge case: notify_user is explicitly set but equals user_id.
/// The guard compares broadcast_user == notif.user_id, which should still
/// match. If someone refactors to check notify_user.is_none() instead,
/// this test catches the regression.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_explicit_same_user_attaches_owner_thread_id() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_explicit_same_user.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    let mut stream = sse
        .subscribe_raw(Some("test-user".to_string()))
        .expect("subscribe should succeed");

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        // Explicitly set to same user — guard must still attach thread_id
        notify_user: Some("test-user".to_string()),
        response: Some("mission result".to_string()),
        is_error: false,
    };

    let owner_thread_id = notif.thread_id.to_string();

    crate::bridge::handle_mission_notification(&notif, &channels, Some(&sse), Some(&store), None)
        .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_eq!(
        thread_id, owner_thread_id,
        "explicit notify_user == user_id should still attach the owner's thread_id"
    );
}
