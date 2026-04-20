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
    // Verbose/debug stream is admin-only — non-admin clients silently
    // get the normal stream so query-param tampering can't leak verbose
    // events. Matches the AdminUser gate on /api/debug/prompt.
    let verbose = params.debug && user.role == "admin";
    let sse = state
        .sse
        .subscribe(
            Some(user.user_id),
            verbose,
            extract_last_event_id(&params, &headers),
        )
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
    Query(params): Query<ChatEventsQuery>,
    State(state): State<Arc<GatewayState>>,
    // `Result<WebSocketUpgrade, _>` instead of plain `WebSocketUpgrade`
    // so the Origin gate below fires *before* the extractor rejects
    // with 426. Two benefits: (a) unit tests reach the origin-rejection
    // branches without needing a real hyper `OnUpgrade` extension
    // (`tower::ServiceExt::oneshot` can't synthesize one), and (b)
    // callers with a bad origin always see a `403` regardless of
    // whether they sent upgrade headers, which is a more accurate
    // security signal than the protocol-level 426.
    ws: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Validate Origin header to prevent cross-site WebSocket hijacking.
    // Require the header outright; browsers always send it for WS upgrades,
    // so a missing Origin means a non-browser client trying to bypass the check.
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return (StatusCode::FORBIDDEN, "WebSocket Origin header required").into_response();
    };

    if !is_local_origin(origin) {
        return (StatusCode::FORBIDDEN, "WebSocket origin not allowed").into_response();
    }

    // Origin accepted — now require the upgrade to succeed. A caller
    // hitting `/api/chat/ws` from a trusted origin but without
    // `Connection: Upgrade` / `Upgrade: websocket` / `Sec-WebSocket-*`
    // (or via a test harness that can't supply hyper's `OnUpgrade`
    // extension) falls into this branch. Return axum's own
    // `WebSocketUpgradeRejection::into_response()` verbatim so RFC 7231
    // §6.5.15 metadata (notably the `Upgrade` header on a 426) is
    // preserved — flagged by PR #2712 review (Copilot + Gemini).
    let ws = match ws {
        Ok(ws) => ws,
        Err(rej) => return rej.into_response(),
    };

    let verbose = params.debug && user.role == "admin";
    ws.on_upgrade(move |socket| {
        crate::channels::web::platform::ws::handle_ws_connection(socket, state, user, verbose)
    })
    .into_response()
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
    // Three ownership sources, in order: v1 conversation row, in-memory v1 session,
    // engine v2 thread store. An engine v2 thread ID will only match the last one
    // because the v1 dual-write uses the *assistant* conversation id, not the
    // engine thread id, so the first two will miss.
    if query.thread_id.is_some() {
        let mut owned = false;
        if let Some(ref store) = state.store {
            owned = match store
                .conversation_belongs_to_user(thread_id, &user.user_id)
                .await
            {
                Ok(owned) => owned,
                Err(error) => {
                    tracing::error!(
                        thread_id = %thread_id,
                        user_id = %user.user_id,
                        %error,
                        "Failed to verify conversation ownership"
                    );
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Database error".to_string(),
                    ));
                }
            };
        }
        if !owned && sess.threads.contains_key(&thread_id) {
            owned = true;
        }
        if !owned
            && let Ok(Some(_)) =
                crate::bridge::get_engine_thread(&thread_id.to_string(), &user.user_id).await
        {
            owned = true;
        }
        if !owned {
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

    // Engine v2 fallback: an engine thread owns its own messages and does not
    // always dual-write them into the v1 conversation table (the assistant
    // flow writes into the *assistant* conversation id, so deep-linking
    // by engine thread id gets a v1 miss). Surface them here so
    // `#/chat/<engine-thread-id>` renders the thread instead of going empty.
    if let Ok(Some(detail)) =
        crate::bridge::get_engine_thread(&thread_id.to_string(), &user.user_id).await
    {
        let synthetic: Vec<crate::history::ConversationMessage> = detail
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| engine_history_entry_to_message(thread_id, index, entry))
            .collect();
        if !synthetic.is_empty() {
            let oldest_timestamp = synthetic.first().map(|m| m.created_at.to_rfc3339());
            let mut turns = build_turns_from_db_messages(&synthetic);
            enforce_generated_image_history_budget(&mut turns);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more: false,
                oldest_timestamp,
                pending_gate: history_pending_gate_info(&state, &user.user_id, thread_scope).await,
                in_progress: None,
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

        // 50 used to be the cap here; threads past that silently disappeared
        // from the sidebar, which also broke hash-based deep links because
        // the UI verified membership before switching. 500 is cheap for a
        // single-user demo and large enough that sliding off the end is
        // rare in practice.
        match store
            .list_conversations_all_channels(&user.user_id, 500)
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

                // Engine v2 threads for this user in the default project. These
                // don't always get a matching v1 conversation row (the assistant
                // flow dual-writes into the single assistant conv id, not the
                // engine thread id), so without this merge they'd be invisible
                // in the sidebar even though the chat history endpoint can now
                // render them by id.
                if let Ok(engine_threads) =
                    crate::bridge::list_engine_threads(None, &user.user_id).await
                {
                    let existing_ids: std::collections::HashSet<uuid::Uuid> = threads
                        .iter()
                        .map(|t| t.id)
                        .chain(assistant_thread.as_ref().map(|a| a.id))
                        .collect();
                    for eng in engine_threads {
                        let Ok(uuid) = uuid::Uuid::parse_str(&eng.id) else {
                            continue;
                        };
                        if existing_ids.contains(&uuid) {
                            continue;
                        }
                        threads.push(ThreadInfo {
                            id: uuid,
                            state: eng.state,
                            turn_count: eng.step_count,
                            created_at: eng.created_at,
                            updated_at: eng.updated_at.clone(),
                            // Engine threads carry their goal as the only
                            // human-readable label; reuse it as the sidebar
                            // title so the user can tell threads apart.
                            title: Some(eng.goal),
                            thread_type: Some(eng.thread_type),
                            channel: Some("engine".to_string()),
                        });
                    }
                    // Re-sort by updated_at descending so engine threads interleave
                    // chronologically with v1 conversations.
                    threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
    #[serde(default)]
    pub debug: bool,
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
    pub(crate) thread_id: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) before: Option<String>,
}

/// Check whether an Origin header value points to a local address.
///
/// Extracts the host from the origin (handling both IPv4/hostname and IPv6
/// literal formats) and compares it against known local addresses. Used to
/// prevent cross-site WebSocket hijacking while allowing localhost access.
pub(crate) fn is_local_origin(origin: &str) -> bool {
    // Accept both `http://localhost` and `http://LOCALHOST`. Browsers
    // normalize the Origin header to lowercase in practice, but RFC 7230
    // §5.4 allows uppercase hostnames. Normalize before parsing so the
    // match never depends on the caller's casing.
    let origin_lc = origin.to_ascii_lowercase();
    let host = origin_lc
        .strip_prefix("http://")
        .or_else(|| origin_lc.strip_prefix("https://"))
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

fn stable_engine_history_message_id(
    thread_id: Uuid,
    index: usize,
    role: &str,
    timestamp: &chrono::DateTime<chrono::Utc>,
    content: &str,
) -> Uuid {
    let seed = format!(
        "engine-v2-history\x1f{thread_id}\x1f{index}\x1f{role}\x1f{}\x1f{content}",
        timestamp.to_rfc3339()
    );
    Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes())
}

fn engine_history_entry_to_message(
    thread_id: Uuid,
    index: usize,
    entry: &serde_json::Value,
) -> Option<crate::history::ConversationMessage> {
    let role_raw = entry.get("role").and_then(|v| v.as_str())?;
    let role = match role_raw {
        "User" => "user",
        "Assistant" => "assistant",
        _ => return None,
    };
    let Some(timestamp_raw) = entry.get("timestamp").and_then(|v| v.as_str()) else {
        tracing::warn!(
            thread_id = %thread_id,
            index,
            "Skipping engine v2 history message without a valid timestamp"
        );
        return None;
    };
    let timestamp = match chrono::DateTime::parse_from_rfc3339(timestamp_raw) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(error) => {
            tracing::warn!(
                thread_id = %thread_id,
                index,
                timestamp = timestamp_raw,
                %error,
                "Skipping engine v2 history message with malformed timestamp"
            );
            return None;
        }
    };
    let content = entry
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Some(crate::history::ConversationMessage {
        id: stable_engine_history_message_id(thread_id, index, role, &timestamp, &content),
        role: role.to_string(),
        content,
        created_at: timestamp,
    })
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
    use std::sync::Arc;

    use axum::{
        Router,
        extract::{Query, State},
        http::StatusCode,
        routing::{get, post},
    };
    use uuid::Uuid;

    use crate::agent::SessionManager;

    use crate::channels::web::auth::UserIdentity;
    use crate::channels::web::features::chat::{
        IN_PROGRESS_STALE_AFTER_MINUTES, chat_approval_handler, chat_auth_cancel_handler,
        chat_auth_token_handler, chat_gate_resolve_handler, chat_history_handler,
        pending_gate_extension_name,
    };
    use crate::db::Database;

    use crate::channels::web::test_helpers::{
        test_gateway_state, test_gateway_state_with_dependencies,
        test_gateway_state_with_store_and_session_manager,
    };
    use crate::channels::web::types::*;

    use crate::testing::credentials::TEST_GATEWAY_CRYPTO_KEY;
    use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};

    use super::*;

    #[test]
    fn test_engine_history_entry_skips_malformed_timestamp() {
        let thread_id = Uuid::new_v4();
        let entry = serde_json::json!({
            "role": "User",
            "content": "hello",
            "timestamp": "not-a-timestamp",
        });

        let message = engine_history_entry_to_message(thread_id, 0, &entry);

        assert!(message.is_none());
    }

    #[test]
    fn test_engine_history_entry_uses_stable_id() {
        let thread_id = Uuid::new_v4();
        let entry = serde_json::json!({
            "role": "Assistant",
            "content": "stable response",
            "timestamp": "2026-04-17T09:30:00Z",
        });

        let first = engine_history_entry_to_message(thread_id, 3, &entry).expect("first message");
        let second = engine_history_entry_to_message(thread_id, 3, &entry).expect("second message");
        let shifted =
            engine_history_entry_to_message(thread_id, 4, &entry).expect("shifted message");

        assert_eq!(first.id, second.id);
        assert_ne!(first.id, shifted.id);
        assert_eq!(first.role, "assistant");
        assert_eq!(first.content, "stable response");
        assert_eq!(first.created_at.to_rfc3339(), "2026-04-17T09:30:00+00:00");
    }

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

    #[test]
    fn test_is_local_origin_accepts_uppercase() {
        // Regression: the exact-case match used to reject `http://LOCALHOST`
        // and other defensible uppercase variants per RFC 7230 §5.4. The
        // implementation now lowercases before parsing so scheme AND host
        // casing are both normalized.
        assert!(is_local_origin("http://LOCALHOST:3001"));
        assert!(is_local_origin("HTTP://localhost"));
        assert!(is_local_origin("HTTP://127.0.0.1"));
        // Spot-check that lowercasing doesn't change the already-passing cases.
        assert!(is_local_origin("http://localhost"));
        assert!(is_local_origin("http://127.0.0.1"));
    }

    // ── Caller-level tests for side-effect-gating handlers ─────────────
    //
    // Per `.claude/rules/testing.md` ("Test Through the Caller, Not Just
    // the Helper"), the four chat handlers below each gate a different
    // side effect (mpsc send, WS upgrade, DB write + session mutation).
    // A helper-level test on their internal predicates isn't enough;
    // these drive the handler directly so a future refactor that drops
    // an input between the predicate and the side effect fails a real
    // regression, not just a lint.

    #[tokio::test]
    async fn test_chat_send_handler_forwards_message_to_msg_tx() {
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::channels::IncomingMessage>(8);
        *state.msg_tx.write().await = Some(tx);

        let req = crate::channels::web::types::SendMessageRequest {
            thread_id: None,
            content: "hello agent".to_string(),
            images: Vec::new(),
            attachments: Vec::new(),
            timezone: None,
        };
        let (status, body) = chat_send_handler(
            axum::extract::State(Arc::clone(&state)),
            crate::channels::web::auth::AuthenticatedUser(UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            axum::http::HeaderMap::new(),
            axum::Json(req),
        )
        .await
        .expect("handler ok");

        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body.status, "accepted");

        // The whole point of the test: the side effect actually fired.
        // Prior regression shape: a wrapper dropped the message silently
        // and returned 202 anyway — caller test catches it, helper test
        // on web_incoming_message alone does not. The timeout prevents
        // such a regression from hanging the whole suite; flagged by
        // PR #2712 review (Copilot).
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("accepted send must enqueue a message promptly")
            .expect("msg_tx must receive the message the handler just accepted");
        assert_eq!(received.content, "hello agent");
        assert_eq!(received.user_id, "alice");
        assert_eq!(received.id, body.message_id);
    }

    #[tokio::test]
    async fn test_chat_send_handler_returns_503_without_channel() {
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        // Deliberately do NOT set msg_tx — the handler must detect the
        // unwired channel and 503 rather than silently drop.
        assert!(state.msg_tx.read().await.is_none());

        let req = crate::channels::web::types::SendMessageRequest {
            thread_id: None,
            content: "noop".to_string(),
            images: Vec::new(),
            attachments: Vec::new(),
            timezone: None,
        };
        let err = chat_send_handler(
            axum::extract::State(Arc::clone(&state)),
            crate::channels::web::auth::AuthenticatedUser(UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            axum::http::HeaderMap::new(),
            axum::Json(req),
        )
        .await
        .expect_err("expected SERVICE_UNAVAILABLE when msg_tx is None");
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_chat_send_handler_rate_limits_after_threshold() {
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::channels::IncomingMessage>(64);
        *state.msg_tx.write().await = Some(tx);

        fn user() -> crate::channels::web::auth::AuthenticatedUser {
            crate::channels::web::auth::AuthenticatedUser(UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            })
        }

        // `PerUserRateLimiter::new(30, 60)` — 30 requests per user per 60s.
        // The 31st must be rejected with 429.
        for _ in 0..30 {
            let req = crate::channels::web::types::SendMessageRequest {
                thread_id: None,
                content: "burst".to_string(),
                images: Vec::new(),
                attachments: Vec::new(),
                timezone: None,
            };
            let (status, _) = chat_send_handler(
                axum::extract::State(Arc::clone(&state)),
                user(),
                axum::http::HeaderMap::new(),
                axum::Json(req),
            )
            .await
            .expect("within-budget call must succeed");
            assert_eq!(status, StatusCode::ACCEPTED);
            // Drain so the channel doesn't block the sender. Wrapped in
            // a timeout so a regression that returns 202 without actually
            // enqueueing can't hang the loop — flagged by PR #2712 review
            // (Copilot).
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .expect("accepted send must enqueue a message promptly")
                .expect("channel closed before queued message was received");
        }

        let req = crate::channels::web::types::SendMessageRequest {
            thread_id: None,
            content: "over-budget".to_string(),
            images: Vec::new(),
            attachments: Vec::new(),
            timezone: None,
        };
        let err = chat_send_handler(
            axum::extract::State(Arc::clone(&state)),
            user(),
            axum::http::HeaderMap::new(),
            axum::Json(req),
        )
        .await
        .expect_err("31st call must be rate-limited");
        assert_eq!(err.0, StatusCode::TOO_MANY_REQUESTS);
    }

    // The three WS handler tests below use `tower::ServiceExt::oneshot`,
    // which cannot synthesize hyper's `OnUpgrade` extension. That means
    // a localhost + valid upgrade-headers request cannot reach the 101
    // SWITCHING_PROTOCOLS response inside these unit tests — it instead
    // stops at the handler's `ws.ok_or(UPGRADE_REQUIRED)?` branch. We
    // use that 426 as the positive signal that the Origin gate passed
    // (the rejection path would have returned 403 first). The real
    // upgrade-completes case is covered by `tests/ws_gateway_integration.rs`
    // where tokio-tungstenite opens a real TCP connection.

    fn ws_request(
        origin: Option<&str>,
        include_upgrade_headers: bool,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = axum::http::Request::builder()
            .method("GET")
            .uri("/api/chat/ws");
        if include_upgrade_headers {
            builder = builder
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==");
        }
        if let Some(o) = origin {
            builder = builder.header("Origin", o);
        }
        let mut req = builder
            .body(axum::body::Body::empty())
            .expect("request build");
        req.extensions_mut().insert(UserIdentity {
            user_id: "alice".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });
        req
    }

    #[tokio::test]
    async fn test_chat_ws_handler_rejects_missing_origin() {
        use tower::ServiceExt;
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        let app = Router::new()
            .route("/api/chat/ws", axum::routing::get(chat_ws_handler))
            .with_state(state);

        let req = ws_request(None, true);
        let resp = ServiceExt::<axum::http::Request<axum::body::Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "WS without Origin must 403 — non-browser client trying to bypass CSRF gate"
        );
    }

    #[tokio::test]
    async fn test_chat_ws_handler_rejects_remote_origin() {
        use tower::ServiceExt;
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        let app = Router::new()
            .route("/api/chat/ws", axum::routing::get(chat_ws_handler))
            .with_state(state);

        let req = ws_request(Some("http://evil.com"), true);
        let resp = ServiceExt::<axum::http::Request<axum::body::Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "WS from remote origin must 403 — cross-site WS hijacking attempt"
        );
    }

    #[tokio::test]
    async fn test_chat_ws_handler_accepts_localhost_origin() {
        use tower::ServiceExt;
        let state = test_gateway_state_with_dependencies(None, None, None, None);
        let app = Router::new()
            .route("/api/chat/ws", axum::routing::get(chat_ws_handler))
            .with_state(state);

        // Valid upgrade + localhost Origin. oneshot can't supply hyper's
        // `OnUpgrade`, so the handler falls through to the `ok_or` branch
        // and returns 426. That 426 (instead of 403) is the positive
        // signal that the Origin gate accepted the request — if the gate
        // had rejected, we'd have gotten 403 before reaching the upgrade
        // check. The real 101 SWITCHING_PROTOCOLS path is exercised by
        // `tests/ws_gateway_integration.rs::test_ws_*` with real TCP.
        let req = ws_request(Some("http://localhost:3001"), true);
        let resp = ServiceExt::<axum::http::Request<axum::body::Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::UPGRADE_REQUIRED,
            "WS with localhost Origin must pass the Origin gate and reach the upgrade \
             step (which oneshot can't complete — real TCP path covered by integration tests)"
        );
    }

    #[tokio::test]
    async fn test_chat_threads_handler_returns_in_memory_threads_without_db() {
        let session_manager = Arc::new(SessionManager::new());
        // Pre-seed one in-memory thread so the handler's fallback branch
        // (no DB store) has something to return.
        {
            let session = session_manager.get_or_create_session("alice").await;
            let mut sess = session.lock().await;
            sess.create_thread(Some("gateway"));
        }
        let mut state = test_gateway_state_with_dependencies(None, None, None, None);
        Arc::get_mut(&mut state)
            .expect("state should be uniquely owned right after construction")
            .session_manager = Some(session_manager);

        let response = chat_threads_handler(
            axum::extract::State(state),
            crate::channels::web::auth::AuthenticatedUser(UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
        )
        .await
        .expect("handler ok");

        // No DB => no assistant_thread synthesized, but the in-memory
        // thread must surface so the sidebar has something to render.
        assert!(response.assistant_thread.is_none());
        assert_eq!(
            response.threads.len(),
            1,
            "handler must surface the in-memory thread when DB is absent"
        );
        assert_eq!(response.threads[0].channel.as_deref(), Some("gateway"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_chat_new_thread_handler_persists_to_db_and_session() {
        let (db, _tmp) = crate::testing::test_db().await;
        let session_manager = Arc::new(SessionManager::new());
        let state =
            test_gateway_state_with_store_and_session_manager(Arc::clone(&db), session_manager);

        let info = chat_new_thread_handler(
            axum::extract::State(Arc::clone(&state)),
            crate::channels::web::auth::AuthenticatedUser(UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
        )
        .await
        .expect("handler ok")
        .0;

        // Both side effects must fire: new thread in session AND conversation
        // row persisted. If either is silently skipped, the sidebar shows
        // a thread that can't be resumed — the bug shape this test pins.
        let session_manager = state.session_manager.as_ref().expect("session manager");
        let session = session_manager.get_or_create_session("alice").await;
        let sess = session.lock().await;
        assert!(
            sess.threads.contains_key(&info.id),
            "new thread must appear in session"
        );
        drop(sess);

        let convs = db
            .list_conversations_all_channels("alice", 50)
            .await
            .expect("list conversations");
        assert!(
            convs.iter().any(|c| c.id == info.id),
            "new thread must be persisted to the conversation store"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_chat_history_handler_drops_stale_in_progress_for_completed_turn() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (db, _tmp) = crate::testing::test_db().await;
        let session_manager = Arc::new(SessionManager::new());
        let state =
            test_gateway_state_with_store_and_session_manager(Arc::clone(&db), session_manager);
        let app = Router::new()
            .route("/api/chat/history", get(chat_history_handler))
            .with_state(state);

        let thread_id = db
            .create_conversation("gateway", "test-user", None)
            .await
            .expect("create conversation");
        let user_message_id = db
            .add_conversation_message(thread_id, "user", "What is 2+2?")
            .await
            .expect("add user message");
        db.add_conversation_message(thread_id, "assistant", "4")
            .await
            .expect("add assistant message");
        db.update_conversation_metadata_field(
            thread_id,
            "live_state",
            &serde_json::json!({
                "turn_number": 0,
                "user_message_id": user_message_id,
                "state": "Processing",
                "user_input": "What is 2+2?",
                "started_at": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .expect("set stale live_state");

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/chat/history?thread_id={thread_id}"))
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("history response json");

        assert!(payload.get("in_progress").is_none());
        let turns = payload["turns"].as_array().expect("turns array");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0]["state"], "Completed");
        assert_eq!(turns[0]["user_input"], "What is 2+2?");
        assert_eq!(turns[0]["response"], "4");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_chat_history_handler_drops_stale_in_progress_when_history_is_windowed() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (db, _tmp) = crate::testing::test_db().await;
        let session_manager = Arc::new(SessionManager::new());
        let state =
            test_gateway_state_with_store_and_session_manager(Arc::clone(&db), session_manager);
        let app = Router::new()
            .route("/api/chat/history", get(chat_history_handler))
            .with_state(state);

        let thread_id = db
            .create_conversation("gateway", "test-user", None)
            .await
            .expect("create conversation");

        let mut last_user_message_id = None;
        for turn_number in 0..8 {
            let user_message_id = db
                .add_conversation_message(thread_id, "user", &format!("Question {turn_number}"))
                .await
                .expect("add user message");
            db.add_conversation_message(thread_id, "assistant", &format!("Answer {turn_number}"))
                .await
                .expect("add assistant message");
            last_user_message_id = Some((turn_number, user_message_id));
        }

        let (last_turn_number, last_user_message_id) =
            last_user_message_id.expect("final turn metadata");
        db.update_conversation_metadata_field(
            thread_id,
            "live_state",
            &serde_json::json!({
                "turn_number": last_turn_number,
                "user_message_id": last_user_message_id,
                "state": "Processing",
                "user_input": format!("Question {last_turn_number}"),
                "started_at": chrono::Utc::now().to_rfc3339(),
            }),
        )
        .await
        .expect("set stale live_state");

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/chat/history?thread_id={thread_id}&limit=10"))
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("history response json");

        assert!(payload.get("in_progress").is_none());
        let turns = payload["turns"].as_array().expect("turns array");
        assert_eq!(turns.len(), 5);
        assert_eq!(turns.last().expect("last turn")["user_input"], "Question 7");
        assert_eq!(turns.last().expect("last turn")["response"], "Answer 7");
        assert_eq!(turns.last().expect("last turn")["state"], "Completed");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_chat_history_handler_empty_thread_drops_stale_in_progress() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (db, _tmp) = crate::testing::test_db().await;
        let session_manager = Arc::new(SessionManager::new());
        let state =
            test_gateway_state_with_store_and_session_manager(Arc::clone(&db), session_manager);
        let app = Router::new()
            .route("/api/chat/history", get(chat_history_handler))
            .with_state(state);

        let thread_id = db
            .create_conversation("gateway", "test-user", None)
            .await
            .expect("create conversation");
        db.update_conversation_metadata_field(
            thread_id,
            "live_state",
            &serde_json::json!({
                "turn_number": 0,
                "user_message_id": serde_json::Value::Null,
                "state": "Processing",
                "user_input": "Question",
                "started_at": (chrono::Utc::now()
                    - chrono::Duration::minutes(IN_PROGRESS_STALE_AFTER_MINUTES + 1))
                .to_rfc3339(),
            }),
        )
        .await
        .expect("set stale live_state");

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/chat/history?thread_id={thread_id}"))
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test-user".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("history response json");

        assert!(payload.get("in_progress").is_none());
        assert_eq!(payload["turns"].as_array().expect("turns array").len(), 0);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_chat_history_returns_500_when_ownership_lookup_errors() {
        use crate::db::libsql::LibSqlBackend;
        use axum::body::Body;
        use tower::ServiceExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("broken.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create backend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrate backend");
        let conn = backend.connect().await.expect("connect backend");
        conn.execute(
            "ALTER TABLE conversations RENAME TO conversations_broken",
            (),
        )
        .await
        .expect("break ownership lookup");

        let store: Arc<dyn Database> = Arc::new(backend);
        let session_manager = Arc::new(SessionManager::new());
        let state =
            test_gateway_state_with_store_and_session_manager(Arc::clone(&store), session_manager);
        let app = Router::new()
            .route("/api/chat/history", get(chat_history_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/chat/history?thread_id={}", Uuid::new_v4()))
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .expect("body");
        assert_eq!(std::str::from_utf8(&body).unwrap_or(""), "Database error");
    }

    fn history_request(
        state: Arc<GatewayState>,
        user_id: &str,
        thread_id: Uuid,
    ) -> (
        State<Arc<GatewayState>>,
        AuthenticatedUser,
        Query<HistoryQuery>,
    ) {
        (
            State(state),
            AuthenticatedUser(UserIdentity {
                user_id: user_id.to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Query(HistoryQuery {
                thread_id: Some(thread_id.to_string()),
                limit: None,
                before: None,
            }),
        )
    }

    #[tokio::test]
    async fn test_chat_history_returns_engine_v2_messages_for_owner() {
        let _lock = crate::bridge::test_support::ENGINE_STATE_TEST_LOCK
            .lock()
            .await;
        crate::bridge::test_support::clear_engine_state().await;

        let project_id =
            crate::bridge::test_support::install_engine_state_with_threads(Vec::new()).await;
        let mut thread = ironclaw_engine::Thread::new(
            "demo goal",
            ironclaw_engine::ThreadType::Foreground,
            project_id,
            "alice",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread
            .messages
            .push(ironclaw_engine::ThreadMessage::user("hello engine"));
        thread
            .messages
            .push(ironclaw_engine::ThreadMessage::assistant("hi back"));
        let thread_uuid = thread.id.0;
        crate::bridge::test_support::install_engine_state_with_threads(vec![thread]).await;

        let mut state = test_gateway_state_with_dependencies(None, None, None, None);
        Arc::get_mut(&mut state)
            .expect("state should be uniquely owned")
            .session_manager = Some(Arc::new(SessionManager::new()));

        let (s, u, q) = history_request(state, "alice", thread_uuid);
        let response = chat_history_handler(s, u, q).await.expect("history");

        assert_eq!(response.thread_id, thread_uuid);
        assert_eq!(
            response.turns.len(),
            1,
            "one user+assistant pair collapses into a single turn"
        );
        let turn = &response.turns[0];
        assert_eq!(turn.user_input, "hello engine");
        assert_eq!(turn.response.as_deref(), Some("hi back"));
        assert!(!response.has_more);

        crate::bridge::test_support::clear_engine_state().await;
    }

    #[tokio::test]
    async fn test_chat_history_returns_404_for_cross_user_engine_thread() {
        let _lock = crate::bridge::test_support::ENGINE_STATE_TEST_LOCK
            .lock()
            .await;
        crate::bridge::test_support::clear_engine_state().await;

        let project_id =
            crate::bridge::test_support::install_engine_state_with_threads(Vec::new()).await;
        let mut thread = ironclaw_engine::Thread::new(
            "bob's secret",
            ironclaw_engine::ThreadType::Foreground,
            project_id,
            "bob",
            ironclaw_engine::ThreadConfig::default(),
        );
        thread
            .messages
            .push(ironclaw_engine::ThreadMessage::assistant("private reply"));
        let thread_uuid = thread.id.0;
        crate::bridge::test_support::install_engine_state_with_threads(vec![thread]).await;

        let mut state = test_gateway_state_with_dependencies(None, None, None, None);
        Arc::get_mut(&mut state)
            .expect("state should be uniquely owned")
            .session_manager = Some(Arc::new(SessionManager::new()));

        let (s, u, q) = history_request(state, "alice", thread_uuid);
        let result = chat_history_handler(s, u, q).await;

        match result {
            Err((status, _)) => assert_eq!(status, StatusCode::NOT_FOUND),
            Ok(resp) => panic!(
                "alice must not see bob's engine thread but got {} turns",
                resp.turns.len()
            ),
        }

        crate::bridge::test_support::clear_engine_state().await;
    }

    #[tokio::test]
    async fn test_chat_history_accepts_session_owned_thread_without_db() {
        let _lock = crate::bridge::test_support::ENGINE_STATE_TEST_LOCK
            .lock()
            .await;
        // Ensure neither engine state nor v1 DB can claim ownership — the
        // only remaining source must be the in-memory v1 session, which
        // this test exercises.
        crate::bridge::test_support::clear_engine_state().await;

        let session_manager = Arc::new(SessionManager::new());
        let thread_uuid = Uuid::new_v4();
        {
            let session = session_manager.get_or_create_session("alice").await;
            let mut sess = session.lock().await;
            let thread = sess.create_thread_with_id(thread_uuid, Some("web"));
            thread.start_turn("from session");
            thread.conclude_turn(crate::agent::session::TurnOutcome::Completed(
                "session reply".to_string(),
            ));
        }

        let mut state = test_gateway_state_with_dependencies(None, None, None, None);
        Arc::get_mut(&mut state)
            .expect("state should be uniquely owned")
            .session_manager = Some(session_manager);

        let (s, u, q) = history_request(state, "alice", thread_uuid);
        let response = chat_history_handler(s, u, q).await.expect("history");

        assert_eq!(response.thread_id, thread_uuid);
        assert_eq!(response.turns.len(), 1);
        assert_eq!(response.turns[0].user_input, "from session");
        assert_eq!(response.turns[0].response.as_deref(), Some("session reply"));
    }

    fn test_auth_manager(
        tool_registry: Option<Arc<ToolRegistry>>,
    ) -> Arc<crate::bridge::auth_manager::AuthManager> {
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        Arc::new(crate::bridge::auth_manager::AuthManager::new(
            secrets,
            None,
            None,
            tool_registry,
        ))
    }

    #[tokio::test]
    async fn pending_gate_extension_name_uses_install_parameters_for_post_install_auth() {
        let registry = Arc::new(ToolRegistry::new());
        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.tool_registry = Some(Arc::clone(&registry));
        state_mut.auth_manager = Some(test_auth_manager(Some(Arc::clone(&registry))));

        let extension_name = pending_gate_extension_name(
            state_mut,
            "test-user",
            "tool_install",
            r#"{"name":"telegram"}"#,
            &ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::new("telegram_bot_token")
                    .unwrap(),
                instructions: "paste token".to_string(),
                auth_url: None,
            },
        )
        .await;

        assert_eq!(
            extension_name.as_ref().map(|n| n.as_str()),
            Some("telegram")
        );
    }

    #[tokio::test]
    async fn pending_gate_extension_name_uses_install_parameters_for_hyphenated_activate_tool() {
        let state = test_gateway_state(None);

        let extension_name = pending_gate_extension_name(
            &state,
            "test-user",
            "tool-activate",
            r#"{"name":"telegram"}"#,
            &ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::from_trusted(
                    "telegram_bot_token".into(),
                ),
                instructions: "paste token".to_string(),
                auth_url: None,
            },
        )
        .await;

        assert_eq!(
            extension_name.as_ref().map(|n| n.as_str()),
            Some("telegram")
        );
    }

    #[tokio::test]
    async fn pending_gate_extension_name_falls_back_to_provider_extension() {
        struct ProviderTool;

        #[async_trait::async_trait]
        impl Tool for ProviderTool {
            fn name(&self) -> &str {
                "notion_search"
            }

            fn description(&self) -> &str {
                "provider tool"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }

            fn provider_extension(&self) -> Option<&str> {
                Some("notion")
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<ToolOutput, ToolError> {
                unreachable!()
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(ProviderTool)).await;

        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.tool_registry = Some(Arc::clone(&registry));
        state_mut.auth_manager = Some(test_auth_manager(Some(Arc::clone(&registry))));

        let extension_name = pending_gate_extension_name(
            state_mut,
            "test-user",
            "notion_search",
            "{}",
            &ironclaw_engine::ResumeKind::Authentication {
                credential_name: ironclaw_common::CredentialName::new("notion_token").unwrap(),
                instructions: "paste token".to_string(),
                auth_url: None,
            },
        )
        .await;

        assert_eq!(extension_name.as_ref().map(|n| n.as_str()), Some("notion"));
    }

    #[tokio::test]
    async fn test_chat_approval_handler_preserves_user_scoped_metadata() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let state = test_gateway_state(None);
        *state.msg_tx.write().await = Some(tx);

        let app = Router::new()
            .route("/api/chat/approval", post(chat_approval_handler))
            .with_state(state);

        let request_id = Uuid::new_v4();
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/chat/approval")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "request_id": request_id,
                    "action": "approve",
                    "thread_id": "gateway-thread-approval",
                })
                .to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let incoming = rx.recv().await.expect("forwarded approval message");
        assert_eq!(incoming.channel, "gateway");
        assert_eq!(incoming.user_id, "member-1");
        assert_eq!(
            incoming.thread_id.as_deref(),
            Some("gateway-thread-approval")
        );
        assert_eq!(
            incoming.metadata.get("user_id").and_then(|v| v.as_str()),
            Some("member-1")
        );
        assert_eq!(
            incoming.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("gateway-thread-approval")
        );
    }

    #[tokio::test]
    async fn test_chat_auth_token_handler_does_not_forward_secret_through_msg_tx() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let mut state = test_gateway_state(None);
        {
            let state_mut = Arc::get_mut(&mut state).expect("test state uniquely owned");
            state_mut.session_manager = Some(Arc::clone(&session_manager));
        }
        *state.msg_tx.write().await = Some(tx);
        let thread_id = {
            let session = session_manager.get_or_create_session("member-1").await;
            let mut sess = session.lock().await;
            let thread_id = {
                let thread = sess.create_thread(Some("gateway"));
                let thread_id = thread.id;
                thread.enter_auth_mode(ironclaw_common::ExtensionName::new("telegram").unwrap());
                thread_id
            };
            sess.switch_thread(thread_id);
            thread_id
        };

        let app = Router::new()
            .route("/api/chat/auth-token", post(chat_auth_token_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/chat/auth-token")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "token": "secret-token",
                    "thread_id": thread_id,
                })
                .to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        match tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await {
            Err(_) | Ok(None) => {}
            Ok(Some(incoming)) => {
                assert_ne!(incoming.content, "secret-token");
            }
        }
    }

    #[tokio::test]
    async fn test_chat_auth_cancel_handler_clears_requested_thread_auth_mode() {
        use axum::body::Body;
        use tower::ServiceExt;

        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let mut state = test_gateway_state(None);
        Arc::get_mut(&mut state)
            .expect("test state uniquely owned")
            .session_manager = Some(Arc::clone(&session_manager));
        {
            let session = session_manager.get_or_create_session("member-1").await;
            let mut sess = session.lock().await;
            let target_thread_id = Uuid::new_v4();
            let other_thread_id = Uuid::new_v4();
            sess.create_thread_with_id(target_thread_id, Some("gateway"))
                .enter_auth_mode(ironclaw_common::ExtensionName::new("telegram").unwrap());
            sess.create_thread_with_id(other_thread_id, Some("gateway"))
                .enter_auth_mode(ironclaw_common::ExtensionName::new("notion").unwrap());
            sess.switch_thread(other_thread_id);
        }

        let app = Router::new()
            .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
            .with_state(state);

        let target_thread_id = {
            let session = session_manager.get_or_create_session("member-1").await;
            let sess = session.lock().await;
            sess.threads
                .iter()
                .find_map(|(id, thread)| {
                    (thread
                        .pending_auth
                        .as_ref()
                        .map(|p| p.extension_name.as_str())
                        == Some("telegram"))
                    .then_some(*id)
                })
                .expect("telegram pending auth thread")
        };

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/chat/auth-cancel")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "thread_id": target_thread_id,
                })
                .to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let session = session_manager.get_or_create_session("member-1").await;
        let sess = session.lock().await;
        assert!(
            sess.threads
                .get(&target_thread_id)
                .and_then(|thread| thread.pending_auth.as_ref())
                .is_none(),
            "requested thread auth mode should be cleared"
        );
        assert!(
            sess.threads.values().any(|thread| {
                thread
                    .pending_auth
                    .as_ref()
                    .map(|p| p.extension_name.as_str())
                    == Some("notion")
            }),
            "other thread auth mode should remain intact"
        );
    }

    #[tokio::test]
    async fn test_chat_gate_resolve_handler_credential_submission_uses_structured_gate_resolution()
    {
        use axum::body::Body;
        use tower::ServiceExt;

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let state = test_gateway_state(None);
        *state.msg_tx.write().await = Some(tx);

        let app = Router::new()
            .route("/api/chat/gate/resolve", post(chat_gate_resolve_handler))
            .with_state(state);

        let request_id = Uuid::new_v4();
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/chat/gate/resolve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "request_id": request_id,
                    "thread_id": "gateway-thread-auth",
                    "resolution": "credential_provided",
                    "token": "secret-token",
                })
                .to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let incoming = rx.recv().await.expect("forwarded gate resolution");
        let submission = incoming
            .structured_submission
            .clone()
            .expect("structured submission sideband");
        assert!(matches!(
            submission,
            crate::agent::submission::Submission::GateAuthResolution {
                request_id: rid,
                resolution: crate::agent::submission::AuthGateResolution::CredentialProvided { token }
            } if rid == request_id && token == "secret-token"
        ));
        assert_eq!(incoming.content, "[structured auth gate resolution]");
        assert_ne!(incoming.content, "secret-token");
        assert_eq!(incoming.thread_id.as_deref(), Some("gateway-thread-auth"));
        assert_eq!(
            incoming.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("gateway-thread-auth")
        );
    }

    #[tokio::test]
    async fn test_chat_auth_token_handler_expired_auth_broadcasts_failed_onboarding_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        let session_manager = Arc::new(crate::agent::SessionManager::new());
        let mut state = test_gateway_state(None);
        {
            let state_mut = Arc::get_mut(&mut state).expect("test state uniquely owned");
            state_mut.session_manager = Some(Arc::clone(&session_manager));
        }
        let mut receiver = state.sse.sender().subscribe();

        let expected_thread_id = {
            let session = session_manager.get_or_create_session("member-1").await;
            let mut sess = session.lock().await;
            let thread = sess.create_thread(Some("gateway"));
            let thread_id = thread.id;
            thread.pending_auth = Some(crate::agent::session::PendingAuth {
                extension_name: ironclaw_common::ExtensionName::new("telegram").unwrap(),
                created_at: chrono::Utc::now() - chrono::Duration::minutes(16),
            });
            sess.switch_thread(thread_id);
            thread_id
        };
        let expected_thread_id_str = expected_thread_id.to_string();

        let app = Router::new()
            .route("/api/chat/auth-token", post(chat_auth_token_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/chat/auth-token")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "token": "secret-token",
                    "thread_id": expected_thread_id,
                })
                .to_string(),
            ))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                thread_id,
                ..
            } => {
                assert_eq!(extension_name, "telegram");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert_eq!(
                    message.as_deref(),
                    Some("Authentication for 'telegram' expired. Please try again.")
                );
                assert_eq!(thread_id.as_deref(), Some(expected_thread_id_str.as_str()));
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }
}
