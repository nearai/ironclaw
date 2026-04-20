//! Chat feature slice.
//!
//! Owns the browser-facing chat surface end-to-end: message ingress, gate
//! resolution, thread management, history playback, SSE event stream, and
//! the WebSocket upgrade. This is the biggest slice extracted so far
//! (ironclaw#2599 stage 4c) — prior stages (oauth, pairing, status, logs)
//! left chat in `server.rs` because the gate-flow and SSE/WS reconnect
//! surfaces needed the widest review window.
//!
//! # Route ownership
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | POST | `/api/chat/send` | [`chat_send_handler`] |
//! | POST | `/api/chat/approval` | [`chat_approval_handler`] |
//! | POST | `/api/chat/gate/resolve` | [`chat_gate_resolve_handler`] |
//! | POST | `/api/chat/auth-token` | [`chat_auth_token_handler`] (legacy v1 shim) |
//! | POST | `/api/chat/auth-cancel` | [`chat_auth_cancel_handler`] (legacy v1 shim) |
//! | GET | `/api/chat/ws` | [`chat_ws_handler`] |
//! | GET | `/api/chat/events` | [`chat_events_handler`] |
//! | GET | `/api/chat/history` | [`chat_history_handler`] |
//! | GET | `/api/chat/threads` | [`chat_threads_handler`] |
//! | POST | `/api/chat/thread/new` | [`chat_new_thread_handler`] |
//!
//! # Dependency boundary
//!
//! The slice calls into:
//!
//! - [`crate::channels::web::util`] for shared helpers
//!   (`web_incoming_message`, `build_turns_from_db_messages`,
//!   `images_to_attachments`, `tool_*`, image-budget enforcement).
//! - [`crate::channels::web::platform::engine_dispatch`] for structured
//!   submissions to the agent loop (gate resolutions, credential
//!   provisioning, cancellations) — migrated into platform in stage 4b.
//! - [`crate::channels::web::platform::legacy_auth`] for the
//!   pre-gate `pending_auth` compatibility path — migrated in stage 4b.
//! - [`crate::bridge`] for the engine v2 pending-gate store and for
//!   the canonical auth-flow identity resolver
//!   (`auth_manager::resolve_auth_flow_extension_name`). The `CLAUDE.md`
//!   "Extension/Auth Invariants" rule requires every gate-display /
//!   resume path to go through this single resolver; the slice's
//!   [`pending_gate_extension_name`] helper is the one wrapper, audited
//!   by check #8 in `scripts/pre-commit-safety.sh`.
//!
//! # In-progress reconciliation
//!
//! The history handler produces a [`HistoryResponse`] that has to agree
//! with both the persisted turn log and any live "Processing…"
//! affordance the server-side engine is driving. The reconciliation
//! helpers ([`reconcile_in_progress_with_turns`] and friends) have
//! unit-test coverage that pins every edge case (stale live state,
//! matching turn with completed response, mismatched message IDs,
//! unpersisted next turn, legacy live-state vs completed-turn). Any
//! change to the helpers must keep those tests green.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::{
    ActionResponse, ApprovalRequest, GateResolutionPayload, GateResolveRequest, HistoryResponse,
    InProgressInfo, PendingGateInfo, SendMessageRequest, SendMessageResponse, ThreadInfo,
    ThreadListResponse, ToolCallInfo, TurnInfo,
};
use crate::channels::web::util::{
    build_turns_from_db_messages, collect_generated_images_from_tool_results,
    enforce_generated_image_history_budget, tool_error_for_display, tool_result_preview,
    web_incoming_message,
};

// ── Handlers ──────────────────────────────────────────────────────────

