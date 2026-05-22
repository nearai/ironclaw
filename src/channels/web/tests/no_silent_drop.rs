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

    // Real chat threads always have a v1 `conversations` row before a
    // mission can target them — the chat ingress writes it. Mirror that
    // here so the sink's ownership/metadata check resolves to "verified".
    // Without this row, the sink correctly treats the parent as missing
    // and re-routes to the assistant conversation (the consistency fix).
    let parent_uuid = store
        .create_conversation("gateway", "test-user", None)
        .await
        .expect("parent v1 conversation should be creatable");
    let parent = ironclaw_engine::ThreadId(parent_uuid);
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

/// IDOR regression at the mission-notification sink. Defense-in-depth pair to
/// the chat-ingress check in
/// `src/channels/web/features/chat/mod.rs::chat_send_handler`: even when an
/// attacker-controlled `parent_thread_id` slips past the entry — e.g. a future
/// channel implementation forgets the ownership gate, or a v1-only path is
/// re-added — the mission notification handler MUST NOT write the mission
/// output to another user's `conversation_messages` row. The v1
/// `add_conversation_message` SQL only enforces the conversation FK, not
/// ownership, so without this check the FK is satisfied and the cross-user
/// write succeeds.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_idor_does_not_write_to_other_users_conversation() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_idor_parent.db");
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

    // Victim owns a v1 conversation row.
    let victim_thread = store
        .create_conversation("gateway", "victim", None)
        .await
        .expect("victim's v1 conversation");

    // Attacker has their own assistant conversation row that the fallback
    // path can route to.
    let attacker_assistant = store
        .get_or_create_assistant_conversation("attacker", "gateway")
        .await
        .expect("attacker's assistant conv");

    // The mission was created by the attacker, but the `parent_thread_id` on
    // the notification points at the victim's conversation UUID — the exact
    // shape an upstream ownership gap would produce.
    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "idor-attempt".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: Some(ironclaw_engine::ThreadId(victim_thread)),
        user_id: "attacker".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("attacker-controlled mission output".to_string()),
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

    // The victim's conversation row must NOT have received the write. This is
    // the load-bearing assertion: without the ownership check at the sink, the
    // attacker's mission output would be persisted under victim's
    // `conversation_id` and surface in victim's `/api/chat/history` response.
    let victim_messages = store
        .list_conversation_messages_paginated(victim_thread, None, 50)
        .await
        .expect("listing victim messages")
        .0;
    assert!(
        victim_messages
            .iter()
            .all(|m| !m.content.contains("attacker-controlled mission output")),
        "mission output must NOT leak into another user's conversation \
         (victim messages: {victim_messages:?})",
    );

    // The attacker's own assistant conversation must have received the
    // fallback write so the output is still visible somewhere for the
    // legitimate owner (the attacker). This keeps the user-facing semantics
    // intact for non-attack cases (a deleted v1 row, a non-`gateway` parent
    // channel) where ownership intentionally falls through to the assistant.
    let attacker_messages = store
        .list_conversation_messages_paginated(attacker_assistant, None, 50)
        .await
        .expect("listing attacker assistant messages")
        .0;
    assert!(
        attacker_messages
            .iter()
            .any(|m| m.role == "assistant"
                && m.content.contains("attacker-controlled mission output")),
        "ownership rejection must redirect the v1 write to the mission \
         user's own assistant conversation (attacker assistant: {attacker_messages:?})",
    );
}

