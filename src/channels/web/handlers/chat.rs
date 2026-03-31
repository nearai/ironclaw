//! Chat handlers: SSE events, WebSocket, threads, and shared helpers.
//!
//! NOTE: The primary chat handlers (chat_send_handler, chat_approval_handler,
//! chat_auth_token_handler, chat_auth_cancel_handler, chat_history_handler)
//! live in server.rs where routes are registered. Do NOT add duplicates here.

use std::sync::Arc;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::channels::web::util::{
    build_turns_from_db_messages, tool_error_for_display, truncate_preview,
};
use axum::{
    Json,
    extract::{Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

// ── Shared helpers used by server.rs handlers ──────────────────────────

/// Clear pending auth mode on the active thread.
pub async fn clear_auth_mode(state: &GatewayState, user_id: &str) {
    if let Some(ref sm) = state.session_manager {
        let session = sm.get_or_create_session(user_id).await;
        let mut sess = session.lock().await;
        if let Some(thread_id) = sess.active_thread
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
}

// ── SSE / WebSocket handlers ───────────────────────────────────────────

pub async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state.sse.subscribe(Some(user.user_id)).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Too many connections".to_string(),
    ))
}

pub async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(identity): AuthenticatedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate Origin header to prevent cross-site WebSocket hijacking.
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .and_then(|rest| rest.split(':').next()?.split('/').next())
        .unwrap_or("");

    let is_local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if !is_local {
        return Err((
            StatusCode::FORBIDDEN,
            "WebSocket origin not allowed".to_string(),
        ));
    }
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::ws::handle_ws_connection(socket, state, identity)
    }))
}

// ── Thread management and history handlers ────────────────────────────

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub thread_id: Option<String>,
    pub limit: Option<usize>,
    pub before: Option<String>,
}

pub async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(identity): AuthenticatedUser,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager
        .get_or_create_session(&identity.user_id)
        .await;

    let limit = query.limit.unwrap_or(50);
    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid 'before' timestamp".to_string(),
                    )
                })
        })
        .transpose()?;

    // Find the thread (lock only briefly to get active_thread if needed)
    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        let sess = session.lock().await;
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };

    // Verify the thread belongs to the authenticated user before returning any data.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &identity.user_id)
            .await
            .unwrap_or(false);
        if !owned {
            let sess = session.lock().await;
            if !sess.threads.contains_key(&thread_id) {
                return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
            }
        }
    }

    // For paginated requests (before cursor set), always go to DB
    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
            pending_approval: None,
            pending_auth: None,
        }));
    }

    // Try in-memory first (freshest data for active threads)
    // Lock only when checking in-memory state
    {
        let sess = session.lock().await;
        if let Some(thread) = sess.threads.get(&thread_id)
            && (!thread.turns.is_empty() || thread.pending_approval.is_some())
        {
            let turns: Vec<TurnInfo> = thread
                .turns
                .iter()
                .map(|t| TurnInfo {
                    turn_number: t.turn_number,
                    user_input: t.user_input.clone(),
                    response: t.response.clone(),
                    state: format!("{:?}", t.state),
                    started_at: t.started_at.to_rfc3339(),
                    completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
                    tool_calls: t
                        .tool_calls
                        .iter()
                        .map(|tc| ToolCallInfo {
                            name: tc.name.clone(),
                            has_result: tc.result.is_some(),
                            has_error: tc.error.is_some(),
                            result_preview: tc.result.as_ref().map(|r| {
                                let s = match r {
                                    serde_json::Value::String(s) => s.clone(),
                                    other => other.to_string(),
                                };
                                truncate_preview(&s, 500)
                            }),
                            error: tc.error.as_deref().map(tool_error_for_display),
                            rationale: tc.rationale.clone(),
                        })
                        .collect(),
                    narrative: t.narrative.clone(),
                })
                .collect();

            let pending_approval = thread
                .pending_approval
                .as_ref()
                .map(|pa| PendingApprovalInfo {
                    request_id: pa.request_id.to_string(),
                    tool_name: pa.tool_name.clone(),
                    description: pa.description.clone(),
                    parameters: serde_json::to_string_pretty(&pa.parameters).unwrap_or_default(),
                });

            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more: false,
                oldest_timestamp: None,
                pending_approval,
                pending_auth: None,
            }));
        }
    }

    // Fall back to DB for historical threads not in memory (paginated)
    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
                pending_approval: None,
                pending_auth: None,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
        pending_approval: None,
        pending_auth: None,
    }))
}