pub(crate) async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    tracing::trace!(
        "[chat_send_handler] Received message: content_len={}, thread_id={:?}",
        req.content.len(),
        req.thread_id
    );

    if !state.chat_rate_limiter.check(&user.user_id) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let mut msg = web_incoming_message(
        "gateway",
        &user.user_id,
        &req.content,
        req.thread_id.as_deref(),
    );
    // Prefer timezone from JSON body, fall back to X-Timezone header
    let tz = req
        .timezone
        .as_deref()
        .or_else(|| headers.get("X-Timezone").and_then(|v| v.to_str().ok()));
    if let Some(tz) = tz {
        msg = msg.with_timezone(tz);
    }

    // Convert uploaded images + generic file attachments to IncomingAttachments
    // through the shared budget-aware helper so HTTP and WS paths enforce
    // identical limits. Empty-text messages with attachments are still valid
    // here; the v2 engine router relaxes the empty-input guard downstream.
    let incoming_attachments =
        crate::channels::web::util::inline_attachments_to_incoming(&req.images, &req.attachments)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if !incoming_attachments.is_empty() {
        msg = msg.with_attachments(incoming_attachments);
    }

    let msg_id = msg.id;
    tracing::trace!(
        "[chat_send_handler] Created message id={}, content_len={}, images={}",
        msg_id,
        req.content.len(),
        req.images.len()
    );

    // Clone sender to avoid holding RwLock read guard across send().await
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    tracing::debug!("[chat_send_handler] Sending message through channel");
    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    tracing::debug!("[chat_send_handler] Message sent successfully, returning 202 ACCEPTED");

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown action: {}", other),
            ));
        }
    };

    let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;

    // Build a structured ExecApproval submission as JSON, sent through the
    // existing message pipeline so the agent loop picks it up.
    let approval = crate::agent::submission::Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize approval: {}", e),
        )
    })?;

    let msg = web_incoming_message("gateway", &user.user_id, content, req.thread_id.as_deref());

    let msg_id = msg.id;

    // Clone sender to avoid holding RwLock read guard across send().await
    let tx = {
        let tx_guard = state.msg_tx.read().await;
        tx_guard
            .as_ref()
            .ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Channel not started".to_string(),
            ))?
            .clone()
    };

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_gate_resolve_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<GateResolveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    match req.resolution {
        GateResolutionPayload::Approved { always } => {
            let action = if always { "always" } else { "approve" }.to_string();
            let _ = chat_approval_handler(
                State(state),
                AuthenticatedUser(user),
                Json(ApprovalRequest {
                    request_id: req.request_id,
                    action,
                    thread_id: req.thread_id,
                }),
            )
            .await?;
            Ok(Json(ActionResponse::ok("Gate resolution accepted.")))
        }
        GateResolutionPayload::Denied => {
            let _ = chat_approval_handler(
                State(state),
                AuthenticatedUser(user),
                Json(ApprovalRequest {
                    request_id: req.request_id,
                    action: "deny".into(),
                    thread_id: req.thread_id,
                }),
            )
            .await?;
            Ok(Json(ActionResponse::ok("Gate resolution accepted.")))
        }
        GateResolutionPayload::CredentialProvided { token } => {
            let thread_id = req.thread_id.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required for credential resolution".to_string(),
            ))?;
            let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid request_id (expected UUID)".to_string(),
                )
            })?;
            let submission = crate::agent::submission::Submission::GateAuthResolution {
                request_id,
                resolution: crate::agent::submission::AuthGateResolution::CredentialProvided {
                    token,
                },
            };
            // Use a structured submission instead of replaying the token as a
            // normal user message. The parser handles this before BeforeInbound
            // hooks, and the bridge resolves the exact gate `request_id`.
            crate::channels::web::platform::engine_dispatch::dispatch_engine_submission(
                &state,
                &user.user_id,
                &thread_id,
                submission,
            )
            .await?;
            Ok(Json(ActionResponse::ok("Credential submitted.")))
        }
        GateResolutionPayload::Cancelled => {
            let thread_id = req.thread_id.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required for cancellation".to_string(),
            ))?;
            let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid request_id (expected UUID)".to_string(),
                )
            })?;
            let submission = crate::agent::submission::Submission::GateAuthResolution {
                request_id,
                resolution: crate::agent::submission::AuthGateResolution::Cancelled,
            };
            crate::channels::web::platform::engine_dispatch::dispatch_engine_submission(
                &state,
                &user.user_id,
                &thread_id,
                submission,
            )
            .await?;
            Ok(Json(ActionResponse::ok("Gate cancelled.")))
        }
    }
}

pub(crate) async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<crate::channels::web::types::AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    crate::channels::web::platform::legacy_auth::handle_legacy_auth_token_submission(
        &state,
        &user.user_id,
        req,
    )
    .await
    .map(Json)
}

pub(crate) async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<crate::channels::web::types::AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    crate::channels::web::platform::legacy_auth::handle_legacy_auth_cancel(
        &state,
        &user.user_id,
        req,
    )
    .await
    .map(Json)
}