/// v2 conversation-key scoping regression. When a verified parent thread is
/// present, `handle_mission_notification` must record the external-agent entry
/// against the scoped engine v2 conversation `<channel>:<parent>` — matching
/// the foreground key at `handle_with_engine_inner` ~line 4373 — so the next
/// chat turn in the same thread can see the mission output in its v2 history.
/// Without scoping, the entry lands in the unscoped `<channel>` conversation
/// and `build_history_from_entries` for the follow-up turn never includes it.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_v2_entry_lands_in_scoped_conversation() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use ironclaw_engine::{
        CapabilityRegistry, ConversationManager, LeaseManager, LlmBackend, LlmCallConfig,
        LlmOutput, LlmResponse, PolicyEngine, ThreadManager, TokenUsage,
    };
    use std::sync::Arc;

    // Minimal LLM/effects stubs — the test never spins up an execution loop.
    struct NoopLlm;
    #[async_trait::async_trait]
    impl LlmBackend for NoopLlm {
        async fn complete(
            &self,
            _: &[ironclaw_engine::ThreadMessage],
            _: &[ironclaw_engine::ActionDef],
            _: &LlmCallConfig,
        ) -> Result<LlmOutput, ironclaw_engine::EngineError> {
            Ok(LlmOutput {
                response: LlmResponse::Text("noop".into()),
                usage: TokenUsage::default(),
            })
        }
        fn model_name(&self) -> &str {
            "noop"
        }
    }
    struct NoopEffects;
    #[async_trait::async_trait]
    impl ironclaw_engine::EffectExecutor for NoopEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &ironclaw_engine::CapabilityLease,
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<ironclaw_engine::ActionResult, ironclaw_engine::EngineError> {
            unreachable!("test does not drive execution")
        }
        async fn available_actions(
            &self,
            _: &[ironclaw_engine::CapabilityLease],
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<Vec<ironclaw_engine::ActionDef>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn available_capabilities(
            &self,
            _: &[ironclaw_engine::CapabilityLease],
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<Vec<ironclaw_engine::CapabilitySummary>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_v2_scope.db");
    let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
        .await
        .unwrap();
    Database::run_migrations(&backend).await.unwrap();
    let store: Arc<dyn Database> = Arc::new(backend);

    // A verified parent: a v1 conversation row owned by the mission user.
    let parent_uuid = store
        .create_conversation("gateway", "owner-user", None)
        .await
        .expect("parent v1 conversation");
    let parent = ironclaw_engine::ThreadId(parent_uuid);

    let gw = test_gateway().with_store(store.clone());
    let sse = Arc::clone(&gw.state.sse);
    let mgr = ChannelManager::new();
    mgr.add(Box::new(gw)).await;
    let channels = Arc::new(mgr);

    // Real v2 ConversationManager — backed by the host's `HybridStore`. The
    // engine Store and the v1 `Database` are decoupled by design; the v2
    // path uses `HybridStore` (which can be workspace-less for tests) and
    // the v1 path uses the libsql `store` above.
    let engine_store: Arc<dyn ironclaw_engine::Store> =
        Arc::new(crate::bridge::store_adapter::HybridStore::new(None));
    let thread_manager = Arc::new(ThreadManager::new(
        Arc::new(NoopLlm),
        Arc::new(NoopEffects),
        Arc::clone(&engine_store),
        Arc::new(CapabilityRegistry::new()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    ));
    let conv_mgr = ConversationManager::new(thread_manager, Arc::clone(&engine_store));

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "v2-scope-test".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: Some(parent),
        user_id: "owner-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("scoped mission output".to_string()),
        is_error: false,
        gate: None,
    };

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        Some(&conv_mgr),
        None,
        None,
        None,
    )
    .await;

    // The scoped conversation MUST exist and carry the mission entry. Calling
    // `get_or_create_conversation` with the scoped key is the exact pattern
    // the foreground turn uses; matching keys means the v2 history for the
    // user's next message will include this entry.
    let scoped_key = format!("gateway:{}", parent.0);
    let scoped_id = conv_mgr
        .get_or_create_conversation(&scoped_key, "owner-user")
        .await
        .expect("scoped conversation lookup");
    let scoped_conv = conv_mgr
        .get_conversation(scoped_id)
        .await
        .expect("scoped conversation snapshot");
    assert!(
        scoped_conv
            .entries
            .iter()
            .any(|e| e.content.contains("scoped mission output")),
        "scoped v2 conversation must contain the mission entry under \
         `<channel>:<parent>` so follow-up turns see it (entries: {:?})",
        scoped_conv.entries,
    );

    // Defense-in-depth: the unscoped `gateway` conversation must NOT carry
    // the entry. Two separate conversations means the v2 follow-up turn
    // reading from the scoped key would miss the mission output entirely if
    // we wrote to the unscoped key — the exact correctness bug Fix 3
    // addresses.
    let unscoped_id = conv_mgr
        .get_or_create_conversation("gateway", "owner-user")
        .await
        .expect("unscoped conversation lookup");
    assert_ne!(
        scoped_id, unscoped_id,
        "scoped and unscoped keys must resolve to different conversations \
         — if they're equal, the scoping is a no-op and the assertion above \
         is hollow",
    );
    let unscoped_conv = conv_mgr
        .get_conversation(unscoped_id)
        .await
        .expect("unscoped conversation snapshot");
    assert!(
        unscoped_conv
            .entries
            .iter()
            .all(|e| !e.content.contains("scoped mission output")),
        "mission output must not leak into the unscoped v2 conversation \
         (entries: {:?})",
        unscoped_conv.entries,
    );
}

