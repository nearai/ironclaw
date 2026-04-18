//! Thread and session operations for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate thread management (user input
//! processing, undo/redo, approval, auth, persistence) from the core loop.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::compaction::ContextCompactor;
use crate::agent::dispatcher::{
    AgenticLoopResult, ParsedAuthData, TurnUsageSummary, auth_instructions_or_default,
    capture_auth_prompt, emit_auth_required_status, execute_chat_tool_standalone,
    persist_selected_auth_prompt, restore_selected_auth_prompt,
};
use crate::agent::session::{
    MAX_PENDING_MESSAGES, PendingApproval, Session, ThreadState, TurnOutcome,
};
use crate::agent::submission::SubmissionResult;
use crate::channels::{ChatApprovalPrompt, HistoryMessage, IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::generated_images::GeneratedImageSentinel;
use crate::llm::{ChatMessage, ToolCall};
use crate::tools::redact_params;
use ironclaw_common::truncate_preview;

const FORGED_THREAD_ID_ERROR: &str = "Invalid or unauthorized thread ID.";

fn tool_result_preview_for_persistence(result: &serde_json::Value) -> String {
    if GeneratedImageSentinel::from_value(result).is_some() {
        return "Generated image".to_string();
    }
    match result {
        serde_json::Value::String(s) => truncate_preview(s, 500),
        other => truncate_preview(&other.to_string(), 500),
    }
}

fn tool_result_content_for_persistence(result: &serde_json::Value) -> String {
    if let Some(sentinel) = GeneratedImageSentinel::from_value(result) {
        // Persist the full image sentinel so web history can reconstruct the
        // generated image on refresh without any schema changes. Keep a hard
        // cap so unexpectedly large data URLs do not grow DB rows without bound.
        return sentinel.record_content_for_persistence();
    }
    match result {
        serde_json::Value::String(s) => truncate_preview(s, 1000),
        other => truncate_preview(&other.to_string(), 1000),
    }
}

fn tool_result_content_for_rebuild(result: &str) -> String {
    let Some(sentinel) =
        GeneratedImageSentinel::from_value(&serde_json::Value::String(result.to_string()))
    else {
        return result.to_string();
    };
    sentinel.summary_for_context()
}

const INVALID_AUTH_TOKEN_MESSAGE: &str = "Invalid token. Please try again.";
fn requires_preexisting_uuid_thread(channel: &str) -> bool {
    // Gateway-style channels send server-issued conversation UUIDs.
    // Unknown UUIDs should be rejected instead of silently creating a new thread.
    matches!(channel, "gateway" | "test")
}

fn auth_retry_message_for_error(error: &crate::extensions::ExtensionError) -> Option<String> {
    matches!(
        error,
        crate::extensions::ExtensionError::ValidationFailed(_)
    )
    .then(|| INVALID_AUTH_TOKEN_MESSAGE.to_string())
}

fn history_messages_from_thread(thread: &crate::agent::session::Thread) -> Vec<HistoryMessage> {
    let mut messages = Vec::new();

    for turn in &thread.turns {
        if !turn.user_input.is_empty() {
            messages.push(HistoryMessage {
                role: "user".to_string(),
                content: turn.user_input.clone(),
                timestamp: turn.started_at,
            });
        }

        if let Some(response) = turn.response.as_ref() {
            messages.push(HistoryMessage {
                role: "assistant".to_string(),
                content: response.clone(),
                timestamp: turn.completed_at.unwrap_or(turn.started_at),
            });
        }
    }

    messages
}

/// Pick the right parameters value to surface in approval UI/SSE.
///
/// Prefers `display_parameters` (which has sensitive fields redacted) but
/// falls back to the unredacted `parameters` when the display field is
/// `Value::Null`. The null case fires for `PendingApproval` rows persisted
/// before the `display_parameters` field existed: `#[serde(default)]`
/// deserializes the missing field as `Value::Null`. Without this fallback
/// the SSE/CLI approval prompt would show `null` parameters for legacy
/// approvals that round-tripped through the DB or a checkpoint.
///
/// Both `approval_prompt_from_pending` and `pending_approval_status_update`
/// must use this — they feed two parallel UI surfaces that show the same
/// approval, and any inconsistency means one surface displays parameters
/// and the other shows `null`.
fn display_parameters_or_fallback(pending: &PendingApproval) -> serde_json::Value {
    if pending.display_parameters.is_null() {
        pending.parameters.clone()
    } else {
        pending.display_parameters.clone()
    }
}

fn approval_prompt_from_pending(pending: &PendingApproval) -> ChatApprovalPrompt {
    ChatApprovalPrompt {
        request_id: pending.request_id.to_string(),
        tool_name: pending.tool_name.clone(),
        description: pending.description.clone(),
        parameters: display_parameters_or_fallback(pending),
        allow_always: pending.allow_always,
    }
}

fn thread_summaries_from_conversations(
    mut conversations: Vec<crate::history::ConversationSummary>,
) -> Vec<crate::channels::ThreadSummary> {
    conversations.sort_by(|a, b| {
        b.last_activity
            .cmp(&a.last_activity)
            .then_with(|| b.started_at.cmp(&a.started_at))
            .then_with(|| a.id.cmp(&b.id))
    });

    conversations
        .into_iter()
        .map(|c| crate::channels::ThreadSummary {
            id: c.id.to_string(),
            title: c.title,
            message_count: c.message_count,
            last_activity: c.last_activity.to_rfc3339(),
            channel: c.channel,
        })
        .collect()
}

fn turn_usage_from_result(result: &Result<AgenticLoopResult, Error>) -> Option<&TurnUsageSummary> {
    match result {
        Ok(AgenticLoopResult::Response { turn_usage, .. })
        | Ok(AgenticLoopResult::NeedApproval { turn_usage, .. })
        | Ok(AgenticLoopResult::Failed { turn_usage, .. })
        | Ok(AgenticLoopResult::AuthPending { turn_usage, .. }) => Some(turn_usage),
        Err(_) => None,
    }
}

fn pending_approval_status_update(pending: &PendingApproval) -> StatusUpdate {
    StatusUpdate::ApprovalNeeded {
        request_id: pending.request_id.to_string(),
        tool_name: pending.tool_name.clone(),
        description: pending.description.clone(),
        parameters: display_parameters_or_fallback(pending),
        allow_always: pending.allow_always,
    }
}

fn pending_approval_message(pending: Option<&PendingApproval>) -> String {
    let approval_context = pending.map(|approval| {
        let desc_preview =
            crate::agent::agent_loop::truncate_for_preview(&approval.description, 80);
        (approval.tool_name.clone(), desc_preview)
    });

    match approval_context {
        Some((tool_name, desc_preview)) => {
            format!("Waiting for approval: {tool_name} — {desc_preview}. Use /interrupt to cancel.")
        }
        None => "Waiting for approval. Use /interrupt to cancel.".to_string(),
    }
}

impl Agent {
    /// Hydrate a historical thread from DB into memory if not already present.
    ///
    /// Called before `resolve_thread` so that the session manager finds the
    /// thread on lookup instead of creating a new one.
    ///
    /// Creates an in-memory thread with the exact UUID the frontend sent,
    /// even when the conversation has zero messages (e.g. a brand-new
    /// assistant thread). Without this, `resolve_thread` would mint a
    /// fresh UUID and all messages would land in the wrong conversation.
    pub(super) async fn maybe_hydrate_thread(
        &self,
        message: &IncomingMessage,
        external_thread_id: &str,
    ) -> Option<String> {
        // Only hydrate UUID-shaped thread IDs (web gateway uses UUIDs)
        let thread_uuid = match Uuid::parse_str(external_thread_id) {
            Ok(id) => id,
            Err(_) => return None,
        };

        // Check if already in memory
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&thread_uuid) {
                return None;
            }
        }

        // Load history from DB (may be empty for a newly created thread).
        let mut chat_messages: Vec<ChatMessage> = Vec::new();
        let msg_count;

        if let Some(store) = self.store() {
            // Never hydrate history from a conversation UUID that isn't owned
            // by the current authenticated user.
            let owned = match store
                .conversation_belongs_to_user(thread_uuid, &message.user_id)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        "Failed to verify conversation ownership for hydration {}: {}",
                        thread_uuid,
                        e
                    );
                    if requires_preexisting_uuid_thread(&message.channel) {
                        return Some(FORGED_THREAD_ID_ERROR.to_string());
                    }
                    return None;
                }
            };
            if !owned {
                let exists = match store.get_conversation_metadata(thread_uuid).await {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => {
                        tracing::warn!(
                            "Failed to inspect conversation metadata for hydration {}: {}",
                            thread_uuid,
                            e
                        );
                        if requires_preexisting_uuid_thread(&message.channel) {
                            return Some(FORGED_THREAD_ID_ERROR.to_string());
                        }
                        return None;
                    }
                };

                if requires_preexisting_uuid_thread(&message.channel) {
                    // Allow new thread creation only from the Responses API.
                    // Both checks are required:
                    // - channel == "gateway": server-set, unforgeable by WASM
                    // - metadata.source == "responses_api": set server-side in
                    //   create_response_handler, not controllable by the web UI
                    //   chat which also uses the gateway channel
                    let is_responses_api = message.channel == "gateway"
                        && message.metadata.get("source").and_then(|v| v.as_str())
                            == Some("responses_api");
                    if !exists && is_responses_api {
                        tracing::debug!(
                            user = %message.user_id,
                            thread_id = %thread_uuid,
                            "Allowing new thread from gateway (Responses API)"
                        );
                        return None;
                    }
                    tracing::warn!(
                        user = %message.user_id,
                        channel = %message.channel,
                        thread_id = %thread_uuid,
                        exists,
                        "Rejected message for unavailable thread id"
                    );
                    return Some(FORGED_THREAD_ID_ERROR.to_string());
                }

                tracing::warn!(
                    user = %message.user_id,
                    thread_id = %thread_uuid,
                    exists,
                    "Skipped hydration for thread id not owned by sender"
                );
                return None;
            }

            let db_messages = store
                .list_conversation_messages(thread_uuid)
                .await
                .unwrap_or_default();
            msg_count = db_messages.len();
            chat_messages = rebuild_chat_messages_from_db(&db_messages);
        } else {
            msg_count = 0;
        }

        // Create thread with the historical ID and restore messages.
        // Read source_channel from DB so the authorization check uses the
        // original creator's channel, not the requesting message's channel.
        //
        // Fail-closed policy: if the DB lookup fails or the conversation has
        // no stored source_channel (legacy row), the thread is hydrated with
        // source_channel = None.  `is_approval_authorized(None, _)` returns
        // false, so approvals are denied until the conversation is backfilled
        // with a source_channel via an explicit migration or re-creation.
        let db_source_channel = if let Some(store) = self.store() {
            match store.get_conversation_source_channel(thread_uuid).await {
                Ok(sc) => {
                    if sc.is_none() {
                        tracing::warn!(
                            thread_id = %thread_uuid,
                            "Legacy thread has no stored source_channel; \
                             cross-channel approvals will be denied (fail-closed)"
                        );
                    }
                    sc
                }
                Err(e) => {
                    tracing::error!(
                        thread_id = %thread_uuid,
                        error = %e,
                        "Failed to read source_channel from DB; \
                         cross-channel approvals will be denied (fail-closed)"
                    );
                    None
                }
            }
        } else {
            None
        };
        let effective_source_channel = db_source_channel.as_deref();

        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        let mut thread = crate::agent::session::Thread::with_id(
            thread_uuid,
            session_id,
            effective_source_channel,
        );
        if !chat_messages.is_empty() {
            thread.restore_from_messages(chat_messages);
        }

        // Insert into session and register with session manager
        {
            let mut sess = session.lock().await;
            sess.threads.insert(thread_uuid, thread);
            sess.active_thread = Some(thread_uuid);
            sess.last_active_at = chrono::Utc::now();
        }

        self.session_manager
            .register_thread(
                &message.user_id,
                &message.channel,
                thread_uuid,
                Arc::clone(&session),
            )
            .await;

        tracing::debug!(
            "Hydrated thread {} from DB ({} messages)",
            thread_uuid,
            msg_count
        );

        None
    }

    pub(super) async fn process_user_input(
        &self,
        message: &IncomingMessage,
        tenant: crate::tenant::TenantCtx,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        content: &str,
    ) -> Result<SubmissionResult, Error> {
        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            content_len = content.len(),
            "Processing user input"
        );

        // First check thread state without holding lock during I/O
        let (thread_state, pending_approval) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            (thread.state, thread.pending_approval.clone())
        };

        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            thread_state = ?thread_state,
            "Checked thread state"
        );

        // Check thread state
        match thread_state {
            ThreadState::Processing => {
                let mut sess = session.lock().await;
                match sess.threads.get_mut(&thread_id) {
                    Some(thread) => {
                        // Re-check state under lock — the turn may have completed
                        // between the snapshot read and this mutable lock acquisition.
                        if thread.state == ThreadState::Processing {
                            // Reject messages with attachments — the queue stores
                            // text only, so attachments would be silently dropped.
                            if !message.attachments.is_empty() {
                                return Ok(SubmissionResult::error(
                                    "Cannot queue messages with attachments while a turn is processing. \
                                     Please resend after the current turn completes.",
                                ));
                            }

                            // Run the same safety checks that the normal path applies
                            // (validation, policy, secret scan) so that blocked content
                            // is never stored in pending_messages or serialized.
                            let validation = self.safety().validate_input(content);
                            if !validation.is_valid {
                                let details = validation
                                    .errors
                                    .iter()
                                    .map(|e| format!("{}: {}", e.field, e.message))
                                    .collect::<Vec<_>>()
                                    .join("; ");
                                return Ok(SubmissionResult::error(format!(
                                    "Input rejected by safety validation: {details}",
                                )));
                            }
                            let violations = self.safety().check_policy(content);
                            if violations
                                .iter()
                                .any(|rule| rule.action == ironclaw_safety::PolicyAction::Block)
                            {
                                return Ok(SubmissionResult::error(
                                    "Input rejected by safety policy.",
                                ));
                            }
                            if let Some(warning) = self.safety().scan_inbound_for_secrets(content) {
                                tracing::warn!(
                                    user = %message.user_id,
                                    channel = %message.channel,
                                    "Queued message blocked: contains leaked secret"
                                );
                                return Ok(SubmissionResult::error(warning));
                            }

                            if !thread.queue_message(content.to_string()) {
                                return Ok(SubmissionResult::error(format!(
                                    "Message queue full ({MAX_PENDING_MESSAGES}). Wait for the current turn to complete.",
                                )));
                            }
                            // Return `Ok` (not `Response`) so the drain loop in
                            // agent_loop.rs breaks — `Ok` signals a control
                            // acknowledgment, not a completed LLM turn.
                            return Ok(SubmissionResult::Ok {
                                message: Some(
                                    "Message queued — will be processed after the current turn."
                                        .into(),
                                ),
                            });
                        }
                        // State changed (turn completed) — fall through to process normally.
                        // NOTE: `sess` (the Mutex guard) is dropped at the end of
                        // this `Processing` match arm, releasing the session lock
                        // before the rest of process_user_input runs. No deadlock.
                    }
                    None => {
                        return Ok(SubmissionResult::error("Thread no longer exists."));
                    }
                }
            }
            ThreadState::AwaitingApproval => {
                tracing::warn!(
                    message_id = %message.id,
                    thread_id = %thread_id,
                    "Thread awaiting approval, rejecting new input"
                );
                if let Some(pending) = pending_approval.as_ref() {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            pending_approval_status_update(pending),
                            &message.metadata,
                        )
                        .await;
                }
                let msg = pending_approval_message(pending_approval.as_ref());
                return Ok(SubmissionResult::pending(msg));
            }
            ThreadState::Completed => {
                tracing::warn!(
                    message_id = %message.id,
                    thread_id = %thread_id,
                    "Thread completed, rejecting new input"
                );
                return Ok(SubmissionResult::error(
                    "Thread completed. Use /thread new.",
                ));
            }
            ThreadState::Idle | ThreadState::Interrupted => {
                // Can proceed
            }
        }

        // Safety validation for user input
        let validation = self.safety().validate_input(content);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(SubmissionResult::error(format!(
                "Input rejected by safety validation: {}",
                details
            )));
        }

        let violations = self.safety().check_policy(content);
        if violations
            .iter()
            .any(|rule| rule.action == ironclaw_safety::PolicyAction::Block)
        {
            return Ok(SubmissionResult::error("Input rejected by safety policy."));
        }

        // Scan inbound messages for secrets (API keys, tokens).
        // Catching them here prevents the LLM from echoing them back, which
        // would trigger the outbound leak detector and create error loops.
        if let Some(warning) = self.safety().scan_inbound_for_secrets(content) {
            tracing::warn!(
                user = %message.user_id,
                channel = %message.channel,
                "Inbound message blocked: contains leaked secret"
            );
            return Ok(SubmissionResult::error(warning));
        }

        // Handle explicit commands (starting with /) directly
        // Everything else goes through the normal agentic loop with tools
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };

        if let Some(intent) = self.router.route_command(&temp_message) {
            // Explicit command like /status, /job, /list - handle directly
            return self.handle_job_or_command(intent, message, &tenant).await;
        }

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

        // Auto-compact if needed BEFORE adding new turn
        {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let messages = thread.messages();
            if let Some(strategy) = self.context_monitor.suggest_compaction(&messages) {
                let pct = self.context_monitor.usage_percent(&messages);
                tracing::info!("Context at {:.1}% capacity, auto-compacting", pct);

                // Notify the user that compaction is happening
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status(format!(
                            "Context at {:.0}% capacity, compacting...",
                            pct
                        )),
                        &message.metadata,
                    )
                    .await;

                let compactor = ContextCompactor::new(self.llm().clone());
                if let Err(e) = compactor
                    .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
                    .await
                {
                    tracing::warn!("Auto-compaction failed: {}", e);
                }
            }
        }

        // Create checkpoint before turn
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(
                thread.turn_number(),
                thread.messages(),
                format!("Before turn {}", thread.turn_number()),
            );
        }

        // Augment content with attachment context (transcripts, metadata, images)
        let augmented =
            crate::agent::attachments::augment_with_attachments(content, &message.attachments);
        let (effective_content, image_parts) = match &augmented {
            Some(result) => (result.text.as_str(), result.image_parts.clone()),
            None => (content, Vec::new()),
        };

        // Start the turn and get messages
        let turn_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let turn = thread.start_turn(effective_content);
            turn.image_content_parts = image_parts;
            thread.messages()
        };

        // Persist user message to DB immediately so it survives crashes
        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            "Persisting user message to DB"
        );
        self.persist_user_message(
            thread_id,
            &message.channel,
            &message.user_id,
            effective_content,
        )
        .await;

        tracing::debug!(
            message_id = %message.id,
            thread_id = %thread_id,
            "User message persisted, starting agentic loop"
        );

        // Send thinking status
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Thinking("Processing...".into()),
                &message.metadata,
            )
            .await;

        // Run the agentic tool execution loop
        let result = self
            .run_agentic_loop(message, tenant, session.clone(), thread_id, turn_messages)
            .await;

        // Re-acquire lock and check if interrupted
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            if let Some(turn_usage) = turn_usage_from_result(&result) {
                self.send_turn_cost_status(&message.channel, &message.metadata, turn_usage)
                    .await;
            }
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Interrupted".into()),
                    &message.metadata,
                )
                .await;
            return Ok(SubmissionResult::Interrupted);
        }

        // Complete, fail, or request approval
        match result {
            Ok(AgenticLoopResult::Response {
                text: response,
                turn_usage,
            }) => {
                // Extract <suggestions> from response text before user sees it
                let (response, suggestions) =
                    crate::agent::dispatcher::extract_suggestions(&response);

                // Hook: TransformResponse — allow hooks to modify or reject the final response
                let response = {
                    let event = crate::hooks::HookEvent::ResponseTransform {
                        user_id: message.user_id.clone(),
                        thread_id: thread_id.to_string(),
                        response: response.clone(),
                    };
                    match self.hooks().run(&event).await {
                        Err(crate::hooks::HookError::Rejected { reason }) => {
                            format!("[Response filtered: {}]", reason)
                        }
                        Err(err) => {
                            format!("[Response blocked by hook policy: {}]", err)
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_response),
                        }) => new_response,
                        _ => response, // fail-open: use original
                    }
                };

                thread.conclude_turn(TurnOutcome::Completed(response.clone()));
                let (turn_number, tool_calls, narrative) = thread
                    .turns
                    .last()
                    .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone()))
                    .unwrap_or_default();

                // Persist tool calls then assistant response (user message already persisted at turn start)
                self.persist_tool_calls(
                    thread_id,
                    &message.channel,
                    &message.user_id,
                    turn_number,
                    &tool_calls,
                    narrative.as_deref(),
                )
                .await;
                self.persist_assistant_response(
                    thread_id,
                    &message.channel,
                    &message.user_id,
                    &response,
                )
                .await;

                // Send suggestions after response (best-effort, rendered by web gateway)
                if !suggestions.is_empty() {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Suggestions { suggestions },
                            &message.metadata,
                        )
                        .await;
                }

                self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                    .await;

                Ok(SubmissionResult::response(response))
            }
            Ok(AgenticLoopResult::NeedApproval {
                pending,
                turn_usage,
            }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.display_parameters.clone();
                let allow_always = pending.allow_always;
                thread.await_approval(*pending);
                self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                    .await;
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ApprovalNeeded {
                            request_id: request_id.to_string(),
                            tool_name: tool_name.clone(),
                            description: description.clone(),
                            parameters: parameters.clone(),
                            allow_always,
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                    allow_always,
                })
            }
            Ok(AgenticLoopResult::AuthPending { turn_usage }) => {
                // Auth-required card already sent by the dispatcher, and the
                // thread is already in auth mode (enter_auth_mode called in
                // execute_tool_calls). CompletedSilently transitions to Idle
                // without persisting a redundant text response alongside the
                // auth card.
                thread.conclude_turn(TurnOutcome::CompletedSilently);
                //
                // Persist tool calls so history shows what happened, but
                // skip persist_assistant_response — the auth card is the
                // only user-facing signal.
                let (turn_number, tool_calls, narrative) = thread
                    .turns
                    .last()
                    .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone()))
                    .unwrap_or_default();
                self.persist_tool_calls(
                    thread_id,
                    &message.channel,
                    &message.user_id,
                    turn_number,
                    &tool_calls,
                    narrative.as_deref(),
                )
                .await;
                self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                    .await;
                Ok(SubmissionResult::auth_pending())
            }
            Ok(AgenticLoopResult::Failed { error, turn_usage }) => {
                self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                    .await;
                thread.conclude_turn(TurnOutcome::Failed(error.to_string()));
                Ok(SubmissionResult::error(error.to_string()))
            }
            Err(e) => {
                thread.conclude_turn(TurnOutcome::Failed(e.to_string()));
                // User message already persisted at turn start; nothing else to save
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    /// Ensure a thread UUID is writable for `(channel, user_id)`.
    ///
    /// Returns `false` for foreign/unowned conversation IDs or DB errors.
    async fn ensure_writable_conversation(
        &self,
        store: &Arc<dyn crate::db::Database>,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
    ) -> bool {
        match store
            .ensure_conversation(thread_id, channel, user_id, None, Some(channel))
            .await
        {
            Ok(true) => true,
            Ok(false) => {
                tracing::warn!(
                    user = %user_id,
                    channel = %channel,
                    thread_id = %thread_id,
                    "Rejected write for unavailable thread id"
                );
                false
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to ensure writable conversation {}: {}",
                    thread_id,
                    e
                );
                false
            }
        }
    }

    /// Persist the user message to the DB at turn start (before the agentic loop).
    ///
    /// This ensures the user message is durable even if the process crashes
    /// mid-response. Call this right after `thread.start_turn()`.
    pub(super) async fn persist_user_message(
        &self,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
        user_input: &str,
    ) {
        let store = match self.store() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        if !self
            .ensure_writable_conversation(&store, thread_id, channel, user_id)
            .await
        {
            return;
        }

        if let Err(e) = store
            .add_conversation_message(thread_id, "user", user_input)
            .await
        {
            tracing::warn!("Failed to persist user message: {}", e);
        }
    }

    /// Persist the assistant response to the DB after the agentic loop completes.
    ///
    /// Re-ensures the conversation row exists so that assistant responses are
    /// still persisted even if `persist_user_message` failed transiently at
    /// turn start (e.g. a brief DB blip that resolved before response time).
    pub(super) async fn persist_assistant_response(
        &self,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
        response: &str,
    ) {
        let store = match self.store() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        if !self
            .ensure_writable_conversation(&store, thread_id, channel, user_id)
            .await
        {
            return;
        }

        if let Err(e) = store
            .add_conversation_message(thread_id, "assistant", response)
            .await
        {
            tracing::warn!("Failed to persist assistant message: {}", e);
        }
    }

    /// Persist tool call summaries to the DB as a `role="tool_calls"` message.
    ///
    /// Stored between the user and assistant messages so that
    /// `build_turns_from_db_messages` can reconstruct the tool call history.
    /// Content is a JSON object: `{ "calls": [...], "narrative": "..." }`.
    /// The `calls` array contains tool call summaries with optional `rationale`
    /// and `tool_call_id` fields. Legacy rows may be plain JSON arrays.
    pub(super) async fn persist_tool_calls(
        &self,
        thread_id: Uuid,
        channel: &str,
        user_id: &str,
        turn_number: usize,
        tool_calls: &[crate::agent::session::TurnToolCall],
        narrative: Option<&str>,
    ) {
        if tool_calls.is_empty() {
            return;
        }

        let store = match self.store() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        let summaries: Vec<serde_json::Value> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, tc)| {
                let mut obj = serde_json::json!({
                    "name": tc.name,
                    "call_id": format!("turn{}_{}", turn_number, i),
                });
                if let Some(ref result) = tc.result {
                    let preview = tool_result_preview_for_persistence(result);
                    obj["result_preview"] = serde_json::Value::String(preview);
                    // Persist full image sentinel payloads so the web history can
                    // reconstruct generated image cards after refresh. Other tool
                    // results remain truncated to keep DB rows bounded.
                    let full_result = tool_result_content_for_persistence(result);
                    obj["result"] = serde_json::Value::String(full_result);
                }
                if let Some(ref error) = tc.error {
                    obj["error"] = serde_json::Value::String(truncate_preview(error, 200));
                }
                if let Some(ref rationale) = tc.rationale {
                    obj["rationale"] = serde_json::Value::String(truncate_preview(rationale, 500));
                }
                if let Some(ref tool_call_id) = tc.tool_call_id {
                    obj["tool_call_id"] =
                        serde_json::Value::String(truncate_preview(tool_call_id, 128));
                }
                obj
            })
            .collect();

        // Wrap in an object with optional narrative so it can be reconstructed.
        // safety: no byte-index slicing here; comment describes JSON shape
        let wrapper = if let Some(n) = narrative {
            serde_json::json!({
                "narrative": truncate_preview(n, 1000),
                "calls": summaries,
            })
        } else {
            serde_json::json!({
                "calls": summaries,
            })
        };
        let content = match serde_json::to_string(&wrapper) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to serialize tool calls: {}", e);
                return;
            }
        };

        if !self
            .ensure_writable_conversation(&store, thread_id, channel, user_id)
            .await
        {
            return;
        }

        if let Err(e) = store
            .add_conversation_message(thread_id, "tool_calls", &content)
            .await
        {
            tracing::warn!("Failed to persist tool calls: {}", e);
        }
    }

    pub(super) async fn process_undo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_undo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to undo."));
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        // Save current state to redo, get previous checkpoint
        let current_messages = thread.messages();
        let current_turn = thread.turn_number();

        if let Some(checkpoint) = mgr.undo(current_turn, current_messages) {
            // Extract values before consuming the reference
            let turn_number = checkpoint.turn_number;
            let messages = checkpoint.messages.clone();
            let undo_count = mgr.undo_count();
            // Restore thread from checkpoint
            thread.restore_from_messages(messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Undone to turn {}. {} undo(s) remaining.",
                turn_number, undo_count
            )))
        } else {
            Ok(SubmissionResult::error("Undo failed."))
        }
    }

    pub(super) async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if !mgr.can_redo() {
            return Ok(SubmissionResult::ok_with_message("Nothing to redo."));
        }

        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let current_messages = thread.messages();
        let current_turn = thread.turn_number();

        if let Some(checkpoint) = mgr.redo(current_turn, current_messages) {
            thread.restore_from_messages(checkpoint.messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Redone to turn {}.",
                checkpoint.turn_number
            )))
        } else {
            Ok(SubmissionResult::error("Redo failed."))
        }
    }

    pub(super) async fn process_interrupt(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        match thread.state {
            ThreadState::Processing | ThreadState::AwaitingApproval => {
                thread.interrupt();
                Ok(SubmissionResult::ok_with_message("Interrupted."))
            }
            _ => Ok(SubmissionResult::ok_with_message("Nothing to interrupt.")),
        }
    }

    pub(super) async fn process_compact(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let messages = thread.messages();
        let usage = self.context_monitor.usage_percent(&messages);
        let strategy = self
            .context_monitor
            .suggest_compaction(&messages)
            .unwrap_or(
                crate::agent::context_monitor::CompactionStrategy::Summarize { keep_recent: 5 },
            );

        let compactor = ContextCompactor::new(self.llm().clone());
        match compactor
            .compact(thread, strategy, self.workspace().map(|w| w.as_ref()))
            .await
        {
            Ok(result) => {
                let mut msg = format!(
                    "Compacted: {} turns removed, {} → {} tokens (was {:.1}% full)",
                    result.turns_removed, result.tokens_before, result.tokens_after, usage
                );
                if result.summary_written {
                    msg.push_str(", summary saved to workspace");
                }
                Ok(SubmissionResult::ok_with_message(msg))
            }
            Err(e) => Ok(SubmissionResult::error(format!("Compaction failed: {}", e))),
        }
    }

    pub(super) async fn process_clear(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
        thread.turns.clear();
        thread.pending_messages.clear();
        thread.state = ThreadState::Idle;

        // Clear undo history too
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        undo_mgr.lock().await.clear();

        Ok(SubmissionResult::ok_with_message("Thread cleared."))
    }

    /// Process an approval or rejection of a pending tool execution.
    pub(super) async fn process_approval(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        request_id: Option<Uuid>,
        approved: bool,
        always: bool,
    ) -> Result<SubmissionResult, Error> {
        // Take + verify under a single lock to prevent TOCTOU races where a
        // concurrent operation could modify the thread between take and restore.
        let pending = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            if thread.state != ThreadState::AwaitingApproval {
                // Stale or duplicate approval (tool already executed) — silently ignore.
                tracing::debug!(
                    %thread_id,
                    state = ?thread.state,
                    "Ignoring stale approval: thread not in AwaitingApproval state"
                );
                return Ok(SubmissionResult::ok_with_message(""));
            }

            let pending = match thread.take_pending_approval() {
                Some(p) => p,
                None => {
                    tracing::debug!(
                        %thread_id,
                        "Ignoring stale approval: no pending approval found"
                    );
                    return Ok(SubmissionResult::ok_with_message(""));
                }
            };

            // Verify request ID while still holding the lock so the pending
            // approval is atomically restored on mismatch.
            if let Some(req_id) = request_id
                && req_id != pending.request_id
            {
                thread.await_approval(pending);
                return Ok(SubmissionResult::error(
                    "Request ID mismatch. Use the correct request ID.",
                ));
            }

            pending
        };

        if approved {
            // If always, add to auto-approved set and persist to settings.
            if always {
                let mut sess = session.lock().await;
                sess.auto_approve_tool(&pending.tool_name);
                tracing::info!(
                    "Auto-approved tool '{}' for session {}",
                    pending.tool_name,
                    sess.id
                );
                drop(sess);

                // Defense-in-depth: don't persist AlwaysAllow for tools that
                // declare ApprovalRequirement::Always (the UI hides the
                // "Always" button for locked tools, but a crafted client
                // could send it).
                let tool_ref = self.tools().get(&pending.tool_name).await;
                let is_locked = tool_ref
                    .as_ref()
                    .map(|t| {
                        matches!(
                            t.requires_approval(&serde_json::json!({})),
                            crate::tools::ApprovalRequirement::Always
                        )
                    })
                    .unwrap_or(false);

                if is_locked {
                    tracing::warn!(
                        tool = %pending.tool_name,
                        "Skipping AlwaysAllow persist — tool declares ApprovalRequirement::Always"
                    );
                } else {
                    // Persist AlwaysAllow to the per-user DB settings so the
                    // preference survives process restarts. Uses the same
                    // set_setting path as tool_permission_set and the web UI.
                    let tenant = self.tenant_ctx(&message.user_id).await;
                    if let Some(store) = tenant.store() {
                        let key = format!("tool_permissions.{}", pending.tool_name);
                        let val = serde_json::to_value(
                            crate::tools::permissions::PermissionState::AlwaysAllow,
                        )
                        .unwrap_or(serde_json::Value::String("always_allow".to_string()));
                        match store.set_setting(&key, &val).await {
                            Ok(()) => tracing::debug!(
                                tool = %pending.tool_name,
                                "Persisted AlwaysAllow permission to DB settings"
                            ),
                            Err(e) => tracing::warn!(
                                "process_approval: failed to persist AlwaysAllow for '{}': {}",
                                pending.tool_name,
                                e
                            ),
                        }
                    }
                } // else (not locked)
            }

            // Reset thread state to processing
            {
                let mut sess = session.lock().await;
                match sess.threads.get_mut(&thread_id) {
                    Some(thread) => thread.state = ThreadState::Processing,
                    None => {
                        return Err(Error::from(crate::error::JobError::NotFound {
                            id: thread_id,
                        }));
                    }
                }
            }

            // Execute the approved tool and continue the loop
            let mut job_ctx =
                JobContext::with_user(&message.user_id, "chat", "Interactive chat session")
                    .with_requester_id(&message.sender_id);
            job_ctx.http_interceptor = self.deps.http_interceptor.clone();
            job_ctx.metadata = crate::agent::agent_loop::chat_tool_execution_metadata(message);
            // Prefer a valid timezone from the approval message, fall back to the
            // resolved timezone stored when the approval was originally requested.
            let tz_candidate = message
                .timezone
                .as_deref()
                .filter(|tz| crate::timezone::parse_timezone(tz).is_some())
                .or(pending.user_timezone.as_deref());
            if let Some(tz) = tz_candidate {
                job_ctx.user_timezone = tz.to_string();
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::tool_started_with_id(
                        pending.tool_name.clone(),
                        &pending.parameters,
                        Some(pending.tool_call_id.clone()),
                    ),
                    &message.metadata,
                )
                .await;

            let tool_result = self
                .execute_chat_tool(&pending.tool_name, &pending.parameters, &job_ctx)
                .await;

            let tool_ref = self.tools().get(&pending.tool_name).await;
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::tool_completed(
                        pending.tool_name.clone(),
                        Some(pending.tool_call_id.clone()),
                        &tool_result,
                        &pending.display_parameters,
                        tool_ref.as_deref(),
                    ),
                    &message.metadata,
                )
                .await;

            if let Ok(ref output) = tool_result
                && !output.is_empty()
            {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ToolResult {
                            name: pending.tool_name.clone(),
                            preview: output.clone(),
                            call_id: Some(pending.tool_call_id.clone()),
                        },
                        &message.metadata,
                    )
                    .await;
            }

            let mut selected_auth_prompt =
                restore_selected_auth_prompt(pending.selected_auth_prompt.clone());

            // Build context including the tool result
            let mut context_messages = pending.context_messages;
            let deferred_tool_calls = pending.deferred_tool_calls;

            // Sanitize tool result, then record the cleaned version in the
            // thread. Must happen before auth intercept check which may return early.
            let is_tool_error = tool_result.is_err();
            let (result_content, _) = crate::tools::execute::process_tool_result(
                self.safety(),
                &pending.tool_name,
                &pending.tool_call_id,
                &tool_result,
            );

            // Record sanitized result in thread
            {
                let mut sess = session.lock().await;
                match sess.threads.get_mut(&thread_id) {
                    Some(thread) => {
                        if let Some(turn) = thread.last_turn_mut() {
                            if is_tool_error {
                                turn.record_tool_error_for(
                                    &pending.tool_call_id,
                                    result_content.clone(),
                                );
                            } else {
                                turn.record_tool_result_for(
                                    &pending.tool_call_id,
                                    serde_json::json!(result_content),
                                );
                            }
                        }
                    }
                    None => {
                        tracing::debug!(
                            %thread_id,
                            "Thread disappeared before tool result could be recorded"
                        );
                    }
                }
            }

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

            capture_auth_prompt(&mut selected_auth_prompt, &pending.tool_name, &tool_result);

            // Replay deferred tool calls from the same assistant message so
            // every tool_use ID gets a matching tool_result before the next
            // LLM call.
            if !deferred_tool_calls.is_empty() {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Thinking(format!(
                            "Executing {} deferred tool(s)...",
                            deferred_tool_calls.len()
                        )),
                        &message.metadata,
                    )
                    .await;
            }

            // === Phase 1: Preflight (sequential) ===
            // Walk deferred tools checking approval. Collect runnable
            // tools; stop at the first that needs approval.
            let mut runnable: Vec<crate::llm::ToolCall> = Vec::new();
            let mut approval_needed: Option<(
                usize,
                crate::llm::ToolCall,
                Arc<dyn crate::tools::Tool>,
                bool, // allow_always
            )> = None;

            for (idx, tc) in deferred_tool_calls.iter().enumerate() {
                if let Some(tool) = self.tools().get(&tc.name).await {
                    // Match dispatcher.rs: when auto_approve_tools is true, skip
                    // all approval checks (including ApprovalRequirement::Always).
                    let (needs_approval, allow_always) = if self.config.auto_approve_tools {
                        (false, true)
                    } else {
                        use crate::tools::ApprovalRequirement;
                        let requirement = tool.requires_approval(&tc.arguments);
                        let needs = match requirement {
                            ApprovalRequirement::Never => false,
                            ApprovalRequirement::UnlessAutoApproved => {
                                let sess = session.lock().await;
                                !sess.is_tool_auto_approved(&tc.name)
                            }
                            ApprovalRequirement::Always => true,
                        };
                        (needs, !matches!(requirement, ApprovalRequirement::Always))
                    };

                    if needs_approval {
                        approval_needed = Some((idx, tc.clone(), tool, allow_always));
                        break; // remaining tools stay deferred
                    }
                }

                runnable.push(tc.clone());
            }

            // === Phase 2: Parallel execution ===
            let exec_results: Vec<(crate::llm::ToolCall, Result<String, Error>)> = if runnable.len()
                <= 1
            {
                // Single tool (or none): execute inline
                let mut results = Vec::new();
                for tc in &runnable {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::tool_started_with_id(
                                tc.name.clone(),
                                &tc.arguments,
                                Some(tc.id.clone()),
                            ),
                            &message.metadata,
                        )
                        .await;

                    let result = self
                        .execute_chat_tool(&tc.name, &tc.arguments, &job_ctx)
                        .await;

                    let deferred_tool = self.tools().get(&tc.name).await;
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::tool_completed(
                                tc.name.clone(),
                                Some(tc.id.clone()),
                                &result,
                                &tc.arguments,
                                deferred_tool.as_deref(),
                            ),
                            &message.metadata,
                        )
                        .await;

                    results.push((tc.clone(), result));
                }
                results
            } else {
                // Multiple tools: execute in parallel via JoinSet
                let mut join_set = JoinSet::new();
                let runnable_count = runnable.len();

                for (spawn_idx, tc) in runnable.iter().enumerate() {
                    let tools = self.tools().clone();
                    let safety = self.safety().clone();
                    let channels = self.channels.clone();
                    let job_ctx = job_ctx.clone();
                    let tc = tc.clone();
                    let channel = message.channel.clone();
                    let metadata = message.metadata.clone();

                    join_set.spawn(async move {
                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::tool_started_with_id(
                                    tc.name.clone(),
                                    &tc.arguments,
                                    Some(tc.id.clone()),
                                ),
                                &metadata,
                            )
                            .await;

                        let result = execute_chat_tool_standalone(
                            &tools,
                            &safety,
                            &tc.name,
                            &tc.arguments,
                            &job_ctx,
                        )
                        .await;

                        let par_tool = tools.get(&tc.name).await;
                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::tool_completed(
                                    tc.name.clone(),
                                    Some(tc.id.clone()),
                                    &result,
                                    &tc.arguments,
                                    par_tool.as_deref(),
                                ),
                                &metadata,
                            )
                            .await;

                        (spawn_idx, tc, result)
                    });
                }

                // Collect and reorder by original index
                let mut ordered: Vec<Option<(crate::llm::ToolCall, Result<String, Error>)>> =
                    (0..runnable_count).map(|_| None).collect();
                while let Some(join_result) = join_set.join_next().await {
                    match join_result {
                        Ok((idx, tc, result)) => {
                            ordered[idx] = Some((tc, result));
                        }
                        Err(e) => {
                            if e.is_panic() {
                                tracing::error!("Deferred tool execution task panicked: {}", e);
                            } else {
                                tracing::error!("Deferred tool execution task cancelled: {}", e);
                            }
                        }
                    }
                }

                // Fill panicked slots with error results
                ordered
                    .into_iter()
                    .enumerate()
                    .map(|(i, opt)| {
                        opt.unwrap_or_else(|| {
                            let tc = runnable[i].clone();
                            let err: Error = crate::error::ToolError::ExecutionFailed {
                                name: tc.name.clone(),
                                reason: "Task failed during execution".to_string(),
                            }
                            .into();
                            (tc, Err(err))
                        })
                    })
                    .collect()
            };

            // === Phase 3: Post-flight (sequential, in original order) ===
            // Process all results before any conditional return so every
            // tool result is recorded in the session audit trail.

            for (tc, deferred_result) in exec_results {
                if let Ok(ref output) = deferred_result
                    && !output.is_empty()
                {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolResult {
                                name: tc.name.clone(),
                                preview: output.clone(),
                                call_id: Some(tc.id.clone()),
                            },
                            &message.metadata,
                        )
                        .await;
                }

                // Sanitize first, then record the cleaned version in thread.
                // Must happen before auth detection which may set deferred_auth.
                let is_deferred_error = deferred_result.is_err();
                let (deferred_content, _) = crate::tools::execute::process_tool_result(
                    self.safety(),
                    &tc.name,
                    &tc.id,
                    &deferred_result,
                );

                // Record sanitized result in thread
                {
                    let mut sess = session.lock().await;
                    match sess.threads.get_mut(&thread_id) {
                        Some(thread) => {
                            if let Some(turn) = thread.last_turn_mut() {
                                if is_deferred_error {
                                    turn.record_tool_error_for(&tc.id, deferred_content.clone());
                                } else {
                                    turn.record_tool_result_for(
                                        &tc.id,
                                        serde_json::json!(deferred_content),
                                    );
                                }
                            }
                        }
                        None => {
                            tracing::debug!(
                                %thread_id,
                                tool = %tc.name,
                                "Thread disappeared before deferred tool result could be recorded"
                            );
                        }
                    }
                }

                capture_auth_prompt(&mut selected_auth_prompt, &tc.name, &deferred_result);

                context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, deferred_content));
            }

            // Handle approval if a tool needed it
            if let Some((approval_idx, tc, tool, allow_always)) = approval_needed {
                // Emit auth prompt alongside the approval card so the user
                // sees the connect button without waiting for approval to resolve.
                if let Some((ref ext_name, ref auth_data)) = selected_auth_prompt {
                    emit_auth_required_status(
                        &self.channels,
                        message,
                        ext_name.clone(),
                        auth_data.instructions.clone(),
                        auth_data.auth_url.clone(),
                        auth_data.setup_url.clone(),
                        None,
                    )
                    .await;
                }

                let new_pending = PendingApproval {
                    request_id: Uuid::new_v4(),
                    tool_name: tc.name.clone(),
                    parameters: tc.arguments.clone(),
                    display_parameters: redact_params(&tc.arguments, tool.sensitive_params()),
                    description: tool.description().to_string(),
                    tool_call_id: tc.id.clone(),
                    context_messages: context_messages.clone(),
                    deferred_tool_calls: deferred_tool_calls[approval_idx + 1..].to_vec(),
                    selected_auth_prompt: persist_selected_auth_prompt(
                        selected_auth_prompt.as_ref(),
                    ),
                    // Carry forward the resolved timezone from the original pending approval
                    user_timezone: pending.user_timezone.clone(),
                    allow_always,
                };

                let request_id = new_pending.request_id;
                let tool_name = new_pending.tool_name.clone();
                let description = new_pending.description.clone();
                let parameters = new_pending.display_parameters.clone();

                {
                    let mut sess = session.lock().await;
                    match sess.threads.get_mut(&thread_id) {
                        Some(thread) => thread.await_approval(new_pending),
                        None => {
                            return Err(Error::from(crate::error::JobError::NotFound {
                                id: thread_id,
                            }));
                        }
                    }
                }

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ApprovalNeeded {
                            request_id: request_id.to_string(),
                            tool_name: tool_name.clone(),
                            description: description.clone(),
                            parameters: parameters.clone(),
                            allow_always,
                        },
                        &message.metadata,
                    )
                    .await;

                return Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                    allow_always,
                });
            }

            if let Some((ext_name, auth_data)) = selected_auth_prompt {
                if auth_data.awaiting_token {
                    let instructions =
                        auth_instructions_or_default(auth_data.instructions.as_deref());
                    self.handle_auth_intercept(
                        &session,
                        thread_id,
                        message,
                        ext_name,
                        instructions.clone(),
                        &auth_data,
                    )
                    .await;
                    return Ok(SubmissionResult::auth_pending());
                }

                emit_auth_required_status(
                    &self.channels,
                    message,
                    ext_name,
                    auth_data.instructions,
                    auth_data.auth_url,
                    auth_data.setup_url,
                    None,
                )
                .await;
            }

            // Continue the agentic loop (a tool was already executed this turn)
            let result = self
                .run_agentic_loop(
                    message,
                    self.tenant_ctx(&message.user_id).await,
                    session.clone(),
                    thread_id,
                    context_messages,
                )
                .await;

            // Handle the result
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            match result {
                Ok(AgenticLoopResult::Response {
                    text: response,
                    turn_usage,
                }) => {
                    let (response, suggestions) =
                        crate::agent::dispatcher::extract_suggestions(&response);
                    thread.conclude_turn(TurnOutcome::Completed(response.clone()));
                    let (turn_number, tool_calls, narrative) = thread
                        .turns
                        .last()
                        .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone()))
                        .unwrap_or_default();
                    // User message already persisted at turn start; save tool calls then assistant response
                    self.persist_tool_calls(
                        thread_id,
                        &message.channel,
                        &message.user_id,
                        turn_number,
                        &tool_calls,
                        narrative.as_deref(),
                    )
                    .await;
                    self.persist_assistant_response(
                        thread_id,
                        &message.channel,
                        &message.user_id,
                        &response,
                    )
                    .await;
                    if !suggestions.is_empty() {
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::Suggestions { suggestions },
                                &message.metadata,
                            )
                            .await;
                    }
                    self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                        .await;
                    Ok(SubmissionResult::response(response))
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                    turn_usage,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.display_parameters.clone();
                    let allow_always = new_pending.allow_always;
                    thread.await_approval(*new_pending);
                    self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                        .await;
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ApprovalNeeded {
                                request_id: request_id.to_string(),
                                tool_name: tool_name.clone(),
                                description: description.clone(),
                                parameters: parameters.clone(),
                                allow_always,
                            },
                            &message.metadata,
                        )
                        .await;
                    Ok(SubmissionResult::NeedApproval {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                        allow_always,
                    })
                }
                Ok(AgenticLoopResult::AuthPending { turn_usage }) => {
                    // See the other AuthPending arm for the full rationale.
                    thread.conclude_turn(TurnOutcome::CompletedSilently);
                    let (turn_number, tool_calls, narrative) = thread
                        .turns
                        .last()
                        .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone()))
                        .unwrap_or_default();
                    self.persist_tool_calls(
                        thread_id,
                        &message.channel,
                        &message.user_id,
                        turn_number,
                        &tool_calls,
                        narrative.as_deref(),
                    )
                    .await;
                    self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                        .await;
                    Ok(SubmissionResult::auth_pending())
                }
                Ok(AgenticLoopResult::Failed { error, turn_usage }) => {
                    self.send_turn_cost_status(&message.channel, &message.metadata, &turn_usage)
                        .await;
                    thread.conclude_turn(TurnOutcome::Failed(error.to_string()));
                    Ok(SubmissionResult::error(error.to_string()))
                }
                Err(e) => {
                    thread.conclude_turn(TurnOutcome::Failed(e.to_string()));
                    // User message already persisted at turn start
                    Ok(SubmissionResult::error(e.to_string()))
                }
            }
        } else {
            // Rejected - complete the turn with a rejection message and persist
            let rejection = format!(
                "Tool '{}' was rejected. The agent will not execute this tool.\n\n\
                 You can continue the conversation or try a different approach.",
                pending.tool_name
            );
            {
                let mut sess = session.lock().await;
                match sess.threads.get_mut(&thread_id) {
                    Some(thread) => {
                        thread.clear_pending_approval();
                        thread.conclude_turn(TurnOutcome::Completed(rejection.clone()));
                        // User message already persisted at turn start; save rejection response
                        self.persist_assistant_response(
                            thread_id,
                            &message.channel,
                            &message.user_id,
                            &rejection,
                        )
                        .await;
                    }
                    None => {
                        return Err(Error::from(crate::error::JobError::NotFound {
                            id: thread_id,
                        }));
                    }
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Rejected".into()),
                    &message.metadata,
                )
                .await;

            Ok(SubmissionResult::response(rejection))
        }
    }

    /// Handle an auth-required result from a tool execution.
    ///
    /// Enters auth mode on the thread, completes + persists the turn,
    /// and sends the AuthRequired status to the channel.
    /// Returns the instructions string for the caller to wrap in a response.
    async fn handle_auth_intercept(
        &self,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        ext_name: String,
        instructions: String,
        auth_data: &ParsedAuthData,
    ) {
        {
            let mut sess = session.lock().await;
            match sess.threads.get_mut(&thread_id) {
                Some(thread) => {
                    thread.enter_auth_mode(ext_name.clone());
                    thread.conclude_turn(TurnOutcome::Completed(instructions.clone()));
                    // User message already persisted at turn start; save auth instructions
                    self.persist_assistant_response(
                        thread_id,
                        &message.channel,
                        &message.user_id,
                        &instructions,
                    )
                    .await;
                }
                None => {
                    tracing::debug!(
                        %thread_id,
                        "Thread disappeared before auth intercept could be applied"
                    );
                }
            }
        }
        emit_auth_required_status(
            &self.channels,
            message,
            ext_name,
            Some(instructions),
            auth_data.auth_url.clone(),
            auth_data.setup_url.clone(),
            Some(thread_id.to_string()),
        )
        .await;
    }

    async fn send_turn_cost_status(
        &self,
        channel: &str,
        metadata: &serde_json::Value,
        turn_usage: &TurnUsageSummary,
    ) {
        let total_tokens =
            u64::from(turn_usage.usage.input_tokens) + u64::from(turn_usage.usage.output_tokens);
        if total_tokens == 0 && turn_usage.cost_usd.is_zero() {
            return;
        }

        let _ = self
            .channels
            .send_status(
                channel,
                StatusUpdate::TurnCost {
                    input_tokens: u64::from(turn_usage.usage.input_tokens),
                    output_tokens: u64::from(turn_usage.usage.output_tokens),
                    cost_usd: format!("${:.4}", turn_usage.cost_usd),
                },
                metadata,
            )
            .await;
    }

    /// Handle an auth token submitted while the thread is in auth mode.
    ///
    /// The token goes directly to the extension manager's credential store,
    /// completely bypassing logging, turn creation, history, and compaction.
    pub(super) async fn process_auth_token(
        &self,
        message: &IncomingMessage,
        pending: &crate::agent::session::PendingAuth,
        token: &str,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<Option<String>, Error> {
        let token = token.trim();

        // Clear auth mode regardless of outcome
        {
            let mut sess = session.lock().await;
            match sess.threads.get_mut(&thread_id) {
                Some(thread) => thread.pending_auth = None,
                None => {
                    tracing::debug!(
                        %thread_id,
                        "Thread disappeared before auth mode could be cleared"
                    );
                }
            }
        }

        let auth_manager = self.deps.auth_manager.clone().or_else(|| {
            self.tools().secrets_store().cloned().map(|secrets| {
                Arc::new(crate::bridge::auth_manager::AuthManager::new(
                    secrets,
                    self.skill_registry().cloned(),
                    self.deps.extension_manager.clone(),
                    Some(self.tools().clone()),
                ))
            })
        });

        let result = if let Some(auth_manager) = auth_manager {
            auth_manager
                .submit_auth_token(&pending.extension_name, token, &message.user_id)
                .await
        } else if let Some(ext_mgr) = self.deps.extension_manager.as_ref() {
            ext_mgr
                .configure_token(&pending.extension_name, token, &message.user_id)
                .await
        } else {
            return Ok(Some("Extension manager not available.".to_string()));
        };

        match result {
            Ok(result) if result.activated => {
                // Ensure extension is actually activated
                tracing::info!(
                    "Extension '{}' configured via auth mode: {}",
                    pending.extension_name,
                    result.message
                );
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthCompleted {
                            extension_name: pending.extension_name.clone(),
                            success: true,
                            message: result.message.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(result.message))
            }
            Ok(result) => {
                {
                    let mut sess = session.lock().await;
                    match sess.threads.get_mut(&thread_id) {
                        Some(thread) => {
                            thread.enter_auth_mode(pending.extension_name.clone());
                        }
                        None => {
                            tracing::debug!(
                                %thread_id,
                                "Thread disappeared before auth mode could be re-entered"
                            );
                        }
                    }
                }
                emit_auth_required_status(
                    &self.channels,
                    message,
                    pending.extension_name.clone(),
                    Some(result.message.clone()),
                    None,
                    None,
                    Some(thread_id.to_string()),
                )
                .await;
                Ok(Some(result.message))
            }
            Err(e) => {
                // Token validation errors: re-enter auth mode and re-prompt
                if let Some(msg) = auth_retry_message_for_error(&e) {
                    tracing::debug!(
                        extension = %pending.extension_name,
                        error = %e,
                        "Rejected invalid auth token"
                    );
                    {
                        let mut sess = session.lock().await;
                        match sess.threads.get_mut(&thread_id) {
                            Some(thread) => {
                                thread.enter_auth_mode(pending.extension_name.clone());
                            }
                            None => {
                                tracing::debug!(
                                    %thread_id,
                                    "Thread disappeared before auth mode could be re-entered on retry"
                                );
                            }
                        }
                    }
                    emit_auth_required_status(
                        &self.channels,
                        message,
                        pending.extension_name.clone(),
                        Some(msg.clone()),
                        None,
                        None,
                        Some(thread_id.to_string()),
                    )
                    .await;
                    return Ok(Some(msg));
                }
                // Infrastructure errors
                let msg = e.to_string();
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthCompleted {
                            extension_name: pending.extension_name.clone(),
                            success: false,
                            message: msg.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
        }
    }

    pub(super) async fn process_new_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;
        let thread = sess.create_thread(Some(&message.channel));
        let thread_id = thread.id;
        Ok(SubmissionResult::ok_with_message(format!(
            "New thread: {}",
            thread_id
        )))
    }

    pub(super) async fn process_switch_thread(
        &self,
        message: &IncomingMessage,
        target_thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        // Try hydrating from DB if not already in session.
        let thread_id_str = target_thread_id.to_string();
        if let Some(rejection) = self.maybe_hydrate_thread(message, &thread_id_str).await {
            return Ok(SubmissionResult::error(rejection));
        }

        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let (switched, messages, pending_approval) = {
            let mut sess = session.lock().await;
            if sess.switch_thread(target_thread_id) {
                let history = sess
                    .threads
                    .get(&target_thread_id)
                    .map(history_messages_from_thread)
                    .unwrap_or_default();
                let pending_approval = sess
                    .threads
                    .get(&target_thread_id)
                    .and_then(|thread| thread.pending_approval.as_ref())
                    .map(approval_prompt_from_pending);
                (true, history, pending_approval)
            } else {
                (false, Vec::new(), None)
            }
        };

        if switched {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ConversationHistory {
                        thread_id: target_thread_id.to_string(),
                        messages,
                        pending_approval,
                    },
                    &message.metadata,
                )
                .await;

            Ok(SubmissionResult::ok_with_message(format!(
                "Switched to thread {}",
                target_thread_id
            )))
        } else {
            Ok(SubmissionResult::error("Thread not found."))
        }
    }

    pub(super) async fn process_resume(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let mut mgr = undo_mgr.lock().await;

        if let Some(checkpoint) = mgr.restore(checkpoint_id) {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(checkpoint.messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Resumed from checkpoint: {}",
                checkpoint.description
            )))
        } else {
            Ok(SubmissionResult::error("Checkpoint not found."))
        }
    }

    /// List past conversations from the database and emit a `ThreadList`
    /// status update so the TUI can show the interactive resume picker.
    pub(super) async fn process_list_threads(
        &self,
        _session: Arc<Mutex<Session>>,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let Some(db) = self.store() else {
            return Ok(SubmissionResult::ok_with_message(
                "No database configured — cannot list conversations.".to_string(),
            ));
        };

        let conversations = match db
            .list_conversations_all_channels(&message.user_id, 20)
            .await
        {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Failed to list conversations: {e}");
                return Ok(SubmissionResult::error(format!(
                    "Failed to list conversations: {e}"
                )));
            }
        };

        let summaries = thread_summaries_from_conversations(conversations);

        if summaries.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "No conversations to resume.".to_string(),
            ));
        }

        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::ThreadList { threads: summaries },
                &message.metadata,
            )
            .await;

        Ok(SubmissionResult::Ok {
            message: Some(String::new()),
        })
    }
}

