//! Feature handlers for the web gateway that have not yet migrated to
//! domain modules under `handlers/` or `features/<slice>/`.
//!
//! The platform-level pieces (shared state, rate limiters, CSP/static
//! serving, route composition, `start_server`) live under
//! `crate::channels::web::platform::*`. This file re-exports them so
//! existing `crate::channels::web::server::*` call sites continue to
//! resolve while the ironclaw#2599 migration is in progress.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::types::*;
use crate::channels::web::util::{
    build_turns_from_db_messages, collect_generated_images_from_tool_results,
    enforce_generated_image_history_budget, tool_error_for_display, tool_result_preview,
    web_incoming_message,
};

// --- Backward-compat re-exports for the ironclaw#2599 migration ---
//
// The platform-level state types, rate limiters, and `start_server` moved
// to `crate::channels::web::platform::*`. External callers (handlers,
// integration tests, `src/main.rs`, `src/app.rs`) still reach them via
// `crate::channels::web::server::*`; re-export until follow-up PRs update
// every call site.
pub use crate::channels::web::platform::router::start_server;
pub(crate) use crate::channels::web::platform::state::rate_limit_key_from_headers;
pub use crate::channels::web::platform::state::{
    ActiveConfigSnapshot, FrontendCacheKey, FrontendHtmlCache, GatewayState, PerUserRateLimiter,
    PromptQueue, RateLimiter, RoutineEngineSlot, WorkspacePool,
};

// --- Chat handlers ---

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

    // Convert uploaded images to IncomingAttachments
    if !req.images.is_empty() {
        let attachments = crate::channels::web::util::images_to_attachments(&req.images);
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

/// Check whether an Origin header value points to a local address.
///
/// Extracts the host from the origin (handling both IPv4/hostname and IPv6
/// literal formats) and compares it against known local addresses. Used to
/// prevent cross-site WebSocket hijacking while allowing localhost access.
fn is_local_origin(origin: &str) -> bool {
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
        crate::channels::web::ws::handle_ws_connection(socket, state, user)
    }))
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
}

async fn pending_gate_extension_name(
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

const IN_PROGRESS_STALE_AFTER_MINUTES: i64 = 10;

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

// Job handlers moved to handlers/jobs.rs
// Logs handlers moved to features/logs/

// --- Extension handlers ---

pub(crate) async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut owner_bound_channels = std::collections::HashSet::new();
    let mut paired_channels = std::collections::HashSet::new();
    for ext in &installed {
        if ext.kind == crate::extensions::ExtensionKind::WasmChannel {
            if ext_mgr.has_wasm_channel_owner_binding(&ext.name).await {
                owner_bound_channels.insert(ext.name.clone());
            }
            if ext_mgr.has_wasm_channel_pairing(&ext.name).await {
                paired_channels.insert(ext.name.clone());
            }
        }
    }
    let extensions = installed
        .into_iter()
        .map(|ext| {
            let activation_status =
                crate::channels::web::handlers::extensions::derive_activation_status(
                    &ext,
                    paired_channels.contains(&ext.name),
                    owner_bound_channels.contains(&ext.name),
                );
            let (onboarding_state, onboarding) =
                crate::channels::web::handlers::extensions::derive_onboarding(
                    &ext.name,
                    activation_status,
                );
            ExtensionInfo {
                name: ext.name,
                display_name: ext.display_name,
                kind: ext.kind.to_string(),
                description: ext.description,
                url: ext.url,
                authenticated: ext.authenticated,
                active: ext.active,
                tools: ext.tools,
                needs_setup: ext.needs_setup,
                has_auth: ext.has_auth,
                activation_status,
                activation_error: ext.activation_error,
                version: ext.version,
                onboarding_state,
                onboarding,
            }
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

fn extension_phase_for_web(
    ext: &crate::extensions::InstalledExtension,
) -> crate::extensions::ExtensionPhase {
    if ext.activation_error.is_some() {
        crate::extensions::ExtensionPhase::Error
    } else if ext.needs_setup {
        crate::extensions::ExtensionPhase::NeedsSetup
    } else if ext.has_auth && !ext.authenticated {
        crate::extensions::ExtensionPhase::NeedsAuth
    } else if ext.active
        || matches!(
            ext.kind,
            crate::extensions::ExtensionKind::WasmChannel
                | crate::extensions::ExtensionKind::ChannelRelay
        )
    {
        crate::extensions::ExtensionPhase::Ready
    } else {
        crate::extensions::ExtensionPhase::NeedsActivation
    }
}

pub(crate) async fn extensions_readiness_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ExtensionReadinessResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let extensions = installed
        .into_iter()
        .map(|ext| {
            let phase = match extension_phase_for_web(&ext) {
                crate::extensions::ExtensionPhase::Installed => "installed",
                crate::extensions::ExtensionPhase::NeedsSetup => "needs_setup",
                crate::extensions::ExtensionPhase::NeedsAuth => "needs_auth",
                crate::extensions::ExtensionPhase::NeedsActivation => "needs_activation",
                crate::extensions::ExtensionPhase::Activating => "activating",
                crate::extensions::ExtensionPhase::Ready => "ready",
                crate::extensions::ExtensionPhase::Error => "error",
            }
            .to_string();
            ExtensionReadinessInfo {
                name: ext.name,
                kind: ext.kind.to_string(),
                phase,
                authenticated: ext.authenticated,
                active: ext.active,
                activation_error: ext.activation_error,
            }
        })
        .collect();

    Ok(Json(ExtensionReadinessResponse { extensions }))
}

pub(crate) async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

pub(crate) async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // When extension manager isn't available, check registry entries for a helpful message
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        // Look up the entry in the catalog to give a specific error
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{}' requires building from source. \
                         Run `ironclaw registry install {}` from the CLI.",
                        req.name, req.name
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). \
                     Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
                    req.name
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        "acp_agent" => Some(crate::extensions::ExtensionKind::AcpAgent),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint, &user.user_id)
        .await
    {
        Ok(result) => {
            let mut resp = ActionResponse::ok(result.message);
            match ext_mgr
                .ensure_extension_ready(
                    &req.name,
                    &user.user_id,
                    crate::extensions::EnsureReadyIntent::PostInstall,
                )
                .await
            {
                Ok(readiness) => apply_extension_readiness_to_response(&mut resp, readiness, true),
                Err(e) => {
                    tracing::debug!(
                        extension = %req.name,
                        error = %e,
                        "Post-install readiness follow-through failed"
                    );
                }
            }

            Ok(Json(resp))
        }
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

fn apply_extension_readiness_to_response(
    resp: &mut ActionResponse,
    readiness: crate::extensions::EnsureReadyOutcome,
    preserve_success: bool,
) {
    match readiness {
        crate::extensions::EnsureReadyOutcome::Ready { activation, .. } => {
            if let Some(activation) = activation {
                resp.message = activation.message;
                resp.activated = Some(true);
            }
        }
        crate::extensions::EnsureReadyOutcome::NeedsAuth { auth, .. } => {
            let fallback = format!("'{}' requires authentication.", auth.name);
            if !preserve_success {
                resp.success = false;
                resp.message = auth
                    .instructions()
                    .map(String::from)
                    .unwrap_or_else(|| fallback.clone());
            } else if let Some(instructions) = auth.instructions() {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.auth_url = auth.auth_url().map(String::from);
            resp.awaiting_token = Some(auth.is_awaiting_token());
            resp.instructions = auth.instructions().map(String::from);
        }
        crate::extensions::EnsureReadyOutcome::NeedsSetup {
            instructions,
            setup_url,
            ..
        } => {
            if !preserve_success {
                resp.success = false;
                resp.message = instructions.clone();
            } else {
                resp.message = format!("{}. {}", resp.message, instructions);
            }
            resp.instructions = Some(instructions);
            resp.auth_url = setup_url;
        }
    }
}

pub(crate) async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // The URL path segment is user input — validate at the boundary via
    // `ExtensionName::new` and use the canonical form for all downstream
    // extension-manager calls and response formatting.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    tracing::trace!(
        extension = %name.as_str(),
        user_id = %user.user_id,
        "extensions_activate_handler: received activate request"
    );
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr
        .ensure_extension_ready(
            name.as_str(),
            &user.user_id,
            crate::extensions::EnsureReadyIntent::ExplicitActivate,
        )
        .await
    {
        Ok(readiness) => {
            let mut resp = ActionResponse::ok(format!("Extension '{}' is ready.", name.as_str()));
            apply_extension_readiness_to_response(&mut resp, readiness, false);
            Ok(Json(resp))
        }
        Err(err) => Ok(Json(ActionResponse::fail(err.to_string()))),
    }
}

