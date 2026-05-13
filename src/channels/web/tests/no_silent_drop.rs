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
        .subscribe_raw(Some("test-user".to_string()), false)
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
        .subscribe_raw(Some("other-user".to_string()), false)
        .expect("subscribe should succeed");

    // Register the gateway channel with the channel manager
    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: None,
        user_id: "owner-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: Some("other-user".to_string()),
        response: Some("mission result".to_string()),
        is_error: false,
        gate: None,
    };

    let owner_thread_id = notif.thread_id.to_string();

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
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
        .subscribe_raw(Some("test-user".to_string()), false)
        .expect("subscribe should succeed");

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: None,
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None, // owner IS the recipient
        response: Some("mission result".to_string()),
        is_error: false,
        gate: None,
    };

    // With no parent_thread_id, the SSE thread_id must match the assistant
    // conversation row the V1 DB write lands in — using `notif.thread_id`
    // (the mission's internal execution thread) was the source of frontend
    // bleed where cron-fired mission outputs rendered in unrelated chats.
    let expected_thread_id = store
        .get_or_create_assistant_conversation(&notif.user_id, "gateway")
        .await
        .expect("assistant conv lookup")
        .to_string();

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
    .await;

    // The owner should receive two events:
    // 1. From GatewayChannel::broadcast() (channel path)
    // 2. From direct SSE broadcast_for_user (SSE path)
    // Both should carry the assistant conversation thread_id (consistent
    // with the persisted V1 conversation row).
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_eq!(
        thread_id, expected_thread_id,
        "same-user broadcast with no parent must carry the assistant conv \
         thread_id, not the mission's internal execution thread"
    );
    assert_ne!(
        thread_id,
        notif.thread_id.to_string(),
        "mission's internal execution thread_id must NOT be exposed to SSE"
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
        .subscribe_raw(Some("test-user".to_string()), false)
        .expect("subscribe should succeed");

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: None,
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        // Explicitly set to same user — guard must still attach thread_id
        notify_user: Some("test-user".to_string()),
        response: Some("mission result".to_string()),
        is_error: false,
        gate: None,
    };

    let expected_thread_id = store
        .get_or_create_assistant_conversation(&notif.user_id, "gateway")
        .await
        .expect("assistant conv lookup")
        .to_string();

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
    .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_eq!(
        thread_id, expected_thread_id,
        "explicit notify_user == user_id must attach the assistant conv \
         thread_id (consistent with the V1 DB row), not the mission's \
         internal execution thread"
    );
}

/// Regression: when `parent_thread_id` is set on the notification, the SSE
/// `Response` event MUST carry the parent (originating conversation) thread,
/// not the mission's internal execution thread. Otherwise mission output
/// gets persisted under a thread the user can't navigate to and is invisible
/// from the conversation where they fired the mission.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_routes_to_parent_thread_when_set() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_parent_thread_routing.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    let mut stream = sse
        .subscribe_raw(Some("test-user".to_string()), false)
        .expect("subscribe should succeed");

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let parent = ironclaw_engine::ThreadId(uuid::Uuid::new_v4());
    let execution_thread = ironclaw_engine::ThreadId(uuid::Uuid::new_v4());

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "test-mission".to_string(),
        thread_id: execution_thread,
        parent_thread_id: Some(parent),
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("mission result".to_string()),
        is_error: false,
        gate: None,
    };

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
    .await;

    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");

    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };

    assert_eq!(
        thread_id,
        parent.to_string(),
        "mission output must route to the originating conversation thread, \
         not to the mission's internal execution thread"
    );
    assert_ne!(
        thread_id,
        execution_thread.to_string(),
        "execution thread must NOT be exposed when a parent thread is set"
    );
}

/// Regression: when `parent_thread_id` is set, the v1 `add_conversation_message`
/// write MUST go to the parent conversation's row — NOT to the user's
/// assistant conversation. Otherwise the chat history endpoint
/// (`list_conversation_messages_paginated`) returns the mission output for
/// the assistant conversation regardless of which chat thread the user opens,
/// causing every viewer of the assistant thread to see every user mission's
/// output. This is the persistent layer of the bug — SSE filtering alone
/// doesn't fix it because the data is on disk.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_v1_history_lands_in_parent_thread_not_assistant_conv() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_v1_parent_routing.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    // Real chat threads always have a v1 conversations row before any mission
    // can target them. Mirror that here — `add_conversation_message` has a FK
    // on `conversations(id)` and would otherwise refuse the write.
    let parent_uuid = store
        .create_conversation("gateway", "test-user", None)
        .await
        .expect("parent v1 conversation should be creatable");
    let parent = ironclaw_engine::ThreadId(parent_uuid);
    let execution_thread = ironclaw_engine::ThreadId(uuid::Uuid::new_v4());

    // Resolve the user's assistant conversation eagerly so we can assert the
    // write did NOT land there.
    let assistant_conv = store
        .get_or_create_assistant_conversation("test-user", "gateway")
        .await
        .expect("assistant conv should be creatable");

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "v1-routing-test".to_string(),
        thread_id: execution_thread,
        parent_thread_id: Some(parent),
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("the mission result text".to_string()),
        is_error: false,
        gate: None,
    };

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
    .await;

    let parent_messages = store
        .list_conversation_messages_paginated(parent.0, None, 50)
        .await
        .expect("listing parent thread messages should succeed")
        .0;
    let parent_has_mission_output = parent_messages
        .iter()
        .any(|m| m.role == "assistant" && m.content.contains("the mission result text"));
    assert!(
        parent_has_mission_output,
        "v1 chat history of parent thread must contain the mission output \
         (mission output: {parent_messages:?})"
    );

    let assistant_messages = store
        .list_conversation_messages_paginated(assistant_conv, None, 50)
        .await
        .expect("listing assistant conv messages should succeed")
        .0;
    let assistant_has_mission_output = assistant_messages
        .iter()
        .any(|m| m.role == "assistant" && m.content.contains("the mission result text"));
    assert!(
        !assistant_has_mission_output,
        "v1 assistant conversation must NOT receive the mission output when a \
         parent thread is set — that's the bug fix (assistant messages: {assistant_messages:?})"
    );
}

/// Backward-compat regression: missions WITHOUT a parent (cron / learning /
/// API-imported) still fall back to the user's assistant conversation so
/// their output is visible somewhere. This locks in the parent-aware fix
/// without breaking the legacy path.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_v1_history_falls_back_to_assistant_when_no_parent() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_v1_fallback.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);

    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    let assistant_conv = store
        .get_or_create_assistant_conversation("test-user", "gateway")
        .await
        .expect("assistant conv should be creatable");

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "cron-mission".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: None,
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("cron output".to_string()),
        is_error: false,
        gate: None,
    };

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        None,
        None,
        None,
        None,
    )
    .await;

    let assistant_messages = store
        .list_conversation_messages_paginated(assistant_conv, None, 50)
        .await
        .expect("listing assistant conv messages should succeed")
        .0;
    let assistant_has_cron = assistant_messages
        .iter()
        .any(|m| m.role == "assistant" && m.content.contains("cron output"));
    assert!(
        assistant_has_cron,
        "missions with no parent_thread_id must still fall back to the assistant \
         conversation so cron/learning output stays visible"
    );
}