/// Missing-parent-row consistency regression. When `parent_thread_id` is set
/// but has no v1 `conversations` row (deleted thread, never-created row),
/// `handle_mission_notification` must collapse the destination to the user's
/// assistant conversation in a single resolution step. Otherwise the
/// destination splits: SSE/v2 keyed by `parent` (treating the missing row as
/// "verified"), v1 FK-failing and falling back to the assistant conversation.
/// The live SSE event then lands in one thread while refresh/history loads
/// from another, and the v2 follow-up turn reads from `gateway:<parent>` —
/// a conversation that has no mission entry — so the agent denies sending it.
///
/// This test asserts the three destinations (SSE thread_id, v1 persisted row,
/// v2 conversation key) all converge on the assistant conversation when the
/// parent row is missing.
#[cfg(feature = "libsql")]
#[tokio::test]
async fn mission_notification_missing_parent_row_converges_on_assistant_conv() {
    use crate::channels::ChannelManager;
    use crate::db::Database;
    use futures::StreamExt;
    use ironclaw_common::AppEvent;
    use ironclaw_engine::{
        CapabilityRegistry, ConversationManager, LeaseManager, LlmBackend, LlmCallConfig,
        LlmOutput, LlmResponse, PolicyEngine, ThreadManager, TokenUsage,
    };
    use std::sync::Arc;

    struct NoopLlm;
    #[async_trait::async_trait]
    impl LlmBackend for NoopLlm {
        async fn complete(
            &self,
            _: &[ironclaw_engine::ThreadMessage],
            _: &[ironclaw_engine::ActionDef],
            _: &LlmCallConfig,
        ) -> Result<LlmOutput, ironclaw_engine::EngineError> {
            Ok(LlmOutput {
                response: LlmResponse::Text("noop".into()),
                usage: TokenUsage::default(),
            })
        }
        fn model_name(&self) -> &str {
            "noop"
        }
    }
    struct NoopEffects;
    #[async_trait::async_trait]
    impl ironclaw_engine::EffectExecutor for NoopEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &ironclaw_engine::CapabilityLease,
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<ironclaw_engine::ActionResult, ironclaw_engine::EngineError> {
            unreachable!("test does not drive execution")
        }
        async fn available_actions(
            &self,
            _: &[ironclaw_engine::CapabilityLease],
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<Vec<ironclaw_engine::ActionDef>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
        async fn available_capabilities(
            &self,
            _: &[ironclaw_engine::CapabilityLease],
            _: &ironclaw_engine::ThreadExecutionContext,
        ) -> Result<Vec<ironclaw_engine::CapabilitySummary>, ironclaw_engine::EngineError> {
            Ok(vec![])
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_missing_parent_row.db");
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

    let engine_store: Arc<dyn ironclaw_engine::Store> =
        Arc::new(crate::bridge::store_adapter::HybridStore::new(None));
    let thread_manager = Arc::new(ThreadManager::new(
        Arc::new(NoopLlm),
        Arc::new(NoopEffects),
        Arc::clone(&engine_store),
        Arc::new(CapabilityRegistry::new()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    ));
    let conv_mgr = ConversationManager::new(thread_manager, Arc::clone(&engine_store));

    // The mission carries a `parent_thread_id` whose v1 row was never
    // created (or has since been deleted). Deliberately do NOT call
    // `create_conversation` here.
    let missing_parent = ironclaw_engine::ThreadId(uuid::Uuid::new_v4());

    let assistant_conv = store
        .get_or_create_assistant_conversation("test-user", "gateway")
        .await
        .expect("assistant conv should be creatable");

    let notif = ironclaw_engine::MissionNotification {
        mission_id: ironclaw_engine::MissionId(uuid::Uuid::new_v4()),
        mission_name: "missing-parent-test".to_string(),
        thread_id: ironclaw_engine::ThreadId(uuid::Uuid::new_v4()),
        parent_thread_id: Some(missing_parent),
        user_id: "test-user".to_string(),
        notify_channels: vec!["gateway".to_string()],
        notify_user: None,
        response: Some("missing-parent mission output".to_string()),
        is_error: false,
        gate: None,
    };

    crate::bridge::handle_mission_notification(
        &notif,
        &channels,
        Some(&sse),
        Some(&store),
        Some(&conv_mgr),
        None,
        None,
        None,
    )
    .await;

    // (1) SSE thread_id must equal the assistant conversation, not the
    //     missing parent UUID. The split-brain bug surfaced exactly here:
    //     SSE advertised the live event under `missing_parent` while the
    //     v1 row landed elsewhere.
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
        .await
        .expect("should receive SSE event within 1s")
        .expect("stream should not be empty");
    let AppEvent::Response { thread_id, .. } = event else {
        panic!("expected AppEvent::Response, got: {event:?}");
    };
    assert_eq!(
        thread_id,
        assistant_conv.to_string(),
        "SSE thread_id must converge on the assistant conversation when the \
         parent row is missing — not the missing parent UUID"
    );
    assert_ne!(
        thread_id,
        missing_parent.to_string(),
        "SSE thread_id must not advertise a parent UUID with no v1 row \
         (history endpoint would return empty for that thread)"
    );

    // (2) v1 persistence must land in the assistant conversation, NOT in
    //     a (nonexistent) row for the missing parent.
    let assistant_messages = store
        .list_conversation_messages_paginated(assistant_conv, None, 50)
        .await
        .expect("listing assistant conv messages should succeed")
        .0;
    assert!(
        assistant_messages
            .iter()
            .any(|m| m.role == "assistant" && m.content.contains("missing-parent mission output")),
        "v1 persistence must converge on the assistant conversation when the \
         parent row is missing (messages: {assistant_messages:?})"
    );

    // (3) v2 conversation key must use the unscoped `gateway` key, matching
    //     the assistant-conv routing. Scoping by `gateway:<missing_parent>`
    //     here would re-create the split-brain at the v2 follow-up layer:
    //     the next chat turn loads from the unscoped key and would miss the
    //     entry.
    let unscoped_id = conv_mgr
        .get_or_create_conversation("gateway", "test-user")
        .await
        .expect("unscoped conversation lookup");
    let unscoped_conv = conv_mgr
        .get_conversation(unscoped_id)
        .await
        .expect("unscoped conversation snapshot");
    assert!(
        unscoped_conv
            .entries
            .iter()
            .any(|e| e.content.contains("missing-parent mission output")),
        "v2 entry must land in the unscoped `gateway` conversation when the \
         parent row is missing (entries: {:?})",
        unscoped_conv.entries,
    );

    let scoped_id = conv_mgr
        .get_or_create_conversation(&format!("gateway:{}", missing_parent.0), "test-user")
        .await
        .expect("scoped conversation lookup");
    let scoped_conv = conv_mgr
        .get_conversation(scoped_id)
        .await
        .expect("scoped conversation snapshot");
    assert!(
        scoped_conv
            .entries
            .iter()
            .all(|e| !e.content.contains("missing-parent mission output")),
        "v2 entry must NOT land under `gateway:<missing_parent>` — otherwise \
         the follow-up turn loading from the unscoped key would not see it \
         (entries: {:?})",
        scoped_conv.entries,
    );
}