pub(crate) async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // Validate user-controlled path segment before it reaches the extension
    // manager — rejects path-traversal, invalid characters, and malformed
    // slugs with a 400.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(name.as_str(), &user.user_id).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

pub(crate) async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter registry entries by query (or return all if empty)
    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    // Cross-reference with installed extensions by (name, kind) to avoid
    // false positives when the same name exists as different kinds.
    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
            ext_mgr
                .list(None, false, &user.user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|ext| (ext.name, ext.kind.to_string()))
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
                version: e.version.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

pub(crate) async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    // Validate user-controlled path segment at entry. Downstream lookups
    // (`get_setup_schema`, `list().find(...)`) consume the canonical form.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let setup = ext_mgr
        .get_setup_schema(name.as_str(), &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false, &user.user_id)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name.as_str()))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name: name.as_str().to_string(),
        kind,
        secrets: setup.secrets,
        fields: setup.fields,
        onboarding_state: None,
        onboarding: None,
    }))
}

pub(crate) async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    // The URL path segment is user input — validate at the boundary via
    // `ExtensionName::new`. Reject path-traversal, invalid characters, or
    // malformed slugs with a 400 before the value reaches extension
    // lookup, SSE broadcast, or any `from_trusted` wrap below.
    let name = ironclaw_common::ExtensionName::new(&name).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid extension name: {e}"),
        )
    })?;

    // Clear auth mode regardless of outcome so the next user message goes
    // through to the LLM instead of being intercepted as a token.
    crate::channels::web::platform::legacy_auth::clear_auth_mode(&state, &user.user_id).await;

    match ext_mgr
        .configure(name.as_str(), &req.secrets, &req.fields, &user.user_id)
        .await
    {
        Ok(result) => {
            // Return ok when activated OR when an OAuth auth_url is present
            // (activation is expected to be false until OAuth completes).
            let mut resp = if result.activated || result.auth_url.is_some() {
                ActionResponse::ok(result.message.clone())
            } else {
                ActionResponse::fail(result.message.clone())
            };
            resp.activated = Some(result.activated);
            resp.auth_url = result.auth_url.clone();
            resp.onboarding_state = result.onboarding_state;
            resp.onboarding = result.onboarding.clone();
            let outcome = crate::channels::web::onboarding::classify_configure_result(&result);
            let mut onboarding_event =
                crate::channels::web::onboarding::event_from_configure_result(
                    name.clone(),
                    &result,
                    req.thread_id.clone(),
                );
            if let (Some(request_id), Some(thread_id)) =
                (req.request_id.as_deref(), req.thread_id.as_deref())
            {
                match outcome {
                    crate::channels::web::onboarding::ConfigureFlowOutcome::AuthRequired => {}
                    crate::channels::web::onboarding::ConfigureFlowOutcome::PairingRequired {
                        instructions,
                        onboarding,
                    } => {
                        let request_id = Uuid::parse_str(request_id).map_err(|_| {
                            (
                                StatusCode::BAD_REQUEST,
                                "Invalid request_id (expected UUID)".to_string(),
                            )
                        })?;
                        if let Some(next_request_id) =
                            crate::bridge::transition_engine_pending_auth_request_to_pairing(
                                &user.user_id,
                                request_id,
                                Some(thread_id),
                                name.as_str(),
                            )
                            .await
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                        {
                            onboarding_event =
                                crate::channels::web::types::OnboardingStateDto::pairing_required(
                                    name.clone(),
                                    Some(next_request_id),
                                    Some(thread_id.to_string()),
                                    Some(result.message.clone()),
                                    instructions,
                                    onboarding,
                                );
                        }
                    }
                    crate::channels::web::onboarding::ConfigureFlowOutcome::Ready => {
                        crate::channels::web::platform::engine_dispatch::dispatch_engine_external_callback(
                            &state,
                            &user.user_id,
                            thread_id,
                            request_id,
                        )
                        .await?;
                    }
                    crate::channels::web::onboarding::ConfigureFlowOutcome::RetryAuth => {}
                }
            }
            // Broadcast the canonical onboarding state so the chat UI can
            // dismiss or advance any in-progress onboarding UI.
            state
                .sse
                .broadcast_for_user(&user.user_id, onboarding_event);
            Ok(Json(resp))
        }
        Err(e) => {
            // Preserve the `activated` field on the failure path so clients
            // (and regression tests) see an explicit `false` rather than
            // `null`. `ActionResponse::fail` leaves `activated` as `None`,
            // which serializes to `null` and makes "did activation fail?"
            // ambiguous from the wire.
            let mut resp = ActionResponse::fail(e.to_string());
            resp.activated = Some(false);
            Ok(Json(resp))
        }
    }
}

// Pairing handlers moved to features/pairing/

