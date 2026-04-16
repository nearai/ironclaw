//! Chat handlers: send, approval, gate resolution, auth, SSE events, WebSocket, history, threads,
//! and shared helpers.

use std::sync::Arc;

use crate::channels::IncomingMessage;
use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::handlers::workspaces::{WorkspaceQuery, resolve_requested_workspace_id};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::channels::web::util::{
    build_turns_from_db_messages, collect_generated_images_from_tool_results,
    enforce_generated_image_history_budget, tool_error_for_display, tool_result_preview,
};
use axum::{
    Json,
    extract::{Query, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

// ── Chat send ────────────────────────────────────────────────────────────

pub async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
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

    let workspace_id =
        resolve_requested_workspace_id(&state, &user, workspace_query.workspace.as_deref()).await?;
    let mut msg = IncomingMessage::new("gateway", &user.user_id, &req.content);
    if let Some(workspace_id) = workspace_id {
        msg.workspace_id = Some(workspace_id.to_string());
    }
    // Prefer timezone from JSON body, fall back to X-Timezone header
    let tz = req
        .timezone
        .as_deref()
        .or_else(|| headers.get("X-Timezone").and_then(|v| v.to_str().ok()));
    if let Some(tz) = tz {
        msg = msg.with_timezone(tz);
    }

    // Always include user_id in metadata so downstream SSE broadcasts can scope events.
    let mut meta = serde_json::json!({"user_id": &user.user_id});
    if let Some(workspace_id) = workspace_id {
        meta["workspace_id"] = serde_json::json!(workspace_id);
    }
    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
        meta["thread_id"] = serde_json::json!(thread_id);
    }
    msg = msg.with_metadata(meta);

    // Convert uploaded images to IncomingAttachments
    if !req.images.is_empty() {
        let attachments = crate::channels::web::server::images_to_attachments(&req.images);
        msg = msg.with_attachments(attachments);
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

// ── Approval ─────────────────────────────────────────────────────────────

pub async fn chat_approval_handler(
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

    let mut msg = IncomingMessage::new("gateway", &user.user_id, content);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
        if let Some(ref sm) = state.session_manager
            && let Ok(thread_uuid) = Uuid::parse_str(thread_id)
        {
            let session = sm.get_or_create_session(&user.user_id).await;
            let sess = session.lock().await;
            if let Some(workspace_id) = sess
                .threads
                .get(&thread_uuid)
                .and_then(|thread| thread.pending_approval.as_ref())
                .and_then(|pending| pending.workspace_id.clone())
            {
                msg.workspace_id = Some(workspace_id.clone());
                msg = msg.with_metadata(serde_json::json!({
                    "user_id": &user.user_id,
                    "thread_id": thread_id,
                    "workspace_id": workspace_id,
                }));
            }
        }
    }

    if msg.metadata.is_null() {
        let mut metadata = serde_json::json!({"user_id": &user.user_id});
        if let Some(ref thread_id) = req.thread_id {
            metadata["thread_id"] = serde_json::json!(thread_id);
        }
        msg = msg.with_metadata(metadata);
    }

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

// ── Gate resolution ──────────────────────────────────────────────────────

pub async fn chat_gate_resolve_handler(
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
            dispatch_engine_auth_resolution(&state, &user.user_id, &thread_id, token).await?;
            Ok(Json(ActionResponse::ok("Credential submitted.")))
        }
        GateResolutionPayload::Cancelled => {
            let thread_id = req.thread_id.ok_or((
                StatusCode::BAD_REQUEST,
                "thread_id is required for cancellation".to_string(),
            ))?;
            dispatch_engine_auth_resolution(&state, &user.user_id, &thread_id, "cancel".into())
                .await?;
            Ok(Json(ActionResponse::ok("Gate cancelled.")))
        }
    }
}

// ── Auth token ───────────────────────────────────────────────────────────