pub(crate) async fn chat_events_handler(
    Query(params): Query<ChatEventsQuery>,
    headers: HeaderMap,
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let sse = state
        .sse
        .subscribe(Some(user.user_id), extract_last_event_id(&params, &headers))
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many connections".to_string(),
        ))?;
    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        sse,
    ))
}

pub(crate) async fn chat_ws_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate Origin header to prevent cross-site WebSocket hijacking.
    // Require the header outright; browsers always send it for WS upgrades,
    // so a missing Origin means a non-browser client trying to bypass the check.
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    let is_local = is_local_origin(origin);
    if !is_local {
        return Err((
            StatusCode::FORBIDDEN,
            "WebSocket origin not allowed".to_string(),
        ));
    }
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::platform::ws::handle_ws_connection(socket, state, user)
    }))
}

pub(crate) async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let sess = session.lock().await;

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

    // Find the thread
    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };
    let thread_id_str = thread_id.to_string();
    let thread_scope = Some(thread_id_str.as_str());

    // Verify the thread belongs to the authenticated user before returning any data.
    // In-memory threads are already scoped by user via session_manager, but DB
    // lookups could expose another user's conversation if the UUID is guessed.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &user.user_id)
            .await
            .map_err(|e| {
                tracing::error!(thread_id = %thread_id, error = %e, "DB error during thread ownership check");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string())
            })?;
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
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
        let mut turns = build_turns_from_db_messages(&messages);
        enforce_generated_image_history_budget(&mut turns);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
            pending_gate: history_pending_gate_info(&state, &user.user_id, thread_scope).await,
            in_progress: None,
        }));
    }

    // Try in-memory first (freshest data for active threads)
    if let Some(thread) = sess.threads.get(&thread_id)
        && (!thread.turns.is_empty() || thread.pending_approval.is_some())
    {
        let mut turns: Vec<TurnInfo> = thread
            .turns
            .iter()
            .map(turn_info_from_in_memory_turn)
            .collect();
        enforce_generated_image_history_budget(&mut turns);

        let pending_gate = history_pending_gate_info(&state, &user.user_id, thread_scope)
            .await
            .or_else(|| {
                thread.pending_approval.as_ref().map(|pa| PendingGateInfo {
                    request_id: pa.request_id.to_string(),
                    thread_id: thread_id.to_string(),
                    gate_name: "approval".into(),
                    tool_name: pa.tool_name.clone(),
                    description: pa.description.clone(),
                    parameters: serde_json::to_string_pretty(&pa.parameters).unwrap_or_default(),
                    extension_name: None,
                    resume_kind: serde_json::json!({"Approval":{"allow_always":true}}),
                })
            });

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
            pending_gate,
            in_progress: in_progress_from_thread(thread),
        }));
    }

    // Fall back to DB for historical threads not in memory (paginated)
    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let mut turns = build_turns_from_db_messages(&messages);
            let metadata = store
                .get_conversation_metadata(thread_id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let in_progress = reconcile_in_progress_with_turns(
                &mut turns,
                in_progress_from_metadata(metadata.as_ref()),
            );
            enforce_generated_image_history_budget(&mut turns);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
                pending_gate: history_pending_gate_info(&state, &user.user_id, thread_scope).await,
                in_progress,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    let in_progress = if let Some(ref store) = state.store {
        let metadata = store
            .get_conversation_metadata(thread_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut turns = Vec::new();
        reconcile_in_progress_with_turns(&mut turns, in_progress_from_metadata(metadata.as_ref()))
    } else {
        None
    };
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
        pending_gate: history_pending_gate_info(&state, &user.user_id, thread_scope).await,
        in_progress,
    }))
}