pub(crate) async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    // Verify ownership before listing runs.
    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if routine.user_id != user.user_id {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: run.status.to_string(),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
            job_id: run.job_id,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

// Gateway status handler moved to features/status/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::SessionManager;
    use crate::auth::oauth;
    use crate::channels::relay::DEFAULT_RELAY_NAME;
    use crate::channels::web::auth::{CombinedAuthState, UserIdentity};
    use crate::channels::web::features::oauth::{
        oauth_callback_handler, slack_relay_oauth_callback_handler,
    };
    use crate::channels::web::features::pairing::{pairing_approve_handler, pairing_list_handler};
    use crate::channels::web::handlers::llm::{
        llm_list_models_handler, llm_test_connection_handler,
    };
    use crate::channels::web::platform::router::start_server;
    use crate::channels::web::platform::static_files::{
        BASE_CSP_HEADER, build_csp, build_csp_with_nonce, build_frontend_html, css_etag,
        css_handler, generate_csp_nonce, stamp_nonce_into_html,
    };
    use crate::channels::web::sse::SseManager;
    use crate::channels::web::types::{
        ExtensionActivationStatus, classify_wasm_channel_activation,
    };
    use crate::db::Database;
    use crate::extensions::{ExtensionKind, ExtensionManager, InstalledExtension};
    use crate::testing::credentials::TEST_GATEWAY_CRYPTO_KEY;
    use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
    use crate::workspace::Workspace;
    use axum::Router;
    use axum::http::header;
    use axum::routing::{get, post};
    use ironclaw_gateway::{NONCE_PLACEHOLDER, assets};

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
    fn test_build_turns_from_db_messages_empty() {
        let turns = build_turns_from_db_messages(&[]);
        assert!(turns.is_empty());
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn workspace_pool_resolve_seeds_new_user_workspace() {
        let (db, _dir) = crate::testing::test_db().await;
        let pool = WorkspacePool::new(
            db,
            None,
            crate::workspace::EmbeddingCacheConfig::default(),
            crate::config::WorkspaceSearchConfig::default(),
            crate::config::WorkspaceConfig::default(),
        );

        let ws = crate::tools::builtin::memory::WorkspaceResolver::resolve(&pool, "alice").await;

        let readme = ws.read(crate::workspace::paths::README).await.unwrap();
        let identity = ws.read(crate::workspace::paths::IDENTITY).await.unwrap();

        assert!(!readme.content.trim().is_empty());
        assert!(!identity.content.trim().is_empty());
    }

    #[test]
    fn test_wasm_channel_activation_status_owner_bound_counts_as_active() -> Result<(), String> {
        let ext = InstalledExtension {
            name: "telegram".to_string(),
            kind: ExtensionKind::WasmChannel,
            display_name: Some("Telegram".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: true,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let owner_bound = classify_wasm_channel_activation(&ext, false, true);
        if owner_bound != Some(ExtensionActivationStatus::Active) {
            return Err(format!(
                "owner-bound channel should be active, got {:?}",
                owner_bound
            ));
        }

        let unbound = classify_wasm_channel_activation(&ext, false, false);
        if unbound != Some(ExtensionActivationStatus::Pairing) {
            return Err(format!(
                "unbound channel should be pairing, got {:?}",
                unbound
            ));
        }

        Ok(())
    }

    #[test]
    fn test_channel_relay_activation_status_is_preserved() -> Result<(), String> {
        let relay = InstalledExtension {
            name: "signal".to_string(),
            kind: ExtensionKind::ChannelRelay,
            display_name: Some("Signal".to_string()),
            description: None,
            url: None,
            authenticated: true,
            active: false,
            tools: Vec::new(),
            needs_setup: true,
            has_auth: false,
            installed: true,
            activation_error: None,
            version: None,
        };

        let status = if relay.kind == crate::extensions::ExtensionKind::WasmChannel {
            classify_wasm_channel_activation(&relay, false, false)
        } else if relay.kind == crate::extensions::ExtensionKind::ChannelRelay {
            Some(if relay.active {
                ExtensionActivationStatus::Active
            } else if relay.authenticated {
                ExtensionActivationStatus::Configured
            } else {
                ExtensionActivationStatus::Installed
            })
        } else {
            None
        };

        if status != Some(ExtensionActivationStatus::Configured) {
            return Err(format!(
                "channel relay should retain configured status, got {:?}",
                status
            ));
        }

        Ok(())
    }

    // --- OAuth callback handler tests ---

    /// Build a minimal `GatewayState` for handler tests.
    fn test_gateway_state_with_dependencies(
        ext_mgr: Option<Arc<ExtensionManager>>,
        store: Option<Arc<dyn Database>>,
        db_auth: Option<Arc<crate::channels::web::auth::DbAuthenticator>>,
        pairing_store: Option<Arc<crate::pairing::PairingStore>>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: ext_mgr,
            tool_registry: None,
            store,
            settings_cache: None,
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: vec![],
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth,
            pairing_store,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
    }

    fn test_gateway_state(ext_mgr: Option<Arc<ExtensionManager>>) -> Arc<GatewayState> {
        test_gateway_state_with_dependencies(ext_mgr, None, None, None)
    }

    fn test_gateway_state_with_store_and_session_manager(
        store: Arc<dyn Database>,
        session_manager: Arc<SessionManager>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: Some(session_manager),
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(store),
            settings_cache: None,
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: vec![],
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: ActiveConfigSnapshot::default(),
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
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
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

    /// Build a minimal `AuthManager` backed by an in-memory secrets store.
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

    /// Build a test router with just the OAuth callback route.
    fn test_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route("/oauth/callback", get(oauth_callback_handler))
            .with_state(state)
    }

    #[cfg(feature = "libsql")]
    async fn insert_test_user(db: &Arc<dyn Database>, id: &str, role: &str) {
        db.get_or_create_user(crate::db::UserRecord {
            id: id.to_string(),
            role: role.to_string(),
            display_name: id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("create test user");
    }

    #[cfg(feature = "libsql")]
    async fn make_pairing_test_state() -> (
        Arc<GatewayState>,
        Arc<dyn Database>,
        Arc<crate::pairing::PairingStore>,
        tempfile::TempDir,
    ) {
        let (db, tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        insert_test_user(&db, "member-1", "member").await;
        let pairing_store = Arc::new(crate::pairing::PairingStore::new(
            Arc::clone(&db),
            Arc::new(crate::ownership::OwnershipCache::new()),
        ));
        let state = test_gateway_state_with_dependencies(
            None,
            Some(Arc::clone(&db)),
            None,
            Some(Arc::clone(&pairing_store)),
        );
        (state, db, pairing_store, tmp)
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_list_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        pairing_store
            .upsert_request("telegram", "tg-user-1", None)
            .await
            .expect("create pairing request");

        let app = Router::new()
            .route("/api/pairing/{channel}", get(pairing_list_handler))
            .with_state(state);

        let mut member_req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/pairing/telegram")
            .body(Body::empty())
            .expect("member request");
        member_req.extensions_mut().insert(UserIdentity {
            user_id: "member-1".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let member_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), member_req)
            .await
            .expect("member response");
        assert_eq!(member_resp.status(), StatusCode::FORBIDDEN);

        let mut admin_req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/pairing/telegram")
            .body(Body::empty())
            .expect("admin request");
        admin_req.extensions_mut().insert(UserIdentity {
            user_id: "admin-1".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let admin_resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, admin_req)
            .await
            .expect("admin response");
        assert_eq!(admin_resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(admin_resp.into_body(), 1024 * 64)
            .await
            .expect("admin body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("pairing list json");
        assert_eq!(
            parsed["channel"],
            serde_json::Value::String("telegram".to_string())
        );
        assert_eq!(parsed["requests"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            parsed["requests"][0]["sender_id"],
            serde_json::Value::String("tg-user-1".to_string())
        );
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
            incoming.thread_id.as_ref().map(|t| t.as_str()),
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
        assert_eq!(
            incoming.thread_id.as_ref().map(|t| t.as_str()),
            Some("gateway-thread-auth")
        );
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_claims_code_for_authenticated_user() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        let request = pairing_store
            .upsert_request("telegram", "tg-user-claim", None)
            .await
            .expect("create pairing request");

        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "code": request.code.to_ascii_lowercase() }).to_string(),
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
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["success"], serde_json::Value::Bool(true));

        let identity = pairing_store
            .resolve_identity("telegram", "tg-user-claim")
            .await
            .expect("resolve identity")
            .expect("claimed identity");
        assert_eq!(identity.owner_id.as_str(), "member-1");
        assert!(
            pairing_store
                .list_pending("telegram")
                .await
                .expect("pending list")
                .is_empty()
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_does_not_inject_followup_agent_turn_without_thread() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        let request = pairing_store
            .upsert_request("telegram", "tg-user-no-followup", None)
            .await
            .expect("create pairing request");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "code": request.code }).to_string(),
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

        let recv = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(
            !matches!(recv, Ok(Some(_))),
            "pairing approval should not inject a synthetic gateway follow-up turn"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_injects_ready_followup_for_active_thread() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        let request = pairing_store
            .upsert_request("telegram", "tg-user-followup", None)
            .await
            .expect("create pairing request");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let thread_id = "gateway-thread-123";
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "code": request.code, "thread_id": thread_id }).to_string(),
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

        let followup = tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
            .await
            .expect("follow-up timeout")
            .expect("follow-up message");
        assert_eq!(followup.channel, "gateway");
        assert_eq!(followup.user_id, "member-1");
        assert_eq!(
            followup.thread_id.as_ref().map(|t| t.as_str()),
            Some(thread_id)
        );
        assert!(
            followup
                .content
                .contains("onboarding for 'telegram' is now fully complete and ready"),
            "unexpected follow-up content: {}",
            followup.content
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_dispatches_external_callback_for_pairing_gate_request() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, pairing_store, _tmp) = make_pairing_test_state().await;
        let request = pairing_store
            .upsert_request("telegram", "tg-user-gate-followup", None)
            .await
            .expect("create pairing request");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let request_id = Uuid::new_v4();
        let thread_id = "gateway-thread-456";
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "code": request.code,
                    "thread_id": thread_id,
                    "request_id": request_id,
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

        let callback = tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
            .await
            .expect("callback timeout")
            .expect("callback message");
        let submission = callback
            .structured_submission
            .clone()
            .expect("structured submission sideband");
        assert!(matches!(
            submission,
            crate::agent::submission::Submission::ExternalCallback { request_id: rid }
                if rid == request_id
        ));
        assert_eq!(callback.content, "[structured external callback]");
        assert_eq!(
            callback.thread_id.as_ref().map(|t| t.as_str()),
            Some(thread_id)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_pairing_approve_rejects_blank_code() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (state, _db, _pairing_store, _tmp) = make_pairing_test_state().await;
        let app = Router::new()
            .route(
                "/api/pairing/{channel}/approve",
                post(pairing_approve_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/pairing/telegram/approve")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({ "code": "   " }).to_string()))
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
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(parsed["success"], serde_json::Value::Bool(false));
        assert_eq!(
            parsed["message"],
            serde_json::Value::String("Pairing code is required.".to_string())
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_delete_user_evicts_auth_and_pairing_caches() {
        use axum::body::Body;
        use tower::ServiceExt;

        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        insert_test_user(&db, "member-1", "member").await;

        let token = "member-token-123";
        let hash = crate::channels::web::auth::hash_token(token);
        db.create_api_token("member-1", "test-token", &hash, &token[..8], None) // safety: test-only, ASCII literal
            .await
            .expect("create api token");

        let db_auth = Arc::new(crate::channels::web::auth::DbAuthenticator::new(
            Arc::clone(&db),
        ));
        let pairing_store = Arc::new(crate::pairing::PairingStore::new(
            Arc::clone(&db),
            Arc::new(crate::ownership::OwnershipCache::new()),
        ));

        let auth_identity = db_auth
            .authenticate(token)
            .await
            .expect("db auth lookup")
            .expect("db auth identity");
        assert_eq!(auth_identity.user_id, "member-1");

        let request = pairing_store
            .upsert_request("telegram", "tg-delete-1", None)
            .await
            .expect("create pairing request");
        pairing_store
            .approve(
                "telegram",
                &request.code,
                &crate::ownership::OwnerId::from("member-1"),
            )
            .await
            .expect("approve pairing");
        assert!(
            pairing_store
                .resolve_identity("telegram", "tg-delete-1")
                .await
                .expect("prime pairing cache")
                .is_some()
        );

        let state = test_gateway_state_with_dependencies(
            None,
            Some(Arc::clone(&db)),
            Some(Arc::clone(&db_auth)),
            Some(Arc::clone(&pairing_store)),
        );
        let app = Router::new()
            .route(
                "/api/admin/users/{id}",
                axum::routing::delete(crate::channels::web::handlers::users::users_delete_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("DELETE")
            .uri("/api/admin/users/member-1")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "admin-1".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(
            db_auth
                .authenticate(token)
                .await
                .expect("post-delete auth lookup")
                .is_none()
        );
        assert!(
            pairing_store
                .resolve_identity("telegram", "tg-delete-1")
                .await
                .expect("post-delete pairing lookup")
                .is_none()
        );
    }

    #[derive(Clone, Debug)]
    struct RecordedOauthProxyRequest {
        authorization: Option<String>,
        form: std::collections::HashMap<String, String>,
    }

    #[derive(Clone)]
    struct MockOauthProxyState {
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
    }

    struct MockOauthProxyServer {
        addr: std::net::SocketAddr,
        requests: Arc<tokio::sync::Mutex<Vec<RecordedOauthProxyRequest>>>,
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        server_task: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockOauthProxyServer {
        async fn start() -> Self {
            async fn exchange_handler(
                State(state): State<MockOauthProxyState>,
                headers: axum::http::HeaderMap,
                axum::Form(form): axum::Form<std::collections::HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                state.requests.lock().await.push(RecordedOauthProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(serde_json::json!({
                    "access_token": "proxy-access-token",
                    "refresh_token": "proxy-refresh-token",
                    "expires_in": 7200
                }))
            }

            let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock oauth proxy");
            let addr = listener.local_addr().expect("mock oauth proxy addr");
            let app = Router::new()
                .route("/oauth/exchange", post(exchange_handler))
                .with_state(MockOauthProxyState {
                    requests: Arc::clone(&requests),
                });
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });

            Self {
                addr,
                requests,
                shutdown_tx: Some(shutdown_tx),
                server_task: Some(server_task),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn requests(&self) -> Vec<RecordedOauthProxyRequest> {
            self.requests.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                let _ = task.await;
            }
        }
    }

    impl Drop for MockOauthProxyServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                task.abort();
            }
        }
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: Tests use lock_env() to serialize environment access.
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let original = std::env::var(key).ok();
        // SAFETY: Tests use lock_env() to serialize environment access.
        unsafe {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        EnvVarGuard { key, original }
    }

    fn fresh_pending_oauth_flow(
        secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync>,
        sse_manager: Option<Arc<SseManager>>,
        oauth_proxy_auth_token: Option<String>,
    ) -> crate::auth::oauth::PendingOAuthFlow {
        crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: Some("test-code-verifier".to_string()),
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: Some("google".to_string()),
            validation_endpoint: None,
            scopes: vec!["email".to_string()],
            user_id: "test".to_string(),
            secrets,
            sse_manager,
            gateway_token: oauth_proxy_auth_token,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at: std::time::Instant::now(),
            auto_activate_extension: true,
        }
    }

    /// Regression for the PR #2617 review (Gemini HIGH/security): the
    /// `extensions_setup_submit_handler` used to wrap the URL path segment
    /// in `ExtensionName::from_trusted`, skipping the newtype's path-
    /// traversal and invalid-character rejection. A handler-level test (not
    /// an `identity.rs`-level test) locks in the boundary: a malformed path
    /// must produce a 400 before the value reaches any downstream
    /// `from_trusted` wrap, extension lookup, or SSE broadcast.
    #[tokio::test]
    async fn test_extensions_setup_submit_rejects_path_traversal_name() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        // Each of these slugs would have silently reached extension lookup
        // under the old `from_trusted(name)` wrap. All must reject at 400.
        // We use axum::http::uri::PathAndQuery-safe escape where needed so
        // the path extractor still decodes into a valid `String`.
        for bad in [
            "..%2Ftraversal",
            "slash%2Fname",
            "BadCase",
            "has%20space",
            "trailing_",
        ] {
            let req_body = serde_json::json!({"secrets": {}});
            let mut req = axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/extensions/{bad}/setup"))
                .header("content-type", "application/json")
                .body(Body::from(req_body.to_string()))
                .expect("request");
            req.extensions_mut().insert(UserIdentity {
                user_id: "test".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            });

            let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
                .await
                .expect("response");
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "expected 400 for malformed extension name {bad:?}, got {:?}",
                resp.status()
            );
        }
    }

    /// Regression for the PR #2617 Copilot review: the sibling
    /// `/api/extensions/{name}/...` handlers (`activate`, `remove`, setup GET)
    /// used to accept `Path<String>` and hand it straight to the extension
    /// manager, leaving path-traversal / malformed slugs unvalidated at the
    /// web boundary. All three must now reject at 400 before any downstream
    /// lookup — same guarantee as `extensions_setup_submit_handler`.
    #[tokio::test]
    async fn test_extensions_sibling_handlers_reject_path_traversal_name() {
        use axum::body::Body;
        use axum::routing::{get, post};
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/activate",
                post(extensions_activate_handler),
            )
            .route(
                "/api/extensions/{name}/remove",
                post(extensions_remove_handler),
            )
            .route(
                "/api/extensions/{name}/setup",
                get(extensions_setup_handler),
            )
            .with_state(state);

        let bad_names = [
            "..%2Ftraversal",
            "slash%2Fname",
            "BadCase",
            "has%20space",
            "trailing_",
        ];
        let routes = [("POST", "activate"), ("POST", "remove"), ("GET", "setup")];

        for bad in bad_names {
            for (method, suffix) in routes {
                let mut builder = axum::http::Request::builder()
                    .method(method)
                    .uri(format!("/api/extensions/{bad}/{suffix}"));
                if method == "POST" {
                    builder = builder.header("content-type", "application/json");
                }
                let body = if method == "POST" {
                    Body::from("{}")
                } else {
                    Body::empty()
                };
                let mut req = builder.body(body).expect("request");
                req.extensions_mut().insert(UserIdentity {
                    user_id: "test".to_string(),
                    role: "admin".to_string(),
                    workspace_read_scopes: Vec::new(),
                });

                let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
                    .await
                    .expect("response");
                assert_eq!(
                    resp.status(),
                    StatusCode::BAD_REQUEST,
                    "expected 400 for {method} {suffix} with malformed name {bad:?}, got {:?}",
                    resp.status()
                );
            }
        }
    }

    #[tokio::test]
    async fn test_extensions_setup_submit_returns_failure_when_not_activated() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);

        // Use underscore-only name: `canonicalize_extension_name` rewrites
        // hyphens to underscores, but `configure`'s capabilities-file lookup
        // does not fall back to the legacy hyphen form, so a hyphenated test
        // channel name causes `Capabilities file not found` and the handler
        // takes the `Err` branch (no `activated` field) instead of the
        // intended "saved but activation failed" branch.
        let channel_name = "test_failing_channel";
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

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/{name}/setup",
                post(extensions_setup_submit_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "secrets": {
                "BOT_TOKEN": "dummy-token"
            }
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(format!("/api/extensions/{channel_name}/setup"))
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        // Inject AuthenticatedUser so the handler's extractor succeeds
        // without needing the full auth middleware layer.
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        assert_eq!(parsed["success"], serde_json::Value::Bool(false));
        assert_eq!(parsed["activated"], serde_json::Value::Bool(false));
        assert!(
            parsed["message"]
                .as_str()
                .unwrap_or_default()
                .contains("Activation failed"),
            "expected activation failure in message: {:?}",
            parsed
        );
    }

    #[tokio::test]
    async fn test_extensions_list_reports_installed_inactive_wasm_channel_as_inactive() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);
        let channel_name = "telegram";
        std::fs::write(
            wasm_channels_dir
                .path()
                .join(format!("{channel_name}.wasm")),
            b"\0asm fake",
        )
        .expect("write fake wasm");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions", get(extensions_list_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == channel_name))
            .expect("telegram extension entry");
        assert_eq!(telegram["kind"], "wasm_channel");
        assert_eq!(telegram["active"], false);
        assert_eq!(telegram["authenticated"], false);
        assert_eq!(telegram["activation_status"], "installed");
    }

    #[test]
    fn test_extension_phase_for_web_prefers_error_then_readiness() {
        let mut ext = crate::extensions::InstalledExtension {
            name: "notion".to_string(),
            kind: crate::extensions::ExtensionKind::McpServer,
            display_name: None,
            description: None,
            url: None,
            authenticated: false,
            active: false,
            tools: Vec::new(),
            needs_setup: false,
            has_auth: true,
            installed: true,
            activation_error: Some("boom".to_string()),
            version: None,
        };
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Error
        );

        ext.activation_error = None;
        ext.needs_setup = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsSetup
        );

        ext.needs_setup = false;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsAuth
        );

        ext.authenticated = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::NeedsActivation
        );

        ext.active = true;
        assert_eq!(
            extension_phase_for_web(&ext),
            crate::extensions::ExtensionPhase::Ready
        );
    }

    #[tokio::test]
    async fn test_extensions_readiness_handler_reports_phase_summary() {
        use axum::body::Body;
        use tower::ServiceExt;

        // DB-backed manager so the install path does not fall back to the
        // developer's real `~/.ironclaw/mcp-servers.json` (which would
        // panic with `AlreadyInstalled("notion")` on dev machines that
        // already have a notion entry configured).
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir, _db_dir) = test_ext_mgr_with_db().await;
        let mut server =
            crate::tools::mcp::McpServerConfig::new("notion", "https://mcp.notion.com/mcp");
        server.description = Some("Notion".to_string());
        ext_mgr
            .install(
                "notion",
                Some(&server.url),
                Some(crate::extensions::ExtensionKind::McpServer),
                "test",
            )
            .await
            .expect("install notion mcp");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route(
                "/api/extensions/readiness",
                get(extensions_readiness_handler),
            )
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions/readiness")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let notion = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "notion"))
            .expect("notion readiness entry");
        assert_eq!(notion["kind"], "mcp_server");
        assert_eq!(notion["phase"], "needs_auth");
        assert_eq!(notion["authenticated"], false);
        assert_eq!(notion["active"], false);
    }

    #[tokio::test]
    async fn test_extensions_list_handler_reports_installed_inactive_wasm_channel_as_inactive() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, wasm_channels_dir) = test_ext_mgr(secrets);
        std::fs::write(wasm_channels_dir.path().join("telegram.wasm"), b"fake-wasm")
            .expect("write fake telegram wasm");
        std::fs::write(
            wasm_channels_dir.path().join("telegram.capabilities.json"),
            serde_json::json!({
                "type": "channel",
                "name": "telegram",
                "description": "Telegram",
                "capabilities": {
                    "channel": {
                        "allowed_paths": ["/webhook/telegram"]
                    }
                }
            })
            .to_string(),
        )
        .expect("write telegram capabilities");

        let state = test_gateway_state(Some(ext_mgr));
        let app = Router::new()
            .route("/api/extensions", get(extensions_list_handler))
            .with_state(state);

        let mut req = axum::http::Request::builder()
            .method("GET")
            .uri("/api/extensions")
            .body(Body::empty())
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        let telegram = parsed["extensions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["name"] == "telegram"))
            .expect("telegram extensions entry");

        assert_eq!(telegram["kind"], "wasm_channel");
        assert_eq!(telegram["active"], false);
        assert_eq!(telegram["activation_status"], "installed");
    }

    #[tokio::test]
    async fn test_llm_test_connection_allows_admin_private_base_url() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route(
                "/api/llm/test_connection",
                post(llm_test_connection_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1",
            "model": "test-model"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/test_connection")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "test".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&body).expect("json response");
        assert_eq!(parsed["ok"], serde_json::Value::Bool(false));
        let message = parsed["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains("Invalid base URL"),
            "private localhost endpoint should pass validation: {message}"
        );
    }

    #[tokio::test]
    async fn test_llm_test_connection_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route(
                "/api/llm/test_connection",
                post(llm_test_connection_handler),
            )
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1",
            "model": "test-model"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/test_connection")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_llm_list_models_requires_admin_role() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = Router::new()
            .route("/api/llm/list_models", post(llm_list_models_handler))
            .with_state(state);

        let req_body = serde_json::json!({
            "adapter": "openai",
            "base_url": "http://127.0.0.1:9/v1"
        });
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/llm/list_models")
            .header("content-type", "application/json")
            .body(Body::from(req_body.to_string()))
            .expect("request");
        req.extensions_mut().insert(UserIdentity {
            user_id: "member".to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        });

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    fn expired_flow_created_at() -> Option<std::time::Instant> {
        std::time::Instant::now()
            .checked_sub(oauth::OAUTH_FLOW_EXPIRY + std::time::Duration::from_secs(1))
    }

    #[test]
    fn apply_extension_readiness_preserves_install_success_for_auth_followup() {
        let mut resp = ActionResponse::ok("Installed notion");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_authorization(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "https://example.com/oauth".to_string(),
                    "gateway".to_string(),
                ),
            },
            true,
        );

        assert!(resp.success);
        assert_eq!(resp.auth_url.as_deref(), Some("https://example.com/oauth"));
        assert_eq!(resp.awaiting_token, Some(false));
    }

    #[test]
    fn apply_extension_readiness_fails_activate_when_auth_is_required() {
        let mut resp = ActionResponse::ok("placeholder");
        apply_extension_readiness_to_response(
            &mut resp,
            crate::extensions::EnsureReadyOutcome::NeedsAuth {
                name: "notion".to_string(),
                kind: crate::extensions::ExtensionKind::McpServer,
                phase: crate::extensions::ExtensionPhase::NeedsAuth,
                credential_name: Some("notion_api_token".to_string()),
                auth: crate::extensions::AuthResult::awaiting_token(
                    "notion",
                    crate::extensions::ExtensionKind::McpServer,
                    "Paste your Notion token".to_string(),
                    None,
                ),
            },
            false,
        );

        assert!(!resp.success);
        assert_eq!(resp.awaiting_token, Some(true));
        assert_eq!(
            resp.instructions.as_deref(),
            Some("Paste your Notion token")
        );
        assert_eq!(resp.message, "Paste your Notion token");
    }

    #[tokio::test]
    async fn test_csp_header_present_on_responses() {
        use std::net::SocketAddr;

        let state = test_gateway_state(None);

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let auth = CombinedAuthState::from(crate::channels::web::auth::MultiAuthState::single(
            "test-token".to_string(),
            "test".to_string(),
        ));
        let bound = start_server(addr, state.clone(), auth)
            .await
            .expect("server should start");

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/api/health", bound))
            .send()
            .await
            .expect("health request should succeed");

        assert_eq!(resp.status(), 200);

        let csp = resp
            .headers()
            .get("content-security-policy")
            .expect("CSP header must be present");

        let csp_str = csp.to_str().expect("CSP header should be valid UTF-8");
        assert!(
            csp_str.contains("default-src 'self'"),
            "CSP must contain default-src"
        );
        assert!(
            csp_str.contains(
                "script-src 'self' https://cdn.jsdelivr.net https://cdnjs.cloudflare.com https://esm.sh"
            ),
            "CSP must allow the explicit script CDNs without unsafe-inline"
        );
        assert!(
            csp_str.contains("object-src 'none'"),
            "CSP must contain object-src 'none'"
        );
        assert!(
            csp_str.contains("frame-ancestors 'none'"),
            "CSP must contain frame-ancestors 'none'"
        );

        if let Some(tx) = state.shutdown_tx.write().await.take() {
            let _ = tx.send(());
        }
    }

    #[test]
    fn test_base_and_nonce_csp_agree_outside_script_src() {
        // Regression for the drift risk flagged in PR #1725 review: the
        // static header and the per-response nonce header must share every
        // directive except `script-src`. Build both, strip `script-src …;`
        // from each, and assert the remaining policy is byte-identical.
        let base = build_csp(None);
        let nonce = build_csp(Some("feedc0de"));

        fn strip_script_src(csp: &str) -> String {
            // Directives are separated by `; `. Drop the one that starts
            // with `script-src` and rejoin the rest.
            csp.split("; ")
                .filter(|d| !d.trim_start().starts_with("script-src"))
                .collect::<Vec<_>>()
                .join("; ")
        }

        assert_eq!(
            strip_script_src(&base),
            strip_script_src(&nonce),
            "base CSP and nonce CSP must agree on every directive except script-src\n\
             base:  {base}\n\
             nonce: {nonce}"
        );
    }

    #[test]
    fn test_base_csp_header_matches_build_csp_none() {
        // The lazy static header used by the response-header layer must be
        // byte-identical to `build_csp(None)`. If the fallback branch of
        // the LazyLock ever fires, the header would regress to
        // `default-src 'self'` and this test would catch it.
        let lazy = BASE_CSP_HEADER.to_str().expect("static CSP is ASCII");
        assert_eq!(lazy, build_csp(None));
    }

    #[test]
    fn test_build_csp_with_nonce_includes_nonce_source() {
        // Per-response CSP must add `'nonce-…'` to script-src so a single
        // inline `<script nonce="…">` block is authorized for that response.
        let csp = build_csp_with_nonce("deadbeefcafebabe");
        assert!(
            csp.contains("script-src 'self' 'nonce-deadbeefcafebabe' https://cdn.jsdelivr.net"),
            "nonce source must appear immediately after 'self' in script-src; got: {csp}"
        );
        // The other directives must match the static BASE_CSP so the
        // per-response value never accidentally relaxes anything else.
        for needle in [
            "default-src 'self'",
            "style-src 'self' 'unsafe-inline'",
            "object-src 'none'",
            "frame-ancestors 'none'",
            "base-uri 'self'",
        ] {
            assert!(csp.contains(needle), "missing directive: {needle}");
        }
        // And it must NOT contain `'unsafe-inline'` for scripts.
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "script-src must not allow 'unsafe-inline'"
        );
    }

    #[test]
    fn test_generate_csp_nonce_is_unique_and_hex() {
        let a = generate_csp_nonce();
        let b = generate_csp_nonce();
        assert_eq!(a.len(), 32, "16 bytes hex-encoded should be 32 chars");
        assert_ne!(a, b, "nonces must be unique per call");
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit()),
            "nonce must be lowercase hex"
        );
    }

    #[test]
    fn test_css_etag_is_strong_validator_format() {
        // Strong validators are double-quoted (no `W/` prefix). The
        // sha-prefix lets future readers identify the digest function at a
        // glance, and 16 hex chars (64 bits) is plenty for content-address
        // collision avoidance on a single-tenant CSS payload.
        let etag = css_etag("body { color: red; }");
        assert!(etag.starts_with("\"sha256-"));
        assert!(etag.ends_with('"'));
        assert!(!etag.starts_with("W/"));
        // Header value must be ASCII so it can land in a `HeaderValue`.
        assert!(etag.is_ascii());
    }

    #[test]
    fn test_css_etag_changes_when_body_changes() {
        // The whole point of the ETag: editing `custom.css` must produce
        // a new validator so the browser fetches the updated body.
        let base = css_etag("body { color: red; }");
        let edited = css_etag("body { color: blue; }");
        assert_ne!(base, edited);
        // Adding even a single byte must invalidate.
        let appended = css_etag("body { color: red; } ");
        assert_ne!(base, appended);
    }

    #[test]
    fn test_css_etag_stable_for_identical_body() {
        // Two requests against the same assembled body must produce the
        // same validator — otherwise every request misses the cache.
        let body = "body { color: red; }";
        assert_eq!(css_etag(body), css_etag(body));
    }

    #[tokio::test]
    async fn test_css_handler_returns_etag_and_serves_304_on_match() {
        use axum::body::Body;
        use tower::ServiceExt;

        // Pure-static path: no workspace overlay, so the body is exactly
        // the embedded `STYLE_CSS`. Cheap and deterministic.
        let state = test_gateway_state(None);
        let app = Router::new()
            .route("/style.css", get(css_handler))
            .with_state(state);

        // First request: 200 with ETag header.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let etag = resp
            .headers()
            .get(header::ETAG)
            .expect("ETag header must be present on 200")
            .to_str()
            .expect("ETag is ASCII")
            .to_string();
        assert!(etag.starts_with("\"sha256-"));

        // Second request with `If-None-Match` matching the validator: 304
        // and an empty body. The browser keeps its cached copy.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app.clone(), req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
        let body = axum::body::to_bytes(resp.into_body(), 1024)
            .await
            .expect("body");
        assert!(body.is_empty(), "304 must have an empty body");

        // Third request with a stale validator: 200 again. Operators
        // expect this when `custom.css` changes underneath them — the
        // browser revalidates, sees the body shifted, and fetches anew.
        let req = axum::http::Request::builder()
            .uri("/style.css")
            .header(header::IF_NONE_MATCH, "\"sha256-0000000000000000\"")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Multi-tenant safety symmetry: in multi-user mode the CSS handler
    /// must mirror `build_frontend_html` and refuse to layer
    /// `.system/gateway/custom.css` from `state.workspace`. The
    /// `/style.css` route is unauthenticated bootstrap, so there is no
    /// user identity at request time — reading the global workspace
    /// would leak one operator's `custom.css` to every other tenant.
    ///
    /// The bait here is a global workspace seeded with hostile-looking
    /// custom CSS. If `css_handler` ever stops short-circuiting on
    /// `workspace_pool.is_some()`, the bait would land in the response
    /// body and this test would fail loudly with the leaked content.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_css_handler_returns_base_in_multi_tenant_mode() {
        use axum::body::Body;
        use tower::ServiceExt;

        use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use crate::workspace::EmbeddingCacheConfig;

        let dir = tempfile::tempdir().expect("tempdir");
        let backend = LibSqlBackend::new_local(&dir.path().join("multi_tenant_css.db"))
            .await
            .expect("backend");
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        // Bait: a global workspace with a hostile-looking custom.css.
        // If css_handler ever reads state.workspace in multi-tenant
        // mode, the marker would leak into the response body and this
        // test would fail with an actionable diagnostic.
        let global_ws = Arc::new(Workspace::new_with_db("tenant-leak-bait", Arc::clone(&db)));
        global_ws
            .write(
                ".system/gateway/custom.css",
                "body { background: #ff0000; } /* TENANT-LEAK-BAIT */",
            )
            .await
            .expect("seed bait custom.css");

        let pool = Arc::new(WorkspacePool::new(
            Arc::clone(&db),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));

        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.workspace = Some(global_ws);
        state_mut.workspace_pool = Some(pool);

        let app = Router::new()
            .route("/style.css", get(css_handler))
            .with_state(state);

        let req = axum::http::Request::builder()
            .uri("/style.css")
            .body(Body::empty())
            .expect("request");
        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let body_str = String::from_utf8_lossy(&body);

        // Contract 1: the bait marker is absent. If a future regression
        // re-reads state.workspace in multi-tenant mode, the marker
        // would land here and this assertion fails with the leaked
        // content visible in the diagnostic.
        assert!(
            !body_str.contains("TENANT-LEAK-BAIT"),
            "custom.css from global workspace leaked into multi-tenant /style.css \
             response — css_handler is missing its workspace_pool guard"
        );

        // Contract 2: the response is exactly the embedded base
        // stylesheet, byte-for-byte. This catches a subtler regression
        // where the leak content is dropped but the multi-tenant path
        // still does the owned `format!` (turning what should be a
        // borrowed hot-path response into an allocation).
        assert_eq!(
            body_str.as_ref(),
            assets::STYLE_CSS,
            "multi-tenant /style.css must serve the embedded base stylesheet \
             unchanged — no overlay, no allocation"
        );
    }

    #[test]
    fn test_stamp_nonce_into_html_replaces_attribute() {
        // Vanilla case: a placeholder inside a `nonce="…"` attribute on
        // a script tag must be substituted with the real nonce. Both
        // the layout-config script and any widget script tags emitted
        // by `assemble_index` carry the same attribute shape, so a
        // single test covers every emission point.
        let html = format!("<script nonce=\"{NONCE_PLACEHOLDER}\">window.X = 1;</script>");
        let stamped = stamp_nonce_into_html(&html, "deadbeef");
        assert!(
            stamped.contains("nonce=\"deadbeef\""),
            "real nonce attribute must be present after substitution: {stamped}"
        );
        assert!(
            !stamped.contains(NONCE_PLACEHOLDER),
            "placeholder must be gone after substitution: {stamped}"
        );
    }

    #[test]
    fn test_stamp_nonce_into_html_does_not_mutate_widget_body() {
        // Regression for the PR #1725 Copilot finding: a bare-string
        // replace would also rewrite any *body content* that happens to
        // contain the literal sentinel — e.g. a widget JS module that
        // mentions `__IRONCLAW_CSP_NONCE__` in a comment, log line, or
        // string constant. The attribute-targeted replace must leave
        // those untouched.
        //
        // Build a fragment with TWO sentinels: one inside the
        // legitimate `nonce="…"` attribute (must be replaced) and one
        // inside the script body as a string constant (must NOT be
        // replaced).
        let html = format!(
            "<script type=\"module\" nonce=\"{NONCE_PLACEHOLDER}\">\n\
             // hostile widget body — author writes the sentinel as a constant\n\
             const SENTINEL = \"{NONCE_PLACEHOLDER}\";\n\
             console.log(SENTINEL);\n\
             </script>"
        );
        let stamped = stamp_nonce_into_html(&html, "cafebabe");

        // Contract 1: the attribute was rewritten.
        assert!(
            stamped.contains("nonce=\"cafebabe\""),
            "attribute must carry the per-response nonce: {stamped}"
        );

        // Contract 2: the body sentinel survived intact. The widget
        // author's source must round-trip byte-for-byte.
        assert!(
            stamped.contains(&format!("const SENTINEL = \"{NONCE_PLACEHOLDER}\"")),
            "widget body sentinel must NOT be rewritten: {stamped}"
        );

        // Contract 3: exactly one occurrence of the placeholder remains
        // (the one in the body). If a future regression switches to a
        // bare-string replace, this count would drop to 0 and the test
        // would fail loudly with the diff.
        assert_eq!(
            stamped.matches(NONCE_PLACEHOLDER).count(),
            1,
            "exactly one placeholder occurrence (in widget body) must \
             survive; the attribute one must be replaced. Got: {stamped}"
        );
    }

    /// Multi-tenant cache safety: when `workspace_pool` is set,
    /// `build_frontend_html` must refuse the assembly path entirely and
    /// return `None` regardless of what `state.workspace` contains.
    ///
    /// Background: `index_handler` (`GET /`) is the unauthenticated
    /// bootstrap route, so it has no user identity at request time.
    /// Reading `state.workspace` in multi-tenant mode would expose one
    /// global workspace's customizations to every user, and the
    /// process-wide `frontend_html_cache` would pin the leak across
    /// requests. The bait here is a global workspace seeded with a
    /// hostile-looking layout — if the function ever stops short-
    /// circuiting on `workspace_pool.is_some()`, that layout would land
    /// in the assembled HTML and this test would fail loudly.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_build_frontend_html_returns_none_in_multi_tenant_mode() {
        use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
        use crate::db::Database as _;
        use crate::db::libsql::LibSqlBackend;
        use crate::workspace::EmbeddingCacheConfig;

        let dir = tempfile::tempdir().expect("tempdir");
        let backend = LibSqlBackend::new_local(&dir.path().join("multi_tenant_index.db"))
            .await
            .expect("backend");
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        // Bait: a *global* workspace with customizations. If
        // build_frontend_html ever read state.workspace in multi-tenant
        // mode, the title "TENANT-LEAK-BAIT" would appear in the
        // assembled HTML for every user. The assertions below pin the
        // refusal contract — both the return value AND the cache slot.
        let global_ws = Arc::new(Workspace::new_with_db("tenant-leak-bait", Arc::clone(&db)));
        global_ws
            .write(
                ".system/gateway/layout.json",
                r#"{"branding":{"title":"TENANT-LEAK-BAIT"}}"#,
            )
            .await
            .expect("seed bait layout");

        let pool = Arc::new(WorkspacePool::new(
            Arc::clone(&db),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));

        // Build state via the standard test helper, then mutate the
        // workspace + workspace_pool fields. `Arc::get_mut` succeeds here
        // because no other strong reference exists yet — the helper just
        // returned the freshly-constructed Arc.
        let mut state = test_gateway_state(None);
        let state_mut = Arc::get_mut(&mut state).expect("test state must be uniquely owned");
        state_mut.workspace = Some(global_ws);
        state_mut.workspace_pool = Some(pool);

        // Contract 1: build_frontend_html refuses to assemble.
        let html = build_frontend_html(&state).await;
        assert!(
            html.is_none(),
            "build_frontend_html must return None in multi-tenant mode \
             (got Some HTML — bait layout may have leaked across tenants)"
        );

        // Contract 2: the cache slot is still empty. The early return
        // above MUST short-circuit before the cache write at the bottom
        // of the function — otherwise a poisoned cache entry would serve
        // the leaked HTML to subsequent requests even after the bug is
        // fixed.
        let cache = state.frontend_html_cache.read().await;
        assert!(
            cache.is_none(),
            "frontend_html_cache must remain empty in multi-tenant mode"
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_missing_params() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_error_from_provider() {
        use axum::body::Body;
        use tower::ServiceExt;

        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?error=access_denied&error_description=access_denied")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_unknown_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        // Build an ExtensionManager so the handler can look up flows
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=unknown_state_value")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping expired OAuth flow test: monotonic uptime below expiry window");
            return;
        };

        // Insert an expired flow.
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Expired flow → error landing page
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_expired_flow_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping expired OAuth flow SSE test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: Some(sse_mgr),
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("expired_state".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr));
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=expired_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert_eq!(
                    message.as_deref(),
                    Some("OAuth flow expired. Please try again.")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }

    #[tokio::test]
    async fn test_oauth_callback_no_extension_manager() {
        use axum::body::Body;
        use tower::ServiceExt;

        // No extension manager set → graceful error
        let state = test_gateway_state(None);
        let app = test_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=test_code&state=some_state")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
    }

    #[tokio::test]
    async fn test_oauth_callback_strips_instance_prefix() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        // Insert a flow keyed by raw nonce "test_nonce" (without instance prefix).
        // Use an expired flow so the handler exits before attempting a real HTTP
        // token exchange — we only need to verify that the instance prefix was
        // stripped and the flow was found by the raw nonce.
        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping OAuth state-prefix test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            // Expired — handler will reject after lookup (no network I/O)
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);

        // Send callback with instance prefix: "myinstance:test_nonce"
        // The handler should strip "myinstance:" and find the flow keyed by "test_nonce"
        let req = axum::http::Request::builder()
            .uri("/oauth/callback?code=fake_code&state=myinstance:test_nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);

        // The flow was found (stripped prefix matched) but is expired, so the
        // handler returns an error landing page. The flow being consumed from
        // the registry (checked below) proves the prefix was stripped correctly.
        assert!(
            html.contains("Authorization Failed"),
            "Expected error page, html was: {}",
            &html[..html.len().min(500)]
        );

        // Verify the flow was consumed (removed from registry)
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let Some(created_at) = expired_flow_created_at() else {
            eprintln!("Skipping versioned OAuth state test: monotonic uptime below expiry window");
            return;
        };
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_oauth_callback_accepts_versioned_hosted_state_without_instance_name() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(crate::secrets::InMemorySecretsStore::new(Arc::new(
                crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                    TEST_GATEWAY_CRYPTO_KEY.to_string(),
                ))
                .expect("crypto"),
            )));
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());

        let Some(created_at) = expired_flow_created_at() else {
            eprintln!(
                "Skipping versioned OAuth state without instance test: monotonic uptime below expiry window"
            );
            return;
        };
        let flow = crate::auth::oauth::PendingOAuthFlow {
            extension_name: ironclaw_common::ExtensionName::new("test_tool").unwrap(),
            display_name: "Test Tool".to_string(),
            token_url: "https://example.com/token".to_string(),
            client_id: "client123".to_string(),
            client_secret: None,
            redirect_uri: "https://example.com/oauth/callback".to_string(),
            code_verifier: None,
            access_token_field: "access_token".to_string(),
            secret_name: "test_token".to_string(),
            provider: None,
            validation_endpoint: None,
            scopes: vec![],
            user_id: "test".to_string(),
            secrets,
            sse_manager: None,
            gateway_token: None,
            token_exchange_extra_params: std::collections::HashMap::new(),
            client_id_secret_name: None,
            client_secret_secret_name: None,
            client_secret_expires_at: None,
            created_at,
            auto_activate_extension: true,
        };

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Authorization Failed"));
        assert!(
            ext_mgr
                .pending_oauth_flows()
                .read()
                .await
                .get("test_nonce")
                .is_none()
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_gateway_token_fallback() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_with_dedicated_proxy_auth_token() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        // Keep the process-wide env locked for the full callback so the handler
        // sees a stable proxy URL/token configuration throughout the test.
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var(
            "IRONCLAW_OAUTH_PROXY_AUTH_TOKEN",
            Some("shared-oauth-proxy-secret"),
        );
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", None);

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state = crate::auth::oauth::encode_hosted_oauth_state("test_nonce", None);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(html.contains("Test Tool Connected"));

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer shared-oauth-proxy-secret")
        );
        assert_eq!(
            requests[0].form.get("code").map(String::as_str),
            Some("fake_code")
        );
        assert_eq!(
            requests[0].form.get("code_verifier").map(String::as_str),
            Some("test-code-verifier")
        );

        let access_token = secrets
            .get_decrypted("test", "test_token")
            .await
            .expect("access token stored");
        assert_eq!(access_token.expose(), "proxy-access-token");

        let refresh_token = secrets
            .get_decrypted("test", "test_token_refresh_token")
            .await
            .expect("refresh token stored");
        assert_eq!(refresh_token.expose(), "proxy-refresh-token");

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_happy_path_without_auto_activation() {
        use axum::body::Body;
        use tower::ServiceExt;

        let proxy = MockOauthProxyServer::start().await;
        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some(&proxy.base_url()));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let mut flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );
        flow.auto_activate_extension = false;

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Ready
                );
                assert_eq!(
                    message.as_deref(),
                    Some("Test Tool authenticated successfully")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }

        proxy.shutdown().await;
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn test_oauth_callback_exchange_failure_broadcasts_auth_completed_failure() {
        use axum::body::Body;
        use tower::ServiceExt;

        let _env_guard = crate::config::helpers::lock_env();
        let _exchange_url_guard =
            set_env_var("IRONCLAW_OAUTH_EXCHANGE_URL", Some("http://127.0.0.1:1"));
        let _proxy_auth_guard = set_env_var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN", None);
        let _gateway_token_guard = set_env_var("GATEWAY_AUTH_TOKEN", Some("gateway-test-token"));

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(Arc::clone(&secrets));
        let sse_mgr = Arc::new(SseManager::new());
        let mut receiver = sse_mgr.sender().subscribe();
        let flow = fresh_pending_oauth_flow(
            Arc::clone(&secrets),
            Some(Arc::clone(&sse_mgr)),
            crate::auth::oauth::oauth_proxy_auth_token(),
        );

        ext_mgr
            .pending_oauth_flows()
            .write()
            .await
            .insert("test_nonce".to_string(), flow);

        let state = test_gateway_state(Some(ext_mgr.clone()));
        let app = test_oauth_router(state);
        let versioned_state =
            crate::auth::oauth::encode_hosted_oauth_state("test_nonce", Some("myinstance"));

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/callback?code=fake_code&state={}",
                urlencoding::encode(&versioned_state)
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK);

        match receiver.recv().await.expect("onboarding_state event").event {
            crate::channels::web::types::AppEvent::OnboardingState {
                extension_name,
                state,
                message,
                ..
            } => {
                assert_eq!(extension_name, "test_tool");
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::Failed
                );
                assert!(
                    message
                        .as_deref()
                        .unwrap_or_default()
                        .contains("authentication failed")
                );
            }
            event => panic!("expected OnboardingState event, got {event:?}"),
        }
    }

    // --- Slack relay OAuth CSRF tests ---

    fn test_relay_oauth_router(state: Arc<GatewayState>) -> Router {
        Router::new()
            .route(
                "/oauth/slack/callback",
                get(slack_relay_oauth_callback_handler),
            )
            .with_state(state)
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
    ) -> (Arc<ExtensionManager>, tempfile::TempDir, tempfile::TempDir) {
        let tool_registry = Arc::new(ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let ext_mgr = Arc::new(ExtensionManager::new(
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

    /// DB-backed `ExtensionManager` for tests that exercise MCP install/list
    /// paths.
    ///
    /// `test_ext_mgr` builds the manager with `store: None`, which makes
    /// `load_mcp_servers` fall back to the file-based path
    /// `~/.ironclaw/mcp-servers.json`. Any test that calls `install` for an
    /// MCP server with `store: None` will read the developer's real config
    /// and may panic with `AlreadyInstalled("notion")` (or similar) on
    /// machines that have configured MCP servers locally.
    ///
    /// This sibling builds an isolated in-memory libsql DB AND pre-seeds
    /// an empty `mcp_servers` setting for the test user so that
    /// `load_mcp_servers_from_db` does not silently fall back to disk
    /// (it falls back when the DB has no entry, see `mcp/config.rs:625`).
    async fn test_ext_mgr_with_db() -> (
        Arc<ExtensionManager>,
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let mcp_sm = Arc::new(crate::tools::mcp::session::McpSessionManager::new());
        let mcp_pm = Arc::new(crate::tools::mcp::process::McpProcessManager::new());
        let wasm_tools_dir = tempfile::tempdir().expect("temp wasm tools dir");
        let wasm_channels_dir = tempfile::tempdir().expect("temp wasm channels dir");
        let (db, db_dir) = crate::testing::test_db().await;

        // Pre-seed an empty servers list so the DB-backed loader does not
        // fall back to `~/.ironclaw/mcp-servers.json` on dev machines.
        let empty_servers = crate::tools::mcp::config::McpServersFile::default();
        crate::tools::mcp::config::save_mcp_servers_to_db(db.as_ref(), "test", &empty_servers)
            .await
            .expect("seed empty mcp_servers setting");

        let ext_mgr = Arc::new(ExtensionManager::new(
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
            Some(db),
            vec![],
        ));
        (ext_mgr, wasm_tools_dir, wasm_channels_dir, db_dir)
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_missing_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback without state param should be rejected
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error, got: {}",
            &html[..html.len().min(300)]
        );
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_wrong_state_param() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();

        // Store a valid nonce
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    "correct-nonce-value",
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with wrong state param
        let req = axum::http::Request::builder()
            .uri("/oauth/slack/callback?team_id=T123&provider=slack&state=wrong-nonce")
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Expected CSRF error for wrong nonce, got: {}",
            &html[..html.len().min(300)]
        );

        let state_key = format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(false);
        assert!(exists, "Wrong nonce must not consume the stored CSRF nonce");
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_correct_canonical_state_proceeds() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "valid-test-nonce-12345";
        let relay_name = crate::extensions::naming::canonicalize_extension_name(DEFAULT_RELAY_NAME)
            .expect("canonical relay name");

        // Store the correct nonce under the canonical extension name used by
        // install/auth/activate flows (`slack_relay`).
        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", relay_name),
                    nonce,
                ),
            )
            .await
            .expect("store nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // Callback with correct state param — will pass CSRF check
        // but may fail downstream (no real relay service) — that's OK,
        // we just verify it doesn't return a CSRF error.
        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        // Should NOT contain the CSRF error message
        assert!(
            !html.contains("Invalid or expired authorization"),
            "Should have passed CSRF check, got: {}",
            &html[..html.len().min(300)]
        );

        // Verify the nonce was consumed (deleted)
        let state_key = format!("relay:{}:oauth_state", relay_name);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(true);
        assert!(!exists, "CSRF nonce should be deleted after use");
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_legacy_state_proceeds_and_is_consumed() {
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "legacy-test-nonce-12345";
        let relay_name = crate::extensions::naming::canonicalize_extension_name(DEFAULT_RELAY_NAME)
            .expect("canonical relay name");
        let legacy_relay_name = crate::extensions::naming::legacy_extension_alias(&relay_name)
            .expect("legacy relay alias");

        secrets
            .create(
                "test",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", legacy_relay_name),
                    nonce,
                ),
            )
            .await
            .expect("store legacy nonce");

        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets.clone());
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            !html.contains("Invalid or expired authorization"),
            "Should have passed CSRF check with legacy nonce, got: {}",
            &html[..html.len().min(300)]
        );

        let state_key = format!("relay:{}:oauth_state", legacy_relay_name);
        let exists = secrets.exists("test", &state_key).await.unwrap_or(true);
        assert!(!exists, "Legacy CSRF nonce should be deleted after use");
    }

    #[tokio::test]
    async fn test_relay_oauth_callback_nonce_under_different_user_fails() {
        // why: In hosted mode, the DB user's UUID differs from the gateway
        //      owner_id. If the nonce is stored under the DB user's scope,
        //      the callback handler (which uses owner_id) cannot find it.
        use axum::body::Body;
        use tower::ServiceExt;

        let secrets = test_secrets_store();
        let nonce = "nonce-stored-under-wrong-user";

        // given: nonce stored under a DB user UUID, NOT the gateway owner ("test")
        secrets
            .create(
                "b50a4a66-ba1b-439c-907b-cc6b371871b0",
                crate::secrets::CreateSecretParams::new(
                    format!("relay:{}:oauth_state", DEFAULT_RELAY_NAME),
                    nonce,
                ),
            )
            .await
            .expect("store nonce");

        // ext_mgr.user_id = "test", gateway owner_id = "test"
        let (ext_mgr, _wasm_tools_dir, _wasm_channels_dir) = test_ext_mgr(secrets);
        let state = test_gateway_state(Some(ext_mgr));
        let app = test_relay_oauth_router(state);

        // when: callback arrives with the correct nonce value
        let req = axum::http::Request::builder()
            .uri(format!(
                "/oauth/slack/callback?team_id=T123&provider=slack&state={}",
                nonce
            ))
            .body(Body::empty())
            .expect("request");

        let resp = ServiceExt::<axum::http::Request<Body>>::oneshot(app, req)
            .await
            .expect("response");

        // then: fails because nonce is under a different user scope
        let body = axum::body::to_bytes(resp.into_body(), 1024 * 64)
            .await
            .expect("body");
        let html = String::from_utf8_lossy(&body);
        assert!(
            html.contains("Invalid or expired authorization"),
            "Nonce stored under wrong user scope should fail lookup, got: {}",
            &html[..html.len().min(300)]
        );
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