/// Submit an auth token directly to the shared auth manager, bypassing the message pipeline.
///
/// The token never touches the LLM, chat history, or SSE stream.
pub async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if let Some(ref thread_id) = req.thread_id
        && crate::bridge::get_engine_pending_auth(&user.user_id, Some(thread_id))
            .await
            .is_some()
    {
        dispatch_engine_auth_resolution(&state, &user.user_id, thread_id, req.token.clone())
            .await?;
        return Ok(Json(ActionResponse::ok("Credential submitted.")));
    }

    let auth_manager = state
        .auth_manager
        .clone()
        .or_else(|| {
            state
                .tool_registry
                .as_ref()
                .and_then(|tr| tr.secrets_store().cloned())
                .or_else(|| state.secrets_store.clone())
                .or_else(|| {
                    state
                        .extension_manager
                        .as_ref()
                        .map(|em| std::sync::Arc::clone(em.secrets()))
                })
                .map(|secrets| {
                    Arc::new(crate::bridge::auth_manager::AuthManager::new(
                        secrets,
                        state.skill_registry.clone(),
                        state.extension_manager.clone(),
                        state.tool_registry.clone(),
                    ))
                })
        })
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Auth manager not available".to_string(),
        ))?;
    let auth_workspace_id =
        resolve_auth_event_workspace_scope(&state, &user, workspace_query.workspace.as_deref())
            .await?;

    match auth_manager
        .submit_auth_token(&req.extension_name, &req.token, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = if result.verification.is_some() || result.activated {
                ActionResponse::ok(result.message.clone())
            } else {
                ActionResponse::fail(result.message.clone())
            };
            resp.activated = Some(result.activated);
            resp.auth_url = result.auth_url.clone();
            resp.verification = result.verification.clone();
            resp.instructions = result.verification.as_ref().map(|v| v.instructions.clone());

            if result.verification.is_some() {
                state.sse.broadcast_for_user_in_workspace(
                    &user.user_id,
                    auth_workspace_id.as_deref(),
                    AppEvent::AuthRequired {
                        extension_name: req.extension_name.clone(),
                        instructions: Some(result.message),
                        auth_url: None,
                        setup_url: None,
                        thread_id: req.thread_id.clone(),
                    },
                );
            } else if result.activated {
                // Clear auth mode on the active thread
                clear_auth_mode(&state, &user.user_id).await;

                state.sse.broadcast_for_user_in_workspace(
                    &user.user_id,
                    auth_workspace_id.as_deref(),
                    AppEvent::AuthCompleted {
                        extension_name: req.extension_name.clone(),
                        success: true,
                        message: result.message,
                        thread_id: req.thread_id.clone(),
                    },
                );
            } else {
                state.sse.broadcast_for_user_in_workspace(
                    &user.user_id,
                    auth_workspace_id.as_deref(),
                    AppEvent::AuthCompleted {
                        extension_name: req.extension_name.clone(),
                        success: false,
                        message: result.message,
                        thread_id: req.thread_id.clone(),
                    },
                );
            }

            Ok(Json(resp))
        }
        Err(e) => {
            let msg = e.to_string();

            // Re-emit auth_required for retry on validation errors
            if matches!(e, crate::extensions::ExtensionError::ValidationFailed(_)) {
                state.sse.broadcast_for_user_in_workspace(
                    &user.user_id,
                    auth_workspace_id.as_deref(),
                    AppEvent::AuthRequired {
                        extension_name: req.extension_name.clone(),
                        instructions: Some(msg.clone()),
                        auth_url: None,
                        setup_url: None,
                        thread_id: req.thread_id.clone(),
                    },
                );
            }
            Ok(Json(ActionResponse::fail(msg)))
        }
    }
}

// ── Auth cancel ──────────────────────────────────────────────────────────

/// Cancel an in-progress auth flow.
pub async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    if let Some(ref thread_id) = req.thread_id
        && crate::bridge::get_engine_pending_auth(&user.user_id, Some(thread_id))
            .await
            .is_some()
    {
        dispatch_engine_auth_resolution(&state, &user.user_id, thread_id, "cancel".into()).await?;
        return Ok(Json(ActionResponse::ok("Auth cancelled")));
    }

    clear_auth_mode(&state, &user.user_id).await;
    // Also clear engine v2 pending auth so the next message isn't consumed as a token.
    crate::bridge::clear_engine_pending_auth(&user.user_id, req.thread_id.as_deref()).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
}

// ── Shared helpers used by server.rs handlers ──────────────────────────