pub(crate) async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let sess = session.lock().await;
    let live_thread_states: std::collections::HashMap<Uuid, String> = sess
        .threads
        .iter()
        .map(|(id, thread)| (*id, thread_state_label(thread.state).to_string()))
        .collect();
    drop(sess);

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        // Auto-create assistant thread if it doesn't exist
        let assistant_id = store
            .get_or_create_assistant_conversation(&user.user_id, "gateway")
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        match store
            .list_conversations_all_channels(&user.user_id, 50)
            .await
        {
            Ok(summaries) => {
                let mut assistant_thread = None;
                let mut threads = Vec::new();

                for s in &summaries {
                    let info = ThreadInfo {
                        id: s.id,
                        state: live_thread_states
                            .get(&s.id)
                            .cloned()
                            .or_else(|| summary_live_state(s))
                            .unwrap_or_else(|| "Idle".to_string()),
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
                        state: live_thread_states
                            .get(&assistant_id)
                            .cloned()
                            .unwrap_or_else(|| "Idle".to_string()),
                        turn_count: 0,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        title: None,
                        thread_type: Some("assistant".to_string()),
                        channel: Some("gateway".to_string()),
                    });
                }

                let active_thread = session.lock().await.active_thread;

                return Ok(Json(ThreadListResponse {
                    assistant_thread,
                    threads,
                    active_thread,
                }));
            }
            Err(e) => {
                tracing::error!(user_id = %user.user_id, error = %e, "DB error listing threads; falling back to in-memory");
            }
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
            state: thread_state_label(t.state).to_string(),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
            channel: Some("gateway".to_string()),
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