pub async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(identity): AuthenticatedUser,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager
        .get_or_create_session(&identity.user_id)
        .await;

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        // Auto-create assistant thread if it doesn't exist
        let assistant_id = store
            .get_or_create_assistant_conversation(&identity.user_id, "gateway")
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if let Ok(summaries) = store
            .list_conversations_all_channels(&identity.user_id, 50)
            .await
        {
            let mut assistant_thread = None;
            let mut threads = Vec::new();

            for s in &summaries {
                let info = ThreadInfo {
                    id: s.id,
                    state: "Idle".to_string(),
                    turn_count: s.message_count.max(0) as usize,
                    created_at: s.started_at.to_rfc3339(),
                    updated_at: s.last_activity.to_rfc3339(),
                    title: s.title.clone(),
                    thread_type: s.thread_type.clone(),
                    channel: Some(s.channel.clone()),
                };

                if s.id == assistant_id {
                    assistant_thread = Some(info);
                } else {
                    threads.push(info);
                }
            }

            // If assistant wasn't in the list (0 messages), synthesize it
            if assistant_thread.is_none() {
                assistant_thread = Some(ThreadInfo {
                    id: assistant_id,
                    state: "Idle".to_string(),
                    turn_count: 0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    title: None,
                    thread_type: Some("assistant".to_string()),
                    channel: Some("gateway".to_string()),
                });
            }

            // Read active thread while holding minimal lock (just before return)
            let active_thread = {
                let sess = session.lock().await;
                sess.active_thread
            };

            return Ok(Json(ThreadListResponse {
                assistant_thread,
                threads,
                active_thread,
            }));
        }
    }

    // Fallback: in-memory only (no assistant thread without DB)
    let sess = session.lock().await;
    let mut sorted_threads: Vec<_> = sess.threads.values().collect();
    sorted_threads.sort_by_key(|t| std::cmp::Reverse(t.updated_at));
    let threads: Vec<ThreadInfo> = sorted_threads
        .into_iter()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
            channel: Some("gateway".to_string()),
        })
        .collect();

    let active_thread = sess.active_thread;
    drop(sess); // Explicit drop to release lock

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread,
    }))
}

pub async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(identity): AuthenticatedUser,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager
        .get_or_create_session(&identity.user_id)
        .await;
    let (thread_id, info) = {
        let mut sess = session.lock().await;
        let thread = sess.create_thread(Some("web"));
        let id = thread.id;
        let info = ThreadInfo {
            id: thread.id,
            state: format!("{:?}", thread.state),
            turn_count: thread.turns.len(),
            created_at: thread.created_at.to_rfc3339(),
            updated_at: thread.updated_at.to_rfc3339(),
            title: None,
            thread_type: Some("thread".to_string()),
            channel: Some("gateway".to_string()),
        };
        (id, info)
    };

    // Persist the empty conversation row with thread_type metadata synchronously
    // so that the subsequent loadThreads() call from the frontend sees it.
    if let Some(ref store) = state.store {
        match store
            .ensure_conversation(
                thread_id,
                "gateway",
                &identity.user_id,
                None,
                Some("gateway"),
            )
            .await
        {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                user = %identity.user_id,
                thread_id = %thread_id,
                "Skipped persisting new thread due to ownership/channel conflict"
            ),
            Err(e) => tracing::warn!("Failed to persist new thread: {}", e),
        }
        let metadata_val = serde_json::json!("thread");
        if let Err(e) = store
            .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
            .await
        {
            tracing::warn!("Failed to set thread_type metadata: {}", e);
        }
    }

    Ok(Json(info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::web::util::build_turns_from_db_messages;

    #[test]
    fn test_build_turns_from_db_messages_complete() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "How are you?".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Doing well!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How are you?");
        assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
    }

    #[test]
    fn test_build_turns_from_db_messages_incomplete_last() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Lost message".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_input, "Lost message");
        assert!(turns[1].response.is_none());
        assert_eq!(turns[1].state, "Failed");
    }

    #[test]
    fn test_build_turns_with_tool_calls() {
        let now = chrono::Utc::now();
        let tool_calls_json = serde_json::json!([
            {"name": "shell", "result_preview": "file1.txt\nfile2.txt"},
            {"name": "http", "error": "timeout"}
        ]);
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "List files".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "tool_calls".to_string(),
                content: tool_calls_json.to_string(),
                created_at: now + chrono::TimeDelta::milliseconds(500),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Here are the files".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].tool_calls.len(), 2);
        assert_eq!(turns[0].tool_calls[0].name, "shell");
        assert!(turns[0].tool_calls[0].has_result);
        assert!(!turns[0].tool_calls[0].has_error);
        assert_eq!(
            turns[0].tool_calls[0].result_preview.as_deref(),
            Some("file1.txt\nfile2.txt")
        );
        assert_eq!(turns[0].tool_calls[1].name, "http");
        assert!(turns[0].tool_calls[1].has_error);
        assert_eq!(turns[0].tool_calls[1].error.as_deref(), Some("timeout"));
        assert_eq!(turns[0].response.as_deref(), Some("Here are the files"));
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_build_turns_with_malformed_tool_calls() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "tool_calls".to_string(),
                content: "not valid json".to_string(),
                created_at: now + chrono::TimeDelta::milliseconds(500),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Done".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_calls.is_empty());
        assert_eq!(turns[0].response.as_deref(), Some("Done"));
    }

    #[test]
    fn test_build_turns_backward_compatible_no_tool_calls() {
        // Old threads without tool_calls messages still work
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].tool_calls.is_empty());
        assert_eq!(turns[0].response.as_deref(), Some("Hi!"));
        assert_eq!(turns[0].state, "Completed");
    }
}