/// Clear pending auth mode on the active thread.
pub(crate) async fn clear_auth_mode(state: &GatewayState, user_id: &str) {
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

pub(crate) async fn active_auth_workspace_scope(
    state: &GatewayState,
    user_id: &str,
) -> Option<String> {
    let session_manager = state.session_manager.as_ref()?;
    let session = session_manager.get_or_create_session(user_id).await;
    let sess = session.lock().await;
    let thread_id = sess.active_thread?;
    sess.threads
        .get(&thread_id)
        .and_then(|thread| thread.pending_auth.as_ref())
        .and_then(|pending| pending.workspace_id.clone())
}

pub(crate) async fn resolve_auth_event_workspace_scope(
    state: &GatewayState,
    user: &crate::channels::web::auth::UserIdentity,
    requested_workspace: Option<&str>,
) -> Result<Option<String>, (StatusCode, String)> {
    let requested_workspace_id = resolve_requested_workspace_id(state, user, requested_workspace)
        .await?
        .map(|id| id.to_string());
    if requested_workspace_id.is_some() {
        return Ok(requested_workspace_id);
    }

    Ok(active_auth_workspace_scope(state, &user.user_id).await)
}

// ── SSE / WebSocket handlers ───────────────────────────────────────────

pub async fn chat_events_handler(
    Query(params): Query<ChatEventsQuery>,
    headers: HeaderMap,
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let workspace_scope =
        resolve_requested_workspace_id(&state, &user, workspace_query.workspace.as_deref())
            .await?
            .map(|id| id.to_string());
    let sse = state
        .sse
        .subscribe_scoped(
            Some(user.user_id),
            workspace_scope,
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

#[derive(Debug, Deserialize, Default)]
pub struct ChatEventsQuery {
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

pub async fn chat_ws_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
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
    let workspace_id =
        resolve_requested_workspace_id(&state, &user, workspace_query.workspace.as_deref())
            .await?
            .map(|id| id.to_string());
    Ok(ws.on_upgrade(move |socket| {
        crate::channels::web::ws::handle_ws_connection(socket, state, user, workspace_id)
    }))
}

// ── Thread management and history handlers ────────────────────────────

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub thread_id: Option<String>,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub workspace: Option<String>,
}

async fn engine_pending_gate_info(
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    let pending = crate::bridge::get_engine_pending_gate(user_id, thread_id)
        .await
        .ok()??;
    Some(PendingGateInfo {
        request_id: pending.request_id,
        thread_id: pending.thread_id.to_string(),
        gate_name: pending.gate_name,
        tool_name: pending.tool_name,
        description: pending.description,
        parameters: pending.parameters,
        resume_kind: serde_json::to_value(pending.resume_kind).unwrap_or_default(),
    })
}

async fn history_pending_gate_info(
    user_id: &str,
    thread_id: Option<&str>,
) -> Option<PendingGateInfo> {
    if thread_id.is_some() {
        // Thread-scoped pending gates are authoritative once the client sends a
        // thread_id. The unscoped fallback only exists for legacy callers that
        // do not know which thread owns the gate yet.
        return engine_pending_gate_info(user_id, thread_id).await;
    }
    engine_pending_gate_info(user_id, None).await
}

async fn dispatch_engine_auth_resolution(
    state: &GatewayState,
    user_id: &str,
    thread_id: &str,
    content: String,
) -> Result<(), (StatusCode, String)> {
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

    let msg = IncomingMessage::new("gateway", user_id, content).with_thread(thread_id.to_string());

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })
}

pub(crate) fn turn_info_from_in_memory_turn(t: &crate::agent::session::Turn) -> TurnInfo {
    TurnInfo {
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
                result_preview: tool_result_preview(tc.result.as_ref()),
                error: tc.error.as_deref().map(tool_error_for_display),
                rationale: tc.rationale.clone(),
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

pub async fn chat_history_handler(
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
    let workspace_id =
        resolve_requested_workspace_id(&state, &user, query.workspace.as_deref()).await?;
    // In-memory threads are already scoped by user via session_manager, but DB
    // lookups could expose another user's conversation if the UUID is guessed.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &user.user_id, workspace_id)
            .await
            .map_err(|e| {
                tracing::error!(thread_id = %thread_id, error = %e, "DB error during thread ownership check");
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error".to_string())
            })?;
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
            pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
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

        let pending_gate = history_pending_gate_info(&user.user_id, thread_scope)
            .await
            .or_else(|| {
                thread.pending_approval.as_ref().map(|pa| PendingGateInfo {
                    request_id: pa.request_id.to_string(),
                    thread_id: thread_id.to_string(),
                    gate_name: "approval".into(),
                    tool_name: pa.tool_name.clone(),
                    description: pa.description.clone(),
                    parameters: serde_json::to_string_pretty(&pa.parameters).unwrap_or_default(),
                    resume_kind: serde_json::json!({"Approval":{"allow_always":true}}),
                })
            });

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
            pending_gate,
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
            enforce_generated_image_history_budget(&mut turns);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
                pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
        pending_gate: history_pending_gate_info(&user.user_id, thread_scope).await,
    }))
}

pub async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&user.user_id).await;
    let sess = session.lock().await;

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        let workspace_id =
            resolve_requested_workspace_id(&state, &user, workspace_query.workspace.as_deref())
                .await?;
        // Auto-create assistant thread if it doesn't exist
        let assistant_id = store
            .get_or_create_assistant_conversation(&user.user_id, workspace_id, "gateway")
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        match store
            .list_conversations_all_channels(&user.user_id, workspace_id, 50)
            .await
        {
            Ok(summaries) => {
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

                return Ok(Json(ThreadListResponse {
                    assistant_thread,
                    threads,
                    active_thread: sess.active_thread,
                }));
            }
            Err(e) => {
                tracing::error!(user_id = %user.user_id, error = %e, "DB error listing threads; falling back to in-memory");
            }
        }
    }

    // Fallback: in-memory only (no assistant thread without DB)
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

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

pub async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(workspace_query): Query<WorkspaceQuery>,
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
        let workspace_id =
            resolve_requested_workspace_id(&state, &user, workspace_query.workspace.as_deref())
                .await?;
        match store
            .ensure_conversation(
                thread_id,
                "gateway",
                &user.user_id,
                workspace_id,
                None,
                Some("gateway"),
            )
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
}