pub(crate) async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let (thread_id, info) = {
        let mut sess = session.lock().await;
        let thread = sess.create_thread(Some("gateway"));
        let id = thread.id;
        let info = ThreadInfo {
            id: thread.id,
            state: thread_state_label(thread.state).to_string(),
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
            .ensure_conversation(thread_id, "gateway", &user.user_id, None, Some("gateway"))
            .await
        {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                user = %user.user_id,
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

// ── Slice-private helpers ─────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ChatEventsQuery {
    pub last_event_id: Option<String>,
}

pub(crate) fn extract_last_event_id(
    params: &ChatEventsQuery,
    headers: &HeaderMap,
) -> Option<String> {
    params.last_event_id.clone().or_else(|| {
        headers
            .get(HeaderName::from_static("last-event-id"))
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
    })
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
}

/// Check whether an Origin header value points to a local address.
///
/// Extracts the host from the origin (handling both IPv4/hostname and IPv6
/// literal formats) and compares it against known local addresses. Used to
/// prevent cross-site WebSocket hijacking while allowing localhost access.
pub(crate) fn is_local_origin(origin: &str) -> bool {
    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .and_then(|rest| {
            if rest.starts_with('[') {
                // IPv6 literal: extract "[::1]" up to and including ']'
                rest.find(']').map(|i| &rest[..=i])
            } else {
                // IPv4 or hostname: take up to the first ':' (port) or '/' (path)
                rest.split(':').next()?.split('/').next()
            }
        })
        .unwrap_or("");

    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

pub(crate) async fn pending_gate_extension_name(
    state: &GatewayState,
    user_id: &str,
    tool_name: &str,
    parameters: &str,
    resume_kind: &ironclaw_engine::ResumeKind,
) -> Option<ironclaw_common::ExtensionName> {
    let ironclaw_engine::ResumeKind::Authentication {
        credential_name, ..
    } = resume_kind
    else {
        return None;
    };

    let parsed_parameters =
        serde_json::from_str::<serde_json::Value>(parameters).unwrap_or(serde_json::Value::Null);

    // Both the "auth manager present" and "bare test harness" paths
    // delegate to the single canonical resolver (see
    // `src/bridge/auth_manager.rs::resolve_auth_flow_extension_name`) so
    // the four branches stay aligned. Without this delegation the wrapper
    // would drift — check #8 in `scripts/pre-commit-safety.sh` and the
    // "one resolver" rule in `src/bridge/CLAUDE.md` exist to prevent
    // exactly that drift.
    Some(
        crate::bridge::auth_manager::resolve_auth_flow_extension_name(
            tool_name,
            &parsed_parameters,
            credential_name.as_str(),
            user_id,
            state.tool_registry.as_deref(),
            state.extension_manager.as_deref(),
        )
        .await,
    )
}

async fn engine_pending_gate_info(
    state: &GatewayState,
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    let pending = crate::bridge::get_engine_pending_gate(user_id, thread_id)
        .await
        .ok()??;
    let extension_name = pending_gate_extension_name(
        state,
        user_id,
        &pending.tool_name,
        &pending.parameters,
        &pending.resume_kind,
    )
    .await;
    Some(PendingGateInfo {
        request_id: pending.request_id,
        thread_id: pending.thread_id.to_string(),
        gate_name: pending.gate_name,
        tool_name: pending.tool_name,
        description: pending.description,
        parameters: pending.parameters,
        extension_name,
        resume_kind: serde_json::to_value(pending.resume_kind).unwrap_or_default(),
    })
}

async fn history_pending_gate_info(
    state: &GatewayState,
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    if thread_id.is_some() {
        // Thread-scoped pending gates are authoritative once the client sends a
        // thread_id. The unscoped fallback only exists for legacy callers that
        // do not know which thread owns the gate yet.
        return engine_pending_gate_info(state, user_id, thread_id).await;
    }
    engine_pending_gate_info(state, user_id, None).await
}

fn turn_info_from_in_memory_turn(t: &crate::agent::session::Turn) -> TurnInfo {
    TurnInfo {
        turn_number: t.turn_number,
        user_message_id: t.user_message_id,
        user_input: t.user_input.clone(),
        response: t.response.clone(),
        state: turn_state_label(t.state).to_string(),
        started_at: t.started_at.to_rfc3339(),
        completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
        tool_calls: t
            .tool_calls
            .iter()
            .map(|tc| {
                // In-memory turns only retain the full result (`tc.result`); no
                // separate short preview is persisted the way the DB path stores
                // `result_preview`. Populate `result` from the live value so the
                // UI can expand it, and leave `result_preview` empty to match
                // the DB semantics where preview and result are distinct fields.
                ToolCallInfo {
                    name: tc.name.clone(),
                    has_result: tc.result.is_some(),
                    has_error: tc.error.is_some(),
                    call_id: tc.tool_call_id.clone(),
                    result: tool_result_preview(tc.result.as_ref()),
                    result_preview: None,
                    error: tc.error.as_deref().map(tool_error_for_display),
                    rationale: tc.rationale.clone(),
                }
            })
            .collect(),
        generated_images: collect_generated_images_from_tool_results(
            t.turn_number,
            t.tool_calls
                .iter()
                .map(|tc| (tc.tool_call_id.as_deref(), tc.result.as_ref())),
        ),
        narrative: t.narrative.clone(),
    }
}

fn in_progress_from_thread(thread: &crate::agent::session::Thread) -> Option<InProgressInfo> {
    if thread.state != crate::agent::session::ThreadState::Processing {
        return None;
    }
    let turn = thread.turns.last()?;
    if turn.state != crate::agent::session::TurnState::Processing {
        return None;
    }
    Some(InProgressInfo {
        turn_number: turn.turn_number,
        user_message_id: turn.user_message_id,
        state: "Processing".to_string(),
        user_input: turn.user_input.clone(),
        started_at: turn.started_at.to_rfc3339(),
    })
}

pub(crate) const IN_PROGRESS_STALE_AFTER_MINUTES: i64 = 10;

fn thread_state_label(state: crate::agent::session::ThreadState) -> &'static str {
    match state {
        crate::agent::session::ThreadState::Idle => "Idle",
        crate::agent::session::ThreadState::Processing => "Processing",
        crate::agent::session::ThreadState::AwaitingApproval => "AwaitingApproval",
        crate::agent::session::ThreadState::Completed => "Completed",
        crate::agent::session::ThreadState::Interrupted => "Interrupted",
    }
}

fn turn_state_label(state: crate::agent::session::TurnState) -> &'static str {
    match state {
        crate::agent::session::TurnState::Processing => "Processing",
        crate::agent::session::TurnState::Completed => "Completed",
        crate::agent::session::TurnState::Failed => "Failed",
        crate::agent::session::TurnState::Interrupted => "Interrupted",
    }
}

fn in_progress_matches_turn(last_turn: &TurnInfo, in_progress: &InProgressInfo) -> bool {
    if last_turn.user_message_id.is_some() && in_progress.user_message_id.is_some() {
        return last_turn.user_message_id == in_progress.user_message_id;
    }

    // Fallback for non-persistent/in-memory-only modes where no DB message ID exists.
    if last_turn.user_message_id.is_none() && in_progress.user_message_id.is_none() {
        return last_turn.turn_number == in_progress.turn_number;
    }

    last_turn.response.is_none() && last_turn.user_input == in_progress.user_input
}

fn in_progress_from_metadata(metadata: Option<&serde_json::Value>) -> Option<InProgressInfo> {
    let raw = metadata?.get("live_state")?;
    if raw.is_null() {
        return None;
    }
    serde_json::from_value::<InProgressInfo>(raw.clone())
        .ok()
        .filter(|live| live.state == "Processing")
        .filter(|live| !is_stale_in_progress(live))
}

fn is_stale_in_progress(in_progress: &InProgressInfo) -> bool {
    chrono::DateTime::parse_from_rfc3339(&in_progress.started_at)
        .ok()
        .map(|started_at| {
            chrono::Utc::now().signed_duration_since(started_at.with_timezone(&chrono::Utc))
                > chrono::Duration::minutes(IN_PROGRESS_STALE_AFTER_MINUTES)
        })
        .unwrap_or(true)
}

fn completed_turn_is_newer_than_in_progress(
    last_turn: &TurnInfo,
    in_progress: &InProgressInfo,
) -> bool {
    if last_turn.response.is_none() || in_progress.user_message_id.is_some() {
        return false;
    }

    let Ok(in_progress_started_at) = chrono::DateTime::parse_from_rfc3339(&in_progress.started_at)
    else {
        return true;
    };

    let completed_or_started_at = last_turn
        .completed_at
        .as_deref()
        .unwrap_or(&last_turn.started_at);

    chrono::DateTime::parse_from_rfc3339(completed_or_started_at)
        .ok()
        .is_some_and(|last_turn_time| last_turn_time >= in_progress_started_at)
}

fn reconcile_in_progress_with_turns(
    turns: &mut [TurnInfo],
    in_progress: Option<InProgressInfo>,
) -> Option<InProgressInfo> {
    let in_progress = in_progress?;

    if is_stale_in_progress(&in_progress) {
        return None;
    }

    let Some(last_turn) = turns.last_mut() else {
        return Some(in_progress);
    };

    if in_progress_matches_turn(last_turn, &in_progress) {
        if last_turn.response.is_some() {
            None
        } else {
            last_turn.state = in_progress.state.clone();
            Some(in_progress)
        }
    } else if completed_turn_is_newer_than_in_progress(last_turn, &in_progress)
        || last_turn.turn_number >= in_progress.turn_number
    {
        None
    } else {
        Some(in_progress)
    }
}

fn summary_live_state(summary: &crate::history::ConversationSummary) -> Option<String> {
    let live_state = summary.live_state.as_ref()?;
    let started_at = summary.live_state_started_at.as_deref()?;

    (!is_stale_in_progress(&InProgressInfo {
        turn_number: 0,
        user_message_id: None,
        state: "Processing".to_string(),
        user_input: String::new(),
        started_at: started_at.to_string(),
    }))
    .then(|| live_state.clone())
}

// ── Tests ──────────────────────────────────────────────────────────────
//
// Helper-level unit tests for chat-private state reconciliation, origin
// validation, turn-info construction, and live-state summarization.
// Caller-level tests (`test_chat_history_handler_*`,
// `test_chat_approval_handler*`, `test_chat_auth_*_handler*`,
// `test_chat_gate_resolve_handler*`) still live in `server.rs::tests`;
// the shared `GatewayState` builders they depend on (`test_gateway_state`,
// `test_gateway_state_with_store_and_session_manager`,
// `test_gateway_state_with_dependencies`) now live in
// `crate::channels::web::test_helpers` as `pub(crate)` functions, so
// stage 6 of ironclaw#2599 can migrate the caller-level tests into this
// module alongside the helpers without an API change.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_turn_info_unwraps_wrapped_tool_error_for_display() {
        let mut thread = crate::agent::session::Thread::new(Uuid::new_v4(), Some("gateway"));
        thread.start_turn("Fetch example");
        {
            let turn = thread.turns.last_mut().expect("turn");
            turn.record_tool_call("http", serde_json::json!({"url": "https://example.com"}));
            turn.record_tool_error(
                "<tool_output name=\"http\">\nTool 'http' failed: timeout\n</tool_output>",
            );
        }

        let info = turn_info_from_in_memory_turn(&thread.turns[0]);

        assert_eq!(info.tool_calls.len(), 1);
        assert_eq!(
            info.tool_calls[0].error.as_deref(),
            Some("Tool 'http' failed: timeout")
        );
    }

    #[test]
    fn test_in_memory_turn_info_populates_result_without_preview() {
        let mut thread = crate::agent::session::Thread::new(Uuid::new_v4(), Some("gateway"));
        thread.start_turn("search");
        {
            let turn = thread.turns.last_mut().expect("turn");
            turn.record_tool_call("memory_search", serde_json::json!({"query": "notes"}));
            turn.record_tool_result(serde_json::json!("found 3 notes"));
        }

        let info = turn_info_from_in_memory_turn(&thread.turns[0]);

        assert_eq!(info.tool_calls.len(), 1);
        assert!(info.tool_calls[0].has_result);
        assert_eq!(
            info.tool_calls[0].result.as_deref(),
            Some("found 3 notes"),
            "in-memory path surfaces full result on `result`"
        );
        assert!(
            info.tool_calls[0].result_preview.is_none(),
            "in-memory path has no separate preview — leave `result_preview` empty to match DB semantics"
        );
    }

    #[test]
    fn test_reconcile_in_progress_with_turns_drops_completed_matching_turn() {
        let started_at = chrono::Utc::now().to_rfc3339();
        let user_message_id = Uuid::new_v4();
        let mut turns = vec![TurnInfo {
            turn_number: 1,
            user_message_id: Some(user_message_id),
            user_input: "What is 2+2?".to_string(),
            response: Some("4".to_string()),
            state: "Completed".to_string(),
            started_at: started_at.clone(),
            completed_at: Some(started_at.clone()),
            tool_calls: Vec::new(),
            generated_images: Vec::new(),
            narrative: None,
        }];

        let in_progress = reconcile_in_progress_with_turns(
            &mut turns,
            Some(InProgressInfo {
                turn_number: 1,
                user_message_id: Some(user_message_id),
                state: "Processing".to_string(),
                user_input: "What is 2+2?".to_string(),
                started_at,
            }),
        );

        assert!(in_progress.is_none());
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_reconcile_in_progress_with_turns_preserves_unpersisted_next_turn() {
        let started_at = chrono::Utc::now().to_rfc3339();
        let mut turns = vec![TurnInfo {
            turn_number: 1,
            user_message_id: Some(Uuid::new_v4()),
            user_input: "Hello".to_string(),
            response: Some("Hi".to_string()),
            state: "Completed".to_string(),
            started_at: started_at.clone(),
            completed_at: Some(started_at.clone()),
            tool_calls: Vec::new(),
            generated_images: Vec::new(),
            narrative: None,
        }];

        let in_progress = reconcile_in_progress_with_turns(
            &mut turns,
            Some(InProgressInfo {
                turn_number: 2,
                user_message_id: Some(Uuid::new_v4()),
                state: "Processing".to_string(),
                user_input: "What is 2+2?".to_string(),
                started_at,
            }),
        );

        assert_eq!(in_progress.as_ref().map(|info| info.turn_number), Some(2));
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_reconcile_in_progress_with_turns_drops_stale_live_state_by_age() {
        let user_message_id = Uuid::new_v4();
        let mut turns = vec![TurnInfo {
            turn_number: 1,
            user_message_id: Some(user_message_id),
            user_input: "Hello".to_string(),
            response: None,
            state: "Processing".to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            tool_calls: Vec::new(),
            generated_images: Vec::new(),
            narrative: None,
        }];

        let in_progress = reconcile_in_progress_with_turns(
            &mut turns,
            Some(InProgressInfo {
                turn_number: 1,
                user_message_id: Some(user_message_id),
                state: "Processing".to_string(),
                user_input: "Hello".to_string(),
                started_at: (chrono::Utc::now()
                    - chrono::Duration::minutes(IN_PROGRESS_STALE_AFTER_MINUTES + 1))
                .to_rfc3339(),
            }),
        );

        assert!(in_progress.is_none());
    }

    #[test]
    fn test_reconcile_in_progress_with_turns_drops_equal_turn_with_mismatched_message_id() {
        let started_at = chrono::Utc::now().to_rfc3339();
        let mut turns = vec![TurnInfo {
            turn_number: 5,
            user_message_id: Some(Uuid::new_v4()),
            user_input: "Question".to_string(),
            response: Some("Answer".to_string()),
            state: "Completed".to_string(),
            started_at: started_at.clone(),
            completed_at: Some(started_at.clone()),
            tool_calls: Vec::new(),
            generated_images: Vec::new(),
            narrative: None,
        }];

        let in_progress = reconcile_in_progress_with_turns(
            &mut turns,
            Some(InProgressInfo {
                turn_number: 5,
                user_message_id: Some(Uuid::new_v4()),
                state: "Processing".to_string(),
                user_input: "Question".to_string(),
                started_at,
            }),
        );

        assert!(in_progress.is_none());
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_reconcile_in_progress_with_turns_drops_legacy_in_progress_if_completed_turn_is_newer() {
        let in_progress_started_at = chrono::Utc::now().to_rfc3339();
        let completed_at = (chrono::Utc::now() + chrono::Duration::seconds(1)).to_rfc3339();
        let mut turns = vec![TurnInfo {
            turn_number: 0,
            user_message_id: Some(Uuid::new_v4()),
            user_input: "Question".to_string(),
            response: Some("Answer".to_string()),
            state: "Completed".to_string(),
            started_at: completed_at.clone(),
            completed_at: Some(completed_at),
            tool_calls: Vec::new(),
            generated_images: Vec::new(),
            narrative: None,
        }];

        let in_progress = reconcile_in_progress_with_turns(
            &mut turns,
            Some(InProgressInfo {
                turn_number: 99,
                user_message_id: None,
                state: "Processing".to_string(),
                user_input: "Legacy question".to_string(),
                started_at: in_progress_started_at,
            }),
        );

        assert!(in_progress.is_none());
        assert_eq!(turns[0].state, "Completed");
    }

    #[test]
    fn test_thread_state_label_is_stable() {
        assert_eq!(
            thread_state_label(crate::agent::session::ThreadState::Processing),
            "Processing"
        );
        assert_eq!(
            thread_state_label(crate::agent::session::ThreadState::AwaitingApproval),
            "AwaitingApproval"
        );
        assert_eq!(
            thread_state_label(crate::agent::session::ThreadState::Interrupted),
            "Interrupted"
        );
    }

    #[test]
    fn test_summary_live_state_drops_stale_processing_state() {
        let summary = crate::history::ConversationSummary {
            id: Uuid::new_v4(),
            title: None,
            message_count: 0,
            started_at: chrono::Utc::now(),
            last_activity: chrono::Utc::now(),
            thread_type: Some("thread".to_string()),
            live_state: Some("Processing".to_string()),
            live_state_started_at: Some(
                (chrono::Utc::now()
                    - chrono::Duration::minutes(IN_PROGRESS_STALE_AFTER_MINUTES + 1))
                .to_rfc3339(),
            ),
            channel: "gateway".to_string(),
        };

        assert!(summary_live_state(&summary).is_none());
    }

    #[test]
    fn test_summary_live_state_drops_missing_started_at() {
        let summary = crate::history::ConversationSummary {
            id: Uuid::new_v4(),
            title: None,
            message_count: 0,
            started_at: chrono::Utc::now(),
            last_activity: chrono::Utc::now(),
            thread_type: Some("thread".to_string()),
            live_state: Some("Processing".to_string()),
            live_state_started_at: None,
            channel: "gateway".to_string(),
        };

        assert!(summary_live_state(&summary).is_none());
    }

    #[test]
    fn test_is_local_origin_localhost() {
        assert!(is_local_origin("http://localhost:3001"));
        assert!(is_local_origin("http://localhost"));
        assert!(is_local_origin("https://localhost:3001"));
    }

    #[test]
    fn test_is_local_origin_ipv4() {
        assert!(is_local_origin("http://127.0.0.1:3001"));
        assert!(is_local_origin("http://127.0.0.1"));
    }

    #[test]
    fn test_is_local_origin_ipv6() {
        assert!(is_local_origin("http://[::1]:3001"));
        assert!(is_local_origin("http://[::1]"));
    }

    #[test]
    fn test_is_local_origin_rejects_remote() {
        assert!(!is_local_origin("http://evil.com"));
        assert!(!is_local_origin("http://localhost.evil.com"));
        assert!(!is_local_origin("http://192.168.1.1:3001"));
    }

    #[test]
    fn test_is_local_origin_rejects_garbage() {
        assert!(!is_local_origin("not-a-url"));
        assert!(!is_local_origin(""));
    }
}
