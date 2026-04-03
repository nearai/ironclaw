//! WebSocket handler for bidirectional client communication.
//!
//! Provides the same event stream as SSE but also accepts incoming messages
//! (chat, approvals) over a single persistent connection.
//!
//! ```text
//! Client ──── WS frame: {"type":"message","content":"hello"} ──► Agent Loop
//!        ◄─── WS frame: {"type":"event","event_type":"response","data":{...}} ── Broadcast
//!        ──── WS frame: {"type":"ping"} ──────────────────────────────────────►
//!        ◄─── WS frame: {"type":"pong"} ──────────────────────────────────────
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::submission::Submission;
use crate::channels::IncomingMessage;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{WsClientMessage, WsServerMessage};

/// Tracks active WebSocket connections.
pub struct WsConnectionTracker {
    count: AtomicU64,
}

impl WsConnectionTracker {
    pub fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
        }
    }

    pub fn connection_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    fn increment(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement(&self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Default for WsConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle an upgraded WebSocket connection.
///
/// Spawns two tasks:
/// - **sender**: forwards broadcast events to the WebSocket client
/// - **receiver**: reads client frames and routes them to the agent
///
/// When either task ends (client disconnect or broadcast closed), both are
/// cleaned up.
pub async fn handle_ws_connection(
    socket: WebSocket,
    state: Arc<GatewayState>,
    user: crate::channels::web::auth::UserIdentity,
    workspace_id: Option<String>,
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Track connection
    if let Some(ref tracker) = state.ws_tracker {
        tracker.increment();
    }
    let tracker_for_drop = state.ws_tracker.clone();

    // Subscribe to broadcast events (same source as SSE), scoped to this user.
    // Reject if we've hit the connection limit.
    let Some(raw_stream) = state
        .sse
        .subscribe_raw_scoped(Some(user.user_id.clone()), workspace_id.clone())
    else {
        tracing::warn!("WebSocket rejected: too many connections");
        // Decrement the WS tracker we already incremented above.
        if let Some(ref tracker) = tracker_for_drop {
            tracker.decrement();
        }
        return;
    };
    let mut event_stream = Box::pin(raw_stream);

    // Channel for the sender task to receive messages from both
    // the broadcast stream and any direct sends (like Pong)
    let (direct_tx, mut direct_rx) = mpsc::channel::<WsServerMessage>(64);

    // Sender task: forward broadcast events + direct messages to WS client
    let sender_handle = tokio::spawn(async move {
        loop {
            let msg = tokio::select! {
                event = event_stream.next() => {
                    match event {
                        Some(app_event) => WsServerMessage::from_app_event(&app_event),
                        None => break, // Broadcast channel closed
                    }
                }
                direct = direct_rx.recv() => {
                    match direct {
                        Some(msg) => msg,
                        None => break, // Direct channel closed
                    }
                }
            };

            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(_) => continue,
            };

            if ws_sink.send(Message::Text(json.into())).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    // Receiver task: read client frames and route to agent
    let user_id = user.user_id;
    while let Some(Ok(frame)) = ws_stream.next().await {
        match frame {
            Message::Text(text) => {
                let parsed: Result<WsClientMessage, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(client_msg) => {
                        handle_client_message(
                            client_msg,
                            &state,
                            &user_id,
                            workspace_id.as_deref(),
                            &direct_tx,
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = direct_tx
                            .send(WsServerMessage::Error {
                                message: format!("Invalid message: {}", e),
                            })
                            .await;
                    }
                }
            }
            Message::Close(_) => break,
            // Ignore binary, ping/pong (axum handles protocol-level pings)
            _ => {}
        }
    }

    // Clean up: abort sender, decrement counter
    sender_handle.abort();
    if let Some(ref tracker) = tracker_for_drop {
        tracker.decrement();
    }
}

/// Route a parsed client message to the appropriate handler.
async fn handle_client_message(
    msg: WsClientMessage,
    state: &GatewayState,
    user_id: &str,
    workspace_id: Option<&str>,
    direct_tx: &mpsc::Sender<WsServerMessage>,
) {
    match msg {
        WsClientMessage::Message {
            content,
            thread_id,
            timezone,
            images,
        } => {
            let mut incoming = IncomingMessage::new("gateway", user_id, &content);
            if let Some(workspace_id) = workspace_id {
                incoming.workspace_id = Some(workspace_id.to_string());
            }
            if let Some(ref tz) = timezone {
                incoming = incoming.with_timezone(tz);
            }
            if let Some(ref tid) = thread_id {
                incoming = incoming.with_thread(tid);
            }
            let mut metadata = serde_json::json!({ "user_id": user_id });
            if let Some(ref tid) = thread_id {
                metadata["thread_id"] = serde_json::json!(tid);
            }
            if let Some(workspace_id) = workspace_id {
                metadata["workspace_id"] = serde_json::json!(workspace_id);
            }
            incoming = incoming.with_metadata(metadata);

            // Convert uploaded images to IncomingAttachments
            if !images.is_empty() {
                let attachments = crate::channels::web::server::images_to_attachments(&images);
                incoming = incoming.with_attachments(attachments);
            }

            // Clone sender to avoid holding RwLock read guard across send().await
            let tx = {
                let tx_guard = state.msg_tx.read().await;
                tx_guard.as_ref().cloned()
            };
            if let Some(tx) = tx {
                if tx.send(incoming).await.is_err() {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: "Channel closed".to_string(),
                        })
                        .await;
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::Error {
                        message: "Channel not started".to_string(),
                    })
                    .await;
            }
        }
        WsClientMessage::Approval {
            request_id,
            action,
            thread_id,
        } => {
            let (approved, always) = match action.as_str() {
                "approve" => (true, false),
                "always" => (true, true),
                "deny" => (false, false),
                other => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: format!("Unknown approval action: {}", other),
                        })
                        .await;
                    return;
                }
            };

            let request_uuid = match Uuid::parse_str(&request_id) {
                Ok(id) => id,
                Err(_) => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: "Invalid request_id (expected UUID)".to_string(),
                        })
                        .await;
                    return;
                }
            };

            let approval = Submission::ExecApproval {
                request_id: request_uuid,
                approved,
                always,
            };
            let content = match serde_json::to_string(&approval) {
                Ok(c) => c,
                Err(e) => {
                    let _ = direct_tx
                        .send(WsServerMessage::Error {
                            message: format!("Failed to serialize approval: {}", e),
                        })
                        .await;
                    return;
                }
            };

            let mut msg = IncomingMessage::new("gateway", user_id, content);
            if let Some(ref tid) = thread_id {
                msg = msg.with_thread(tid);
            }
            if let Some(workspace_id) = workspace_id {
                msg.workspace_id = Some(workspace_id.to_string());
            }
            let mut metadata = serde_json::json!({ "user_id": user_id });
            if let Some(ref tid) = thread_id {
                metadata["thread_id"] = serde_json::json!(tid);
            }
            if let Some(workspace_id) = workspace_id {
                metadata["workspace_id"] = serde_json::json!(workspace_id);
            }
            msg = msg.with_metadata(metadata);
            // Clone sender to avoid holding RwLock read guard across send().await
            let tx = {
                let tx_guard = state.msg_tx.read().await;
                tx_guard.as_ref().cloned()
            };
            if let Some(tx) = tx {
                let _ = tx.send(msg).await;
            }
        }
        WsClientMessage::AuthToken {
            extension_name,
            token,
        } => {
            if let Some(ref ext_mgr) = state.extension_manager {
                match ext_mgr
                    .configure_token(&extension_name, &token, user_id)
                    .await
                {
                    Ok(result) => {
                        if result.verification.is_some() {
                            state.sse.broadcast_for_user_in_workspace(
                                user_id,
                                workspace_id,
                                crate::channels::web::types::AppEvent::AuthRequired {
                                    extension_name: extension_name.clone(),
                                    instructions: Some(result.message),
                                    auth_url: None,
                                    setup_url: None,
                                },
                            );
                        } else {
                            crate::channels::web::server::clear_auth_mode(state, user_id).await;
                            state.sse.broadcast_for_user_in_workspace(
                                user_id,
                                workspace_id,
                                crate::channels::web::types::AppEvent::AuthCompleted {
                                    extension_name,
                                    success: true,
                                    message: result.message,
                                },
                            );
                        }
                    }
                    Err(e) => {
                        let msg = format!("Auth failed: {}", e);
                        if matches!(e, crate::extensions::ExtensionError::ValidationFailed(_)) {
                            state.sse.broadcast_for_user_in_workspace(
                                user_id,
                                workspace_id,
                                crate::channels::web::types::AppEvent::AuthRequired {
                                    extension_name: extension_name.clone(),
                                    instructions: Some(msg.clone()),
                                    auth_url: None,
                                    setup_url: None,
                                },
                            );
                        }
                        let _ = direct_tx
                            .send(WsServerMessage::Error { message: msg })
                            .await;
                    }
                }
            } else {
                let _ = direct_tx
                    .send(WsServerMessage::Error {
                        message: "Extension manager not available".to_string(),
                    })
                    .await;
            }
        }
        WsClientMessage::AuthCancel { .. } => {
            crate::channels::web::server::clear_auth_mode(state, user_id).await;
        }
        WsClientMessage::Ping => {
            let _ = direct_tx.send(WsServerMessage::Pong).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    #[test]
    fn test_ws_connection_tracker() {
        let tracker = WsConnectionTracker::new();
        assert_eq!(tracker.connection_count(), 0);

        tracker.increment();
        assert_eq!(tracker.connection_count(), 1);

        tracker.increment();
        assert_eq!(tracker.connection_count(), 2);

        tracker.decrement();
        assert_eq!(tracker.connection_count(), 1);

        tracker.decrement();
        assert_eq!(tracker.connection_count(), 0);
    }

    #[test]
    fn test_ws_connection_tracker_default() {
        let tracker = WsConnectionTracker::default();
        assert_eq!(tracker.connection_count(), 0);
    }

    #[tokio::test]
    async fn test_handle_client_message_ping() {
        // Ping should produce a Pong on the direct channel
        let (direct_tx, mut direct_rx) = mpsc::channel(16);
        let state = make_test_state(None).await;

        handle_client_message(WsClientMessage::Ping, &state, "user1", None, &direct_tx).await;

        let response = direct_rx.recv().await.unwrap();
        assert!(matches!(response, WsServerMessage::Pong));
    }

    #[tokio::test]
    async fn test_handle_client_message_sends_to_agent() {
        // A Message should be forwarded to the agent's msg_tx
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Message {
                content: "hello agent".to_string(),
                thread_id: Some("t1".to_string()),
                timezone: None,
                images: Vec::new(),
            },
            &state,
            "user1",
            None,
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        assert_eq!(incoming.content, "hello agent");
        assert_eq!(incoming.thread_id.as_deref(), Some("t1"));
        assert_eq!(incoming.channel, "gateway");
        assert_eq!(incoming.user_id, "user1");
    }

    #[tokio::test]
    async fn test_handle_client_message_no_channel() {
        // When msg_tx is None, should send an error back
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Message {
                content: "hello".to_string(),
                thread_id: None,
                timezone: None,
                images: Vec::new(),
            },
            &state,
            "user1",
            None,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("not started"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_client_approval_approve() {
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);

        let request_id = Uuid::new_v4();
        handle_client_message(
            WsClientMessage::Approval {
                request_id: request_id.to_string(),
                action: "approve".to_string(),
                thread_id: Some("thread-42".to_string()),
            },
            &state,
            "user1",
            None,
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        // The content should be a serialized ExecApproval
        assert!(incoming.content.contains("ExecApproval"));
        // Thread should be forwarded onto the IncomingMessage.
        assert_eq!(incoming.thread_id.as_deref(), Some("thread-42"));
    }

    #[tokio::test]
    async fn test_handle_client_approval_invalid_action() {
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Approval {
                request_id: Uuid::new_v4().to_string(),
                action: "maybe".to_string(),
                thread_id: None,
            },
            &state,
            "user1",
            None,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("Unknown approval action"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_client_approval_invalid_uuid() {
        let state = make_test_state(None).await;
        let (direct_tx, mut direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Approval {
                request_id: "not-a-uuid".to_string(),
                action: "approve".to_string(),
                thread_id: None,
            },
            &state,
            "user1",
            None,
            &direct_tx,
        )
        .await;

        let response = direct_rx.recv().await.unwrap();
        match response {
            WsServerMessage::Error { message } => {
                assert!(message.contains("Invalid request_id"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_client_auth_token_broadcasts_workspace_scoped_auth_completed() {
        use tokio::time::{Duration, timeout};

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        let channel_name = "scoped-ws-channel";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");
        let caps = serde_json::json!({
            "type": "channel",
            "name": channel_name,
            "setup": {
                "required_secrets": [
                    {"name": "BOT_TOKEN", "prompt": "Enter bot token"}
                ]
            }
        });
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.capabilities.json")),
            serde_json::to_string(&caps).expect("serialize caps"),
        )
        .expect("write capabilities");

        let state = make_test_state_with_extension_manager(None, Some(ext_mgr)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);
        let mut receiver = state.sse.sender().subscribe();

        handle_client_message(
            WsClientMessage::AuthToken {
                extension_name: channel_name.to_string(),
                token: "secret".to_string(),
            },
            &state,
            "user1",
            Some("workspace-123"),
            &direct_tx,
        )
        .await;

        let scoped = timeout(Duration::from_millis(250), receiver.recv())
            .await
            .expect("workspace-scoped auth event")
            .expect("broadcast event");
        assert_eq!(scoped.user_id.as_deref(), Some("user1"));
        assert_eq!(scoped.workspace_id.as_deref(), Some("workspace-123"));
        assert!(matches!(
            scoped.event,
            crate::channels::web::types::AppEvent::AuthCompleted { .. }
        ));
    }

    /// Helper to create a GatewayState for testing.
    async fn make_test_state(msg_tx: Option<mpsc::Sender<IncomingMessage>>) -> GatewayState {
        make_test_state_with_extension_manager(msg_tx, None).await
    }

    async fn make_test_state_with_extension_manager(
        msg_tx: Option<mpsc::Sender<IncomingMessage>>,
        extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
    ) -> GatewayState {
        use crate::channels::web::sse::SseManager;

        GatewayState {
            msg_tx: tokio::sync::RwLock::new(msg_tx),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: crate::channels::web::server::PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: crate::channels::web::server::RateLimiter::new(10, 60),
            webhook_rate_limiter: crate::channels::web::server::RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: crate::channels::web::server::ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth: None,
        }
    }

    fn test_secrets_store() -> Arc<dyn crate::secrets::SecretsStore + Send + Sync> {
        Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                "test-key-at-least-32-chars-long!!".to_string(),
            ))
            .expect("crypto"),
        )))
    }

    fn test_ext_mgr(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
    ) -> (
        Arc<crate::extensions::ExtensionManager>,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let tool_registry = Arc::new(crate::tools::ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let ext_mgr = Arc::new(crate::extensions::ExtensionManager::new(
            mcp_sm,
            mcp_pm,
            secrets,
            tool_registry,
            None,
            None,
            wasm_tools_dir.path().to_path_buf(),
            wasm_channels_dir.path().to_path_buf(),
            None,
            "test".to_string(),
            None,
            vec![],
        ));
        (ext_mgr, wasm_tools_dir, wasm_channels_dir)
    }
}