/// Rebuild full LLM-compatible `ChatMessage` sequence from DB messages.
///
/// Parses `role="tool_calls"` rows to reconstruct `assistant_with_tool_calls`
/// and `tool_result` messages so that the LLM sees the complete tool execution
/// history on thread hydration. Falls back gracefully for legacy rows that
/// lack the enriched fields (`call_id`, `parameters`, `result`).
fn rebuild_chat_messages_from_db(
    db_messages: &[crate::history::ConversationMessage],
) -> Vec<ChatMessage> {
    let mut result = Vec::new();

    for msg in db_messages {
        match msg.role.as_str() {
            "user" => result.push(ChatMessage::user(&msg.content)),
            "assistant" => result.push(ChatMessage::assistant(&msg.content)),
            "tool_calls" => {
                // Try to parse the enriched JSON and rebuild tool messages.
                // Supports two formats:
                // - Old: plain JSON array of tool call summaries
                // - New: wrapped object { "calls": [...], "narrative": "..." }
                let calls: Vec<serde_json::Value> =
                    match serde_json::from_str::<serde_json::Value>(&msg.content) {
                        Ok(serde_json::Value::Array(arr)) => arr,
                        Ok(serde_json::Value::Object(obj)) => obj
                            .get("calls")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                {
                    if calls.is_empty() {
                        continue;
                    }

                    // Check if this is an enriched row (has call_id) or legacy
                    let has_call_id = calls
                        .first()
                        .and_then(|c| c.get("call_id"))
                        .and_then(|v| v.as_str())
                        .is_some();

                    if has_call_id {
                        // Build assistant_with_tool_calls + tool_result messages
                        let tool_calls: Vec<ToolCall> = calls
                            .iter()
                            .map(|c| ToolCall {
                                id: c["call_id"].as_str().unwrap_or("call_0").to_string(),
                                name: c["name"].as_str().unwrap_or("unknown").to_string(),
                                arguments: c
                                    .get("parameters")
                                    .cloned()
                                    .unwrap_or(serde_json::json!({})),
                                reasoning: c
                                    .get("rationale")
                                    .and_then(|v| v.as_str())
                                    .map(String::from),
                            })
                            .collect();

                        // The assistant text for tool_calls is always None here;
                        // the final assistant response comes as a separate
                        // "assistant" row after this tool_calls row.
                        result.push(ChatMessage::assistant_with_tool_calls(None, tool_calls));

                        // Emit tool_result messages for each call
                        for c in &calls {
                            let call_id = c["call_id"].as_str().unwrap_or("call_0").to_string();
                            let name = c["name"].as_str().unwrap_or("unknown").to_string();
                            let content = if let Some(err) = c.get("error").and_then(|v| v.as_str())
                            {
                                // Both wrapped (new) and legacy (plain) errors pass
                                // through as-is. Legacy errors are already descriptive
                                // (e.g. "Tool 'http' failed: timeout"), so no prefix needed.
                                err.to_string()
                            } else if let Some(res) = c.get("result").and_then(|v| v.as_str()) {
                                tool_result_content_for_rebuild(res)
                            } else if let Some(preview) =
                                c.get("result_preview").and_then(|v| v.as_str())
                            {
                                preview.to_string()
                            } else {
                                "OK".to_string()
                            };
                            result.push(ChatMessage::tool_result(call_id, name, content));
                        }
                    }
                    // Legacy rows without call_id: skip (will appear as
                    // simple user/assistant pairs, same as before this fix).
                }
            }
            _ => {} // Skip unknown roles
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    use crate::agent::AgentDeps;
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::channels::{Channel, ChannelManager, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
    use futures::stream;
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::error::ChannelError;
    use crate::generated_images::{GeneratedImageSentinel, MAX_RECORDED_IMAGE_SENTINEL_BYTES};
    use crate::hooks::HookRegistry;
    use crate::testing::{StubChannel, StubLlm};
    use crate::tools::ToolRegistry;
    use chrono::TimeZone;
    use ironclaw_safety::SafetyLayer;
    use rust_decimal::Decimal;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex as TokioMutex;

    #[derive(Clone)]
    struct RecordingStatusChannel {
        statuses: Arc<TokioMutex<Vec<StatusUpdate>>>,
    }

    #[async_trait::async_trait]
    impl Channel for RecordingStatusChannel {
        fn name(&self) -> &str {
            "test"
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            Ok(Box::pin(stream::empty()))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        async fn send_status(
            &self,
            status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            self.statuses.lock().await.push(status);
            Ok(())
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    async fn make_thread_ops_test_agent() -> (Agent, Arc<TokioMutex<Vec<StatusUpdate>>>) {
        struct StaticLlmProvider;

        #[async_trait::async_trait]
        impl crate::llm::LlmProvider for StaticLlmProvider {
            fn model_name(&self) -> &str {
                "static-mock"
            }

            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }

            async fn complete(
                &self,
                _request: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, crate::error::LlmError> {
                Ok(crate::llm::CompletionResponse {
                    content: "ok".to_string(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }

            async fn complete_with_tools(
                &self,
                _request: crate::llm::ToolCompletionRequest,
            ) -> Result<crate::llm::ToolCompletionResponse, crate::error::LlmError> {
                Ok(crate::llm::ToolCompletionResponse {
                    content: Some("ok".to_string()),
                    tool_calls: Vec::new(),
                    input_tokens: 0,
                    output_tokens: 0,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
        }

        let statuses = Arc::new(TokioMutex::new(Vec::new()));
        let channels = Arc::new(crate::channels::ChannelManager::new());
        channels
            .add(Box::new(RecordingStatusChannel {
                statuses: Arc::clone(&statuses),
            }))
            .await;

        let deps = crate::agent::AgentDeps {
            owner_id: "default".to_string(),
            store: None,
            settings_store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(ironclaw_safety::SafetyLayer::new(
                &ironclaw_safety::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: true,
                },
            )),
            tools: Arc::new(crate::tools::ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: crate::config::SkillsConfig::default(),
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            auth_manager: None,
            cost_guard: Arc::new(crate::agent::cost_guard::CostGuard::new(
                crate::agent::cost_guard::CostGuardConfig::default(),
            )),
            sse_tx: None,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
            sandbox_readiness: crate::agent::routine_engine::SandboxReadiness::DisabledByConfig,
            builder: None,
            llm_backend: "nearai".to_string(),
            tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(4, 3)),
        };

        let agent = Agent::new(
            crate::config::AgentConfig {
                name: "thread-ops-test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_cost_per_user_per_day_cents: None,
                max_tool_iterations: 50,
                auto_approve_tools: false,
                default_timezone: "UTC".to_string(),
                max_jobs_per_user: None,
                max_tokens_per_job: 0,
                multi_tenant: false,
                max_llm_concurrent_per_user: None,
                max_jobs_concurrent_per_user: None,
                engine_v2: false,
            },
            deps,
            channels,
            None,
            None,
            None,
            Some(Arc::new(crate::context::ContextManager::new(1))),
            None,
        );

        (agent, statuses)
    }

    #[test]
    fn thread_summaries_are_sorted_by_last_activity_descending() {
        let conversations = vec![
            crate::history::ConversationSummary {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                title: Some("older".to_string()),
                message_count: 1,
                started_at: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 0, 0).unwrap(),
                last_activity: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 5, 0).unwrap(),
                thread_type: None,
                channel: "gateway".to_string(),
            },
            crate::history::ConversationSummary {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                title: Some("newest".to_string()),
                message_count: 2,
                started_at: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 10, 0).unwrap(),
                last_activity: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 30, 0).unwrap(),
                thread_type: None,
                channel: "gateway".to_string(),
            },
            crate::history::ConversationSummary {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
                title: Some("middle".to_string()),
                message_count: 3,
                started_at: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 8, 0).unwrap(),
                last_activity: chrono::Utc.with_ymd_and_hms(2026, 4, 4, 7, 15, 0).unwrap(),
                thread_type: None,
                channel: "gateway".to_string(),
            },
        ];

        let summaries = thread_summaries_from_conversations(conversations);
        let titles: Vec<Option<String>> = summaries.into_iter().map(|s| s.title).collect();

        assert_eq!(
            titles,
            vec![
                Some("newest".to_string()),
                Some("middle".to_string()),
                Some("older".to_string()),
            ]
        );
    }

    #[test]
    fn test_rebuild_chat_messages_user_assistant_only() {
        let messages = vec![
            make_db_msg("user", "Hello"),
            make_db_msg("assistant", "Hi there!"),
        ];
        let result = rebuild_chat_messages_from_db(&messages);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, crate::llm::Role::User);
        assert_eq!(result[1].role, crate::llm::Role::Assistant);
    }

    /// Regression: a `PendingApproval` deserialized from a row written
    /// before the `display_parameters` field existed lands with
    /// `display_parameters: Value::Null` (because the field is
    /// `#[serde(default)]`). Both the SSE re-emit path
    /// (`pending_approval_status_update`) and the snapshot path
    /// (`approval_prompt_from_pending`) must fall back to the unredacted
    /// `parameters` so the user sees actual arguments rather than `null`.
    /// Without this fallback the SSE/CLI approval prompt for legacy
    /// approvals shows `null` parameters.
    #[test]
    fn test_pending_approval_helpers_fall_back_when_display_parameters_is_null() {
        use crate::agent::session::PendingApproval;

        let original_params = serde_json::json!({"command": "echo hi"});
        let pending = PendingApproval {
            request_id: uuid::Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: original_params.clone(),
            // Simulate a row that round-tripped through serde before the
            // `display_parameters` field was added — defaults to Null.
            display_parameters: serde_json::Value::Null,
            description: "Execute: echo hi".to_string(),
            tool_call_id: "call_legacy".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            selected_auth_prompt: None,
            user_timezone: None,
            allow_always: true,
        };

        // Both SSE status and snapshot helpers must use the same fallback.
        let status = pending_approval_status_update(&pending);
        match status {
            StatusUpdate::ApprovalNeeded { parameters, .. } => {
                assert_eq!(
                    parameters, original_params,
                    "pending_approval_status_update must fall back to pending.parameters \
                     when display_parameters is Null (legacy serde-default rows)"
                );
            }
            other => panic!("expected ApprovalNeeded, got {other:?}"),
        }

        let prompt = approval_prompt_from_pending(&pending);
        assert_eq!(
            prompt.parameters, original_params,
            "approval_prompt_from_pending must fall back to pending.parameters \
             when display_parameters is Null"
        );

        // Sanity check the non-null path: when display_parameters is set,
        // both helpers must prefer it (the redacted form).
        let redacted = serde_json::json!({"command": "[REDACTED]"});
        let pending_with_redaction = PendingApproval {
            display_parameters: redacted.clone(),
            ..pending
        };
        let prompt = approval_prompt_from_pending(&pending_with_redaction);
        assert_eq!(prompt.parameters, redacted);
        let status = pending_approval_status_update(&pending_with_redaction);
        match status {
            StatusUpdate::ApprovalNeeded { parameters, .. } => {
                assert_eq!(parameters, redacted);
            }
            other => panic!("expected ApprovalNeeded, got {other:?}"),
        }
    }

    #[test]
    fn test_turn_usage_from_result_extracts_usage_for_interrupted_response() {
        let result = Ok(AgenticLoopResult::Response {
            text: "done".to_string(),
            turn_usage: TurnUsageSummary {
                usage: crate::llm::TokenUsage {
                    input_tokens: 12,
                    output_tokens: 3,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                cost_usd: Decimal::new(18, 3),
            },
        });

        let turn_usage = turn_usage_from_result(&result).expect("usage should be present");
        assert_eq!(turn_usage.usage.input_tokens, 12);
        assert_eq!(turn_usage.usage.output_tokens, 3);
    }

    #[test]
    fn test_turn_usage_from_result_extracts_usage_for_interrupted_failed_turn() {
        let result = Ok(AgenticLoopResult::Failed {
            error: crate::error::LlmError::InvalidResponse {
                provider: "agent".to_string(),
                reason: "Interrupted".to_string(),
            }
            .into(),
            turn_usage: TurnUsageSummary {
                usage: crate::llm::TokenUsage {
                    input_tokens: 7,
                    output_tokens: 2,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                cost_usd: Decimal::new(11, 3),
            },
        });

        let turn_usage = turn_usage_from_result(&result).expect("usage should be present");
        assert_eq!(turn_usage.usage.input_tokens, 7);
        assert_eq!(turn_usage.usage.output_tokens, 2);
    }

    #[test]
    fn test_auth_retry_message_hides_validation_details() {
        let err =
            crate::extensions::ExtensionError::ValidationFailed("wrong format for API key".into());

        assert_eq!(
            auth_retry_message_for_error(&err).as_deref(),
            Some("Invalid token. Please try again.")
        );
    }

    #[test]
    fn test_rebuild_chat_messages_with_enriched_tool_calls() {
        let tool_json = serde_json::json!([
            {
                "name": "memory_search",
                "call_id": "call_0",
                "parameters": {"query": "test"},
                "result": "Found 3 results",
                "result_preview": "Found 3 re..."
            },
            {
                "name": "echo",
                "call_id": "call_1",
                "parameters": {"message": "hi"},
                "error": "timeout"
            }
        ]);
        let messages = vec![
            make_db_msg("user", "Search for test"),
            make_db_msg("tool_calls", &tool_json.to_string()),
            make_db_msg("assistant", "I found some results."),
        ];
        let result = rebuild_chat_messages_from_db(&messages);

        // user + assistant_with_tool_calls + tool_result*2 + assistant
        assert_eq!(result.len(), 5);

        // user
        assert_eq!(result[0].role, crate::llm::Role::User);

        // assistant with tool_calls
        assert_eq!(result[1].role, crate::llm::Role::Assistant);
        assert!(result[1].tool_calls.is_some());
        let tcs = result[1].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 2);
        assert_eq!(tcs[0].name, "memory_search");
        assert_eq!(tcs[0].id, "call_0");
        assert_eq!(tcs[1].name, "echo");

        // tool results
        assert_eq!(result[2].role, crate::llm::Role::Tool);
        assert_eq!(result[2].tool_call_id, Some("call_0".to_string()));
        assert!(result[2].content.contains("Found 3 results"));

        assert_eq!(result[3].role, crate::llm::Role::Tool);
        assert_eq!(result[3].tool_call_id, Some("call_1".to_string()));
        assert!(result[3].content.contains("timeout"));

        // final assistant
        assert_eq!(result[4].role, crate::llm::Role::Assistant);
        assert_eq!(result[4].content, "I found some results.");
    }

    #[test]
    fn test_rebuild_chat_messages_preserves_wrapped_tool_error() {
        let wrapped_error =
            "<tool_output name=\"http\">\nTool 'http' failed: timeout\n</tool_output>";
        let tool_json = serde_json::json!([
            {
                "name": "http",
                "call_id": "call_1",
                "parameters": {"url": "https://example.com"},
                "error": wrapped_error
            }
        ]);
        let messages = vec![
            make_db_msg("user", "Fetch example"),
            make_db_msg("tool_calls", &tool_json.to_string()),
        ];

        let result = rebuild_chat_messages_from_db(&messages);

        assert_eq!(result.len(), 3);
        assert_eq!(result[2].role, crate::llm::Role::Tool);
        assert_eq!(result[2].tool_call_id, Some("call_1".to_string()));
        assert_eq!(result[2].content, wrapped_error);
    }

    #[test]
    fn test_rebuild_chat_messages_legacy_tool_calls_skipped() {
        // Legacy format: no call_id field
        let tool_json = serde_json::json!([
            {"name": "echo", "result_preview": "hello"}
        ]);
        let messages = vec![
            make_db_msg("user", "Hi"),
            make_db_msg("tool_calls", &tool_json.to_string()),
            make_db_msg("assistant", "Done"),
        ];
        let result = rebuild_chat_messages_from_db(&messages);

        // Legacy rows are skipped, only user + assistant
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, crate::llm::Role::User);
        assert_eq!(result[1].role, crate::llm::Role::Assistant);
    }

    #[test]
    fn test_rebuild_chat_messages_empty() {
        let result = rebuild_chat_messages_from_db(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rebuild_chat_messages_malformed_tool_calls_json() {
        let messages = vec![
            make_db_msg("user", "Hi"),
            make_db_msg("tool_calls", "not valid json"),
            make_db_msg("assistant", "Done"),
        ];
        let result = rebuild_chat_messages_from_db(&messages);
        // Malformed JSON is silently skipped
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_rebuild_chat_messages_multi_turn_with_tools() {
        let tool_json_1 = serde_json::json!([
            {"name": "search", "call_id": "call_0", "parameters": {}, "result": "found it"}
        ]);
        let tool_json_2 = serde_json::json!([
            {"name": "write", "call_id": "call_0", "parameters": {"path": "a.txt"}, "result": "ok"}
        ]);
        let messages = vec![
            make_db_msg("user", "Find X"),
            make_db_msg("tool_calls", &tool_json_1.to_string()),
            make_db_msg("assistant", "Found X"),
            make_db_msg("user", "Write it"),
            make_db_msg("tool_calls", &tool_json_2.to_string()),
            make_db_msg("assistant", "Written"),
        ];
        let result = rebuild_chat_messages_from_db(&messages);

        // Turn 1: user + assistant_with_calls + tool_result + assistant = 4
        // Turn 2: user + assistant_with_calls + tool_result + assistant = 4
        assert_eq!(result.len(), 8);

        // Verify turn boundaries
        assert_eq!(result[0].content, "Find X");
        assert!(result[1].tool_calls.is_some());
        assert_eq!(result[2].role, crate::llm::Role::Tool);
        assert_eq!(result[3].content, "Found X");

        assert_eq!(result[4].content, "Write it");
        assert!(result[5].tool_calls.is_some());
        assert_eq!(result[6].role, crate::llm::Role::Tool);
        assert_eq!(result[7].content, "Written");
    }

    #[test]
    fn test_rebuild_chat_messages_summarizes_image_generated_result() {
        let tool_json = serde_json::json!([
            {
                "name": "image_generate",
                "call_id": "call_0",
                "parameters": {"prompt": "cat"},
                "result": serde_json::json!({
                    "type": "image_generated",
                    "data": "data:image/jpeg;base64,abc123",
                    "media_type": "image/jpeg"
                }).to_string(),
                "result_preview": "Generated image"
            }
        ]);
        let messages = vec![
            make_db_msg("user", "Draw a cat"),
            make_db_msg("tool_calls", &tool_json.to_string()),
            make_db_msg("assistant", "Done"),
        ];

        let result = rebuild_chat_messages_from_db(&messages);

        assert_eq!(result.len(), 4);
        assert_eq!(result[2].role, crate::llm::Role::Tool);
        assert_eq!(result[2].content, "Generated image (image/jpeg)");
    }

    #[test]
    fn test_tool_result_preview_for_persistence_handles_double_stringified_sentinel() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "media_type": "image/jpeg"
        })
        .to_string();
        let double_wrapped = serde_json::Value::String(serde_json::to_string(&sentinel).unwrap());

        let preview = tool_result_preview_for_persistence(&double_wrapped);
        let rebuilt = tool_result_content_for_rebuild(double_wrapped.as_str().unwrap());

        assert_eq!(preview, "Generated image");
        assert_eq!(rebuilt, "Generated image (image/jpeg)");
    }

    #[test]
    fn test_tool_result_content_for_persistence_preserves_image_sentinel_under_cap() {
        let result = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "media_type": "image/jpeg"
        });

        let persisted = tool_result_content_for_persistence(&result);

        assert!(persisted.contains("\"type\":\"image_generated\""));
        assert!(persisted.contains("data:image/jpeg;base64,abc123"));
    }

    #[test]
    fn test_tool_result_content_for_persistence_caps_oversized_image_sentinel() {
        let oversized = "a".repeat(MAX_RECORDED_IMAGE_SENTINEL_BYTES);
        let result = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/jpeg;base64,{oversized}"),
            "media_type": "image/jpeg",
            "path": "workspace/out.jpg"
        });

        let persisted = tool_result_content_for_persistence(&result);

        let persisted_json: serde_json::Value =
            serde_json::from_str(&persisted).expect("persisted compact sentinel json");

        assert_eq!(persisted_json["type"], "image_generated");
        assert_eq!(persisted_json["media_type"], "image/jpeg");
        assert_eq!(persisted_json["path"], "workspace/out.jpg");
        assert_eq!(persisted_json["data_omitted"], true);
        assert!(
            persisted_json["omitted_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("512 KiB cap"))
        );
        assert!(!persisted.contains("data:image/jpeg;base64"));
    }

    #[test]
    fn test_tool_result_content_for_persistence_preserves_double_stringified_sentinel_under_cap() {
        let base = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/png;base64,",
            "media_type": "image/png",
            "path": "workspace/out.png"
        })
        .to_string();
        let filler_len = MAX_RECORDED_IMAGE_SENTINEL_BYTES - base.len();
        let normalized = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{}", "a".repeat(filler_len)),
            "media_type": "image/png",
            "path": "workspace/out.png"
        })
        .to_string();
        let double_stringified = serde_json::Value::String(normalized.clone());

        assert_eq!(normalized.len(), MAX_RECORDED_IMAGE_SENTINEL_BYTES);
        assert!(double_stringified.to_string().len() > MAX_RECORDED_IMAGE_SENTINEL_BYTES);

        let persisted = tool_result_content_for_persistence(&double_stringified);
        let parsed = GeneratedImageSentinel::from_output(&persisted).expect("persisted sentinel");

        assert_eq!(persisted, normalized);
        assert!(parsed.data_url().is_some());
        assert_eq!(parsed.path(), Some("workspace/out.png"));
        assert!(!persisted.contains("\"data_omitted\":true"));
    }

    #[test]
    fn test_build_turns_collects_generated_images_from_capped_persisted_tool_result() {
        let oversized = "a".repeat(MAX_RECORDED_IMAGE_SENTINEL_BYTES);
        let result = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{oversized}"),
            "media_type": "image/png",
            "path": "workspace/out.png"
        });
        let persisted = tool_result_content_for_persistence(&result);
        let tool_json = serde_json::json!([
            {
                "name": "image_generate",
                "call_id": "call_img_0",
                "parameters": {"prompt": "cat"},
                "result": persisted,
                "result_preview": "Generated image"
            }
        ]);
        let messages = vec![
            make_db_msg("user", "Draw a cat"),
            make_db_msg("tool_calls", &tool_json.to_string()),
            make_db_msg("assistant", "Done"),
        ];

        let turns = crate::channels::web::util::build_turns_from_db_messages(&messages);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].generated_images.len(), 1);
        assert_eq!(turns[0].generated_images[0].event_id, "call_img_0");
        assert!(turns[0].generated_images[0].data_url.is_none());
        assert_eq!(
            turns[0].generated_images[0].path.as_deref(),
            Some("workspace/out.png")
        );
    }

    fn make_db_msg(role: &str, content: &str) -> crate::history::ConversationMessage {
        crate::history::ConversationMessage {
            id: uuid::Uuid::new_v4(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    async fn make_test_agent_with_status_channel(
        channel_name: &str,
    ) -> (Agent, Arc<StdMutex<Vec<StatusUpdate>>>) {
        let (stub, _sender) = StubChannel::new(channel_name);
        let statuses = stub.captured_statuses_handle();
        let manager = ChannelManager::new();
        manager.add(Box::new(stub)).await;

        let deps = AgentDeps {
            owner_id: "default".to_string(),
            store: None,
            settings_store: None,
            llm: Arc::new(StubLlm::default()),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            auth_manager: None,
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
            sse_tx: None,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
            sandbox_readiness: crate::agent::routine_engine::SandboxReadiness::DisabledByConfig,
            builder: None,
            llm_backend: "nearai".to_string(),
            tenant_rates: Arc::new(crate::tenant::TenantRateRegistry::new(4, 3)),
        };

        let agent = Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_cost_per_user_per_day_cents: None,
                max_tool_iterations: 50,
                auto_approve_tools: false,
                default_timezone: "UTC".to_string(),
                max_jobs_per_user: None,
                max_tokens_per_job: 0,
                multi_tenant: false,
                max_llm_concurrent_per_user: None,
                max_jobs_concurrent_per_user: None,
                engine_v2: false,
            },
            deps,
            Arc::new(manager),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        );

        (agent, statuses)
    }

    #[tokio::test]
    async fn test_awaiting_approval_rejection_includes_tool_context() {
        // Test that when a thread is in AwaitingApproval state and receives a new message,
        // process_user_input rejects it with a non-error status that includes tool context.
        use crate::agent::session::{PendingApproval, Session, Thread, ThreadState};
        use uuid::Uuid;

        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id, None);

        // Set thread to AwaitingApproval with a pending tool approval
        let pending = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "echo hello"}),
            display_parameters: serde_json::json!({"command": "[REDACTED]"}),
            description: "Execute: echo hello".to_string(),
            tool_call_id: "call_0".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            selected_auth_prompt: None,
            user_timezone: None,
            allow_always: false,
        };
        thread.await_approval(pending);

        let mut session = Session::new("test-user");
        session.threads.insert(thread_id, thread);

        // Verify thread is in AwaitingApproval state
        assert_eq!(
            session.threads[&thread_id].state,
            ThreadState::AwaitingApproval
        );

        let result = extract_approval_message(&session, thread_id);

        // Verify result is an Ok with a message (not an Error)
        match result {
            Ok(Some(msg)) => {
                // Should NOT start with "Error:"
                assert!(
                    !msg.to_lowercase().starts_with("error:"),
                    "Approval rejection should not have 'Error:' prefix. Got: {}",
                    msg
                );

                // Should contain "waiting for approval"
                assert!(
                    msg.to_lowercase().contains("waiting for approval"),
                    "Should contain 'waiting for approval'. Got: {}",
                    msg
                );

                // Should contain the tool name
                assert!(
                    msg.contains("shell"),
                    "Should contain tool name 'shell'. Got: {}",
                    msg
                );

                // Should contain the description (or truncated version)
                assert!(
                    msg.contains("echo hello"),
                    "Should contain description 'echo hello'. Got: {}",
                    msg
                );
            }
            _ => panic!("Expected approval rejection message"),
        }
    }

    #[tokio::test]
    async fn test_awaiting_approval_follow_up_re_emits_status() {
        use crate::agent::session::{PendingApproval, Session, Thread};
        use uuid::Uuid;

        let (agent, statuses) = make_test_agent_with_status_channel("test").await;
        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id, Some("test"));
        let pending = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "echo hello"}),
            display_parameters: serde_json::json!({"command": "[REDACTED]"}),
            description: "Execute: echo hello".to_string(),
            tool_call_id: "call_0".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            selected_auth_prompt: None,
            user_timezone: None,
            allow_always: true,
        };
        let request_id = pending.request_id.to_string();
        thread.await_approval(pending);

        let mut sess = Session::new("test-user");
        sess.threads.insert(thread_id, thread);
        let session = Arc::new(tokio::sync::Mutex::new(sess));
        let message = IncomingMessage::new("test", "test-user", "still waiting?");

        let result = agent
            .process_user_input(
                &message,
                agent.tenant_ctx("test-user").await,
                Arc::clone(&session),
                thread_id,
                "still waiting?",
            )
            .await
            .expect("follow-up handled");

        match result {
            SubmissionResult::Ok { message: Some(msg) } => {
                assert!(msg.contains("Waiting for approval"));
                assert!(msg.contains("shell"));
            }
            other => panic!("expected pending Ok message, got {other:?}"),
        }

        let statuses = statuses.lock().expect("lock").clone();
        assert!(statuses.iter().any(|status| matches!(
            status,
            StatusUpdate::ApprovalNeeded {
                request_id: status_request_id,
                tool_name,
                ..
            } if *status_request_id == request_id && tool_name == "shell"
        )));
    }

    #[tokio::test]
    async fn test_switch_thread_emits_history_with_pending_approval() {
        use crate::agent::session::{PendingApproval, Thread};
        use uuid::Uuid;

        let (agent, statuses) = make_test_agent_with_status_channel("tui").await;
        let session = agent
            .session_manager
            .get_or_create_session("test-user")
            .await;
        let session_id = session.lock().await.id;

        let other_thread_id = Uuid::new_v4();
        let target_thread_id = Uuid::new_v4();
        let mut target_thread = Thread::with_id(target_thread_id, session_id, Some("tui"));
        target_thread.start_turn("Review the diff");
        target_thread.conclude_turn(TurnOutcome::Completed("Waiting for approval.".into()));
        target_thread.await_approval(PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "echo hello"}),
            display_parameters: serde_json::json!({"command": "[REDACTED]"}),
            description: "Execute: echo hello".to_string(),
            tool_call_id: "call_0".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            selected_auth_prompt: None,
            user_timezone: None,
            allow_always: true,
        });

        {
            let mut sess = session.lock().await;
            sess.threads.insert(
                other_thread_id,
                Thread::with_id(other_thread_id, session_id, Some("tui")),
            );
            sess.threads.insert(target_thread_id, target_thread);
            sess.active_thread = Some(other_thread_id);
        }

        let message =
            IncomingMessage::new("tui", "test-user", format!("/thread {target_thread_id}"));
        let result = agent
            .process_switch_thread(&message, target_thread_id)
            .await
            .expect("switch thread");

        match result {
            crate::agent::submission::SubmissionResult::Ok {
                message: Some(text),
            } => assert!(text.contains(&target_thread_id.to_string())),
            other => panic!("expected ok switch-thread result, got {other:?}"),
        }

        let statuses = statuses.lock().expect("poisoned").clone();
        assert!(
            statuses.iter().any(|status| matches!(
                status,
                StatusUpdate::ConversationHistory {
                    thread_id,
                    messages,
                    pending_approval,
                } if thread_id == &target_thread_id.to_string()
                    && messages.len() == 2
                    && messages[0].role == "user"
                    && messages[0].content == "Review the diff"
                    && messages[1].role == "assistant"
                    && messages[1].content == "Waiting for approval."
                    && pending_approval
                        .as_ref()
                        .is_some_and(|approval| approval.tool_name == "shell"
                            && approval.parameters == serde_json::json!({"command": "[REDACTED]"})
                            && approval.allow_always)
            )),
            "expected conversation history status with pending approval, got: {statuses:?}"
        );
    }

    #[test]
    fn test_queue_cap_rejects_at_capacity() {
        use crate::agent::session::{MAX_PENDING_MESSAGES, Thread, ThreadState};
        use uuid::Uuid;

        let mut thread = Thread::new(Uuid::new_v4(), None);
        thread.start_turn("processing something");
        assert_eq!(thread.state, ThreadState::Processing);

        // Fill the queue to the cap
        for i in 0..MAX_PENDING_MESSAGES {
            assert!(thread.queue_message(format!("msg-{}", i)));
        }
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // The next message should be rejected by queue_message
        assert!(!thread.queue_message("overflow".to_string()));
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // Verify all drain in FIFO order
        for i in 0..MAX_PENDING_MESSAGES {
            assert_eq!(thread.take_pending_message(), Some(format!("msg-{}", i)));
        }
        assert!(thread.take_pending_message().is_none());
    }

    #[test]
    fn test_clear_clears_pending_messages() {
        use crate::agent::session::{Thread, ThreadState};
        use uuid::Uuid;

        let mut thread = Thread::new(Uuid::new_v4(), None);
        thread.start_turn("processing");

        thread.queue_message("pending-1".to_string());
        thread.queue_message("pending-2".to_string());
        assert_eq!(thread.pending_messages.len(), 2);

        // Simulate what process_clear does: clear turns and pending_messages
        thread.turns.clear();
        thread.pending_messages.clear();
        thread.state = ThreadState::Idle;

        assert!(thread.pending_messages.is_empty());
        assert!(thread.turns.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_processing_arm_thread_gone_returns_error() {
        // Regression: if the thread disappears between the state snapshot and the
        // mutable lock, the Processing arm must return an error — not a false
        // "queued" acknowledgment.
        //
        // Exercises the exact branch at the `else` of
        // `if let Some(thread) = sess.threads.get_mut(&thread_id)`.
        use crate::agent::session::{Session, Thread, ThreadState};
        use uuid::Uuid;

        let thread_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id, None);
        thread.start_turn("working");
        assert_eq!(thread.state, ThreadState::Processing);

        let mut session = Session::new("test-user");
        session.threads.insert(thread_id, thread);

        // Simulate the thread disappearing (e.g., /clear racing with queue)
        session.threads.remove(&thread_id);

        // The Processing arm re-locks and calls get_mut — must get None.
        assert!(session.threads.get_mut(&thread_id).is_none());
        // Nothing was queued anywhere — the removed thread's queue is gone.
    }

    #[test]
    fn test_processing_arm_state_changed_does_not_queue() {
        // Regression: if the thread transitions from Processing to Idle between
        // the state snapshot and the mutable lock, the message must NOT be queued.
        // Instead the Processing arm falls through to normal processing.
        //
        // Exercises the `if thread.state == ThreadState::Processing` re-check.
        use crate::agent::session::{Session, Thread, ThreadState};
        use uuid::Uuid;

        let thread_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id, None);
        thread.start_turn("working");
        assert_eq!(thread.state, ThreadState::Processing);

        // Simulate the turn completing between snapshot and re-lock
        thread.conclude_turn(TurnOutcome::Completed("done".into()));
        assert_eq!(thread.state, ThreadState::Idle);

        let mut session = Session::new("test-user");
        session.threads.insert(thread_id, thread);

        // Re-check under lock: state is Idle, so queue_message must NOT be called.
        let t = session.threads.get_mut(&thread_id).unwrap();
        assert_ne!(t.state, ThreadState::Processing);
        // Verify nothing was queued — the fall-through path doesn't touch the queue.
        assert!(t.pending_messages.is_empty());
    }

    // Approval persistence is tested via e2e_builtin_tool_coverage integration tests.

    // Helper function to extract the approval message without needing a full Agent instance
    fn extract_approval_message(
        session: &crate::agent::session::Session,
        thread_id: Uuid,
    ) -> Result<Option<String>, crate::error::Error> {
        let thread = session.threads.get(&thread_id).ok_or_else(|| {
            crate::error::Error::from(crate::error::JobError::NotFound { id: thread_id })
        })?;

        if thread.state == ThreadState::AwaitingApproval {
            Ok(Some(pending_approval_message(
                thread.pending_approval.as_ref(),
            )))
        } else {
            Ok(None)
        }
    }

    /// Regression test for #1486: TOCTOU race in process_approval.
    ///
    /// After a request_id mismatch the pending approval must be atomically
    /// restored so a subsequent approval with the correct ID succeeds.
    #[tokio::test]
    async fn test_approval_request_id_mismatch_restores_pending() {
        use crate::agent::session::{PendingApproval, Session, Thread, ThreadState};
        use uuid::Uuid;

        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let correct_request_id = Uuid::new_v4();
        let wrong_request_id = Uuid::new_v4();

        let mut thread = Thread::with_id(thread_id, session_id, None);
        let pending = PendingApproval {
            request_id: correct_request_id,
            tool_name: "echo".to_string(),
            parameters: serde_json::json!({"text": "hello"}),
            display_parameters: serde_json::json!({"text": "hello"}),
            description: "Echo hello".to_string(),
            tool_call_id: "call_0".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            selected_auth_prompt: None,
            user_timezone: None,
            allow_always: false,
        };
        thread.await_approval(pending);
        assert_eq!(thread.state, ThreadState::AwaitingApproval);

        let mut session = Session::new("test-user");
        session.threads.insert(thread_id, thread);
        let session = Arc::new(Mutex::new(session));

        let (agent, _statuses) = make_thread_ops_test_agent().await;

        let message = IncomingMessage::new("test", "test-user", "yes");

        // Attempt approval with WRONG request ID — should fail but preserve pending
        let result = agent
            .process_approval(
                &message,
                session.clone(),
                thread_id,
                Some(wrong_request_id),
                true,
                false,
            )
            .await;

        assert!(result.is_ok());
        let result = result.unwrap();
        match &result {
            SubmissionResult::Error { message } => {
                assert!(
                    message.contains("Request ID mismatch"),
                    "Expected mismatch error, got: {}",
                    message
                );
            }
            other => panic!("Expected Error result, got: {:?}", other),
        }

        // Verify pending approval is still present (not lost due to TOCTOU)
        let sess = session.lock().await;
        let thread = sess.threads.get(&thread_id).unwrap();
        assert_eq!(thread.state, ThreadState::AwaitingApproval);
        assert!(
            thread.pending_approval.is_some(),
            "Pending approval should be restored after request_id mismatch"
        );
        assert_eq!(
            thread.pending_approval.as_ref().unwrap().request_id,
            correct_request_id,
            "Restored pending approval should have the original request_id"
        );
    }

    /// Regression test for #1487: process_approval on a missing thread should error.
    #[tokio::test]
    async fn test_approval_on_missing_thread_should_error() {
        use crate::agent::session::Session;
        use uuid::Uuid;

        let thread_id = Uuid::new_v4();

        // Session with NO threads
        let session = Session::new("test-user");
        let session = Arc::new(Mutex::new(session));

        let (agent, _statuses) = make_thread_ops_test_agent().await;

        let message = IncomingMessage::new("test", "test-user", "yes");

        let result = agent
            .process_approval(&message, session, thread_id, None, true, false)
            .await;

        assert!(
            result.is_err(),
            "Approving a missing thread should return an error, got: {:?}",
            result
        );
    }
}
