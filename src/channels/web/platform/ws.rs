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
use crate::channels::web::platform::state::GatewayState;
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
) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Track connection
    if let Some(ref tracker) = state.ws_tracker {
        tracker.increment();
    }
    let tracker_for_drop = state.ws_tracker.clone();

    // Subscribe to broadcast events (same source as SSE), scoped to this user.
    // Reject if we've hit the connection limit.
    let Some(raw_stream) = state.sse.subscribe_raw(Some(user.user_id.clone())) else {
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
                        handle_client_message(client_msg, &state, &user_id, &direct_tx).await;
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
    direct_tx: &mpsc::Sender<WsServerMessage>,
) {
    match msg {
        WsClientMessage::Message {
            content,
            thread_id,
            timezone,
            images,
            attachments,
        } => {
            let mut incoming = crate::channels::web::util::web_incoming_message(
                "gateway",
                user_id,
                &content,
                thread_id.as_deref(),
            );
            if let Some(ref tz) = timezone {
                incoming = incoming.with_timezone(tz);
            }

            // Convert uploaded files + images to IncomingAttachments.
            let incoming_attachments =
                match crate::channels::web::util::inline_attachments_to_incoming(
                    &images,
                    &attachments,
                ) {
                    Ok(incoming) => incoming,
                    Err(message) => {
                        let _ = direct_tx.send(WsServerMessage::Error { message }).await;
                        return;
                    }
                };
            if !incoming_attachments.is_empty() {
                incoming = incoming.with_attachments(incoming_attachments);
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

            let msg = crate::channels::web::util::web_incoming_message(
                "gateway",
                user_id,
                content,
                thread_id.as_deref(),
            );
            // Clone sender to avoid holding RwLock read guard across send().await
            let tx = {
                let tx_guard = state.msg_tx.read().await;
                tx_guard.as_ref().cloned()
            };
            if let Some(tx) = tx {
                let _ = tx.send(msg).await;
            }
        }
        // Temporary legacy WebSocket aliases. Remove together with the
        // `/api/chat/auth-token` and `/api/chat/auth-cancel` shims once the
        // gateway no longer supports v1 thread-level auth mode.
        WsClientMessage::AuthToken {
            extension_name,
            token,
            thread_id,
        } => {
            let req = crate::channels::web::types::AuthTokenRequest {
                extension_name,
                token,
                request_id: None,
                thread_id,
            };
            if let Err((_, message)) =
                crate::channels::web::platform::legacy_auth::handle_legacy_auth_token_submission(
                    state, user_id, req,
                )
                .await
            {
                let _ = direct_tx.send(WsServerMessage::Error { message }).await;
            }
        }
        WsClientMessage::AuthCancel {
            extension_name,
            thread_id,
        } => {
            let req = crate::channels::web::types::AuthCancelRequest {
                extension_name,
                request_id: None,
                thread_id,
            };
            if let Err((_, message)) =
                crate::channels::web::platform::legacy_auth::handle_legacy_auth_cancel(
                    state, user_id, req,
                )
                .await
            {
                let _ = direct_tx.send(WsServerMessage::Error { message }).await;
            }
        }
        WsClientMessage::Ping => {
            let _ = direct_tx.send(WsServerMessage::Pong).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingMessage;

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

        handle_client_message(WsClientMessage::Ping, &state, "user1", &direct_tx).await;

        let response = direct_rx.recv().await.unwrap();
        assert!(matches!(response, WsServerMessage::Pong));
    }

    #[tokio::test]
    async fn test_handle_client_message_sends_to_agent() {
        // A Message should be forwarded to the agent's msg_tx
        let (agent_tx, mut agent_rx) = mpsc::channel::<IncomingMessage>(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Message {
                content: "hello agent".to_string(),
                thread_id: Some("t1".to_string()),
                timezone: None,
                images: Vec::new(),
                attachments: Vec::new(),
            },
            &state,
            "user1",
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        assert_eq!(incoming.content, "hello agent");
        assert_eq!(incoming.thread_id.as_ref().map(|t| t.as_str()), Some("t1"));
        assert_eq!(incoming.channel, "gateway");
        assert_eq!(incoming.user_id, "user1");
        assert_eq!(
            incoming.metadata.get("user_id").and_then(|v| v.as_str()),
            Some("user1")
        );
        assert_eq!(
            incoming.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("t1")
        );
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
                attachments: Vec::new(),
            },
            &state,
            "user1",
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
    async fn test_handle_client_message_forwards_attachments() {
        let (agent_tx, mut agent_rx) = mpsc::channel(16);
        let state = make_test_state(Some(agent_tx)).await;
        let (direct_tx, _direct_rx) = mpsc::channel(16);

        handle_client_message(
            WsClientMessage::Message {
                content: "check attachment".to_string(),
                thread_id: None,
                timezone: None,
                images: Vec::new(),
                attachments: vec![crate::channels::web::types::AttachmentData {
                    mime_type: "text/plain".to_string(),
                    filename: Some("notes.txt".to_string()),
                    data_base64: "aGVsbG8=".to_string(),
                }],
            },
            &state,
            "user1",
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        assert_eq!(incoming.attachments.len(), 1);
        assert_eq!(incoming.attachments[0].mime_type, "text/plain");
        assert_eq!(
            incoming.attachments[0].filename.as_deref(),
            Some("notes.txt")
        );
        assert_eq!(incoming.attachments[0].data, b"hello".to_vec());
    }

    #[tokio::test]
    async fn test_handle_client_approval_approve() {
        let (agent_tx, mut agent_rx) = mpsc::channel::<IncomingMessage>(16);
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
            &direct_tx,
        )
        .await;

        let incoming = agent_rx.recv().await.unwrap();
        // The content should be a serialized ExecApproval
        assert!(incoming.content.contains("ExecApproval"));
        // Thread should be forwarded onto the IncomingMessage.
        assert_eq!(
            incoming.thread_id.as_ref().map(|t| t.as_str()),
            Some("thread-42")
        );
        assert_eq!(
            incoming.metadata.get("user_id").and_then(|v| v.as_str()),
            Some("user1")
        );
        assert_eq!(
            incoming.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("thread-42")
        );
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

    /// Helper to create a GatewayState for testing.
    async fn make_test_state(msg_tx: Option<mpsc::Sender<IncomingMessage>>) -> GatewayState {
        use crate::channels::web::sse::SseManager;

        GatewayState {
            msg_tx: tokio::sync::RwLock::new(msg_tx),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            settings_cache: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: None,
            llm_reload: None,
            llm_session_manager: None,
            config_toml_path: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            chat_rate_limiter: crate::channels::web::platform::state::PerUserRateLimiter::new(
                30, 60,
            ),
            oauth_rate_limiter: crate::channels::web::platform::state::PerUserRateLimiter::new(
                20, 60,
            ),
            webhook_rate_limiter: crate::channels::web::platform::state::RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Arc::new(tokio::sync::RwLock::new(
                crate::channels::web::platform::state::ActiveConfigSnapshot::default(),
            )),
            secrets_store: None,
            db_auth: None,
            pairing_store: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        }
    }
}
