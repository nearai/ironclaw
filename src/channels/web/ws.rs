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
        } => {
            let mut incoming = IncomingMessage::new("gateway", user_id, &content);
            if let Some(ref tz) = timezone {
                incoming = incoming.with_timezone(tz);
            }
            if let Some(ref tid) = thread_id {
                incoming = incoming.with_thread(tid);
            }

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
                            state.sse.broadcast_for_user(
                                user_id,
                                crate::channels::web::types::AppEvent::AuthRequired {
                                    extension_name: extension_name.clone(),
                                    instructions: Some(result.message),
                                    auth_url: None,
                                    setup_url: None,
                                    thread_id: None,
                                },
                            );
                        } else {
                            crate::channels::web::server::clear_auth_mode(state, user_id).await;
                            state.sse.broadcast_for_user(
                                user_id,
                                crate::channels::web::types::AppEvent::AuthCompleted {
                                    extension_name,
                                    success: true,
                                    message: result.message,
                                    thread_id: None,
                                },
                            );
                        }
                    }
                    Err(e) => {
                        let msg = format!("Auth failed: {}", e);
                        if matches!(e, crate::extensions::ExtensionError::ValidationFailed(_)) {
                            state.sse.broadcast_for_user(
                                user_id,
                                crate::channels::web::types::AppEvent::AuthRequired {
                                    extension_name: extension_name.clone(),
                                    instructions: Some(msg.clone()),
                                    auth_url: None,
                                    setup_url: None,
                                    thread_id: None,
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
