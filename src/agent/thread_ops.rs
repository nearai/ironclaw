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
    AgenticLoopResult, check_auth_required, execute_chat_tool_standalone, parse_auth_result,
};
use crate::agent::session::{MAX_PENDING_MESSAGES, PendingApproval, Session, ThreadState};
use crate::agent::submission::SubmissionResult;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::{ChatMessage, ToolCall};
use crate::tools::redact_params;
use ironclaw_common::truncate_preview;

const FORGED_THREAD_ID_ERROR: &str = "Invalid or unauthorized thread ID.";

fn requires_preexisting_uuid_thread(channel: &str) -> bool {
    // Gateway-style channels send server-issued conversation UUIDs.
    // Unknown UUIDs should be rejected instead of silently creating a new thread.
    matches!(channel, "gateway" | "test")
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

        // Create thread with the historical ID and restore messages
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        let mut thread = crate::agent::session::Thread::with_id(thread_uuid, session_id);
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
        let (thread_state, approval_context) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let approval_context = thread.pending_approval.as_ref().map(|a| {
                let desc_preview =
                    crate::agent::agent_loop::truncate_for_preview(&a.description, 80);
                (a.tool_name.clone(), desc_preview)
            });
            (thread.state, approval_context)
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
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
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
                            .any(|rule| rule.action == crate::safety::PolicyAction::Block)
                        {
                            return Ok(SubmissionResult::error("Input rejected by safety policy."));
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
                                "Message queued — will be processed after the current turn.".into(),
                            ),
                        });
                    }
                    // State changed (turn completed) — fall through to process normally.
                    // NOTE: `sess` (the Mutex guard) is dropped at the end of
                    // this `Processing` match arm, releasing the session lock
                    // before the rest of process_user_input runs. No deadlock.
                } else {
                    return Ok(SubmissionResult::error("Thread no longer exists."));
                }
            }
            ThreadState::AwaitingApproval => {
                tracing::warn!(
                    message_id = %message.id,
                    thread_id = %thread_id,
                    "Thread awaiting approval, rejecting new input"
                );
                let msg = match approval_context {
                    Some((tool_name, desc_preview)) => format!(
                        "Waiting for approval: {tool_name} — {desc_preview}. Use /interrupt to cancel."
                    ),
                    None => "Waiting for approval. Use /interrupt to cancel.".to_string(),
                };
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
            .any(|rule| rule.action == crate::safety::PolicyAction::Block)
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
            Ok(AgenticLoopResult::Response(response)) => {
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

                thread.complete_turn(&response);
                let (turn_number, tool_calls, narrative) = thread
                    .turns
                    .last()
                    .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone()))
                    .unwrap_or_default();
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Done".into()),
                        &message.metadata,
                    )
                    .await;

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

                // Emit per-turn cost summary
                {
                    let usage = self.cost_guard().model_usage().await;
                    let (total_in, total_out, total_cost) =
                        usage
                            .values()
                            .fold((0u64, 0u64, rust_decimal::Decimal::ZERO), |acc, m| {
                                (
                                    acc.0 + m.input_tokens,
                                    acc.1 + m.output_tokens,
                                    acc.2 + m.cost,
                                )
                            });
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::TurnCost {
                                input_tokens: total_in,
                                output_tokens: total_out,
                                cost_usd: format!("${:.4}", total_cost),
                            },
                            &message.metadata,
                        )
                        .await;
                }

                Ok(SubmissionResult::response(response))
            }
            Ok(AgenticLoopResult::NeedApproval { pending }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.display_parameters.clone();
                let allow_always = pending.allow_always;
                thread.await_approval(*pending);
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
            Err(e) => {
                thread.fail_turn(e.to_string());
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
            .ensure_conversation(thread_id, channel, user_id, None)
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
                    let preview = match result {
                        serde_json::Value::String(s) => truncate_preview(s, 500),
                        other => truncate_preview(&other.to_string(), 500),
                    };
                    obj["result_preview"] = serde_json::Value::String(preview);
                    // Store full result (truncated to ~1000 chars) for LLM context rebuild
                    let full_result = match result {
                        serde_json::Value::String(s) => truncate_preview(s, 1000),
                        other => truncate_preview(&other.to_string(), 1000),
                    };
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
        // Get pending approval for this thread
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

            thread.take_pending_approval()
        };

        let pending = match pending {
            Some(p) => p,
            None => {
                tracing::debug!(
                    %thread_id,
                    "Ignoring stale approval: no pending approval found"
                );
                return Ok(SubmissionResult::ok_with_message(""));
            }
        };

        // Verify request ID if provided
        if let Some(req_id) = request_id
            && req_id != pending.request_id
        {
            // Put it back and return error
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.await_approval(pending);
            }
            return Ok(SubmissionResult::error(
                "Request ID mismatch. Use the correct request ID.",
            ));
        }

        if approved {
            // If always, add to auto-approved set
            if always {
                let mut sess = session.lock().await;
                sess.auto_approve_tool(&pending.tool_name);
                tracing::info!(
                    "Auto-approved tool '{}' for session {}",
                    pending.tool_name,
                    sess.id
                );
            }

            // Reset thread state to processing
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.state = ThreadState::Processing;
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
                    StatusUpdate::ToolStarted {
                        name: pending.tool_name.clone(),
                    },
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
                        },
                        &message.metadata,
                    )
                    .await;
            }

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
                if let Some(thread) = sess.threads.get_mut(&thread_id)
                    && let Some(turn) = thread.last_turn_mut()
                {
                    if is_tool_error {
                        turn.record_tool_error_for(&pending.tool_call_id, result_content.clone());
                    } else {
                        turn.record_tool_result_for(
                            &pending.tool_call_id,
                            serde_json::json!(result_content),
                        );
                    }
                }
            }

            // If tool_auth returned awaiting_token, enter auth mode and
            // return instructions directly (skip agentic loop continuation).
            if let Some((ext_name, instructions)) =
                check_auth_required(&pending.tool_name, &tool_result)
            {
                self.handle_auth_intercept(
                    &session,
                    thread_id,
                    message,
                    &tool_result,
                    ext_name,
                    instructions.clone(),
                )
                .await;
                return Ok(SubmissionResult::response(instructions));
            }

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

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
                            StatusUpdate::ToolStarted {
                                name: tc.name.clone(),
                            },
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
                                StatusUpdate::ToolStarted {
                                    name: tc.name.clone(),
                                },
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
            let mut deferred_auth: Option<String> = None;

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
                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                        && let Some(turn) = thread.last_turn_mut()
                    {
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

                // Auth detection — defer return until all results are recorded
                if deferred_auth.is_none()
                    && let Some((ext_name, instructions)) =
                        check_auth_required(&tc.name, &deferred_result)
                {
                    self.handle_auth_intercept(
                        &session,
                        thread_id,
                        message,
                        &deferred_result,
                        ext_name,
                        instructions.clone(),
                    )
                    .await;
                    deferred_auth = Some(instructions);
                }

                context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, deferred_content));
            }

            // Return auth response after all results are recorded
            if let Some(instructions) = deferred_auth {
                return Ok(SubmissionResult::response(instructions));
            }

            // Handle approval if a tool needed it
            if let Some((approval_idx, tc, tool, allow_always)) = approval_needed {
                let new_pending = PendingApproval {
                    request_id: Uuid::new_v4(),
                    tool_name: tc.name.clone(),
                    parameters: tc.arguments.clone(),
                    display_parameters: redact_params(&tc.arguments, tool.sensitive_params()),
                    description: tool.description().to_string(),
                    tool_call_id: tc.id.clone(),
                    context_messages: context_messages.clone(),
                    deferred_tool_calls: deferred_tool_calls[approval_idx + 1..].to_vec(),
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
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.await_approval(new_pending);
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
                Ok(AgenticLoopResult::Response(response)) => {
                    let (response, suggestions) =
                        crate::agent::dispatcher::extract_suggestions(&response);
                    thread.complete_turn(&response);
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
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Done".into()),
                            &message.metadata,
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
                    Ok(SubmissionResult::response(response))
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.display_parameters.clone();
                    let allow_always = new_pending.allow_always;
                    thread.await_approval(*new_pending);
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
                Err(e) => {
                    thread.fail_turn(e.to_string());
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
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.clear_pending_approval();
                    thread.complete_turn(&rejection);
                    // User message already persisted at turn start; save rejection response
                    self.persist_assistant_response(
                        thread_id,
                        &message.channel,
                        &message.user_id,
                        &rejection,
                    )
                    .await;
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
        tool_result: &Result<String, Error>,
        ext_name: String,
        instructions: String,
    ) {
        let auth_data = parse_auth_result(tool_result);
        {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.enter_auth_mode(ext_name.clone());
                thread.complete_turn(&instructions);
                // User message already persisted at turn start; save auth instructions
                self.persist_assistant_response(
                    thread_id,
                    &message.channel,
                    &message.user_id,
                    &instructions,
                )
                .await;
            }
        }
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::AuthRequired {
                    extension_name: ext_name,
                    instructions: Some(instructions.clone()),
                    auth_url: auth_data.auth_url,
                    setup_url: auth_data.setup_url,
                },
                &message.metadata,
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
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.pending_auth = None;
            }
        }

        let ext_mgr = match self.deps.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return Ok(Some("Extension manager not available.".to_string())),
        };

        match ext_mgr
            .configure_token(&pending.extension_name, token, &message.user_id)
            .await
        {
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
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.enter_auth_mode(pending.extension_name.clone());
                    }
                }
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: pending.extension_name.clone(),
                            instructions: Some(result.message.clone()),
                            auth_url: None,
                            setup_url: None,
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(result.message))
            }
            Err(e) => {
                let msg = e.to_string();
                // Token validation errors: re-enter auth mode and re-prompt
                if matches!(e, crate::extensions::ExtensionError::ValidationFailed(_)) {
                    {
                        let mut sess = session.lock().await;
                        if let Some(thread) = sess.threads.get_mut(&thread_id) {
                            thread.enter_auth_mode(pending.extension_name.clone());
                        }
                    }
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::AuthRequired {
                                extension_name: pending.extension_name.clone(),
                                instructions: Some(msg.clone()),
                                auth_url: None,
                                setup_url: None,
                            },
                            &message.metadata,
                        )
                        .await;
                    return Ok(Some(msg));
                }
                // Infrastructure errors
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
        let thread = sess.create_thread();
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
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;

        if sess.switch_thread(target_thread_id) {
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
                                thought_signature: None,
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
                                res.to_string()
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

    fn make_db_msg(role: &str, content: &str) -> crate::history::ConversationMessage {
        crate::history::ConversationMessage {
            id: uuid::Uuid::new_v4(),
            role: role.to_string(),
            content: content.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_awaiting_approval_rejection_includes_tool_context() {
        // Test that when a thread is in AwaitingApproval state and receives a new message,
        // process_user_input rejects it with a non-error status that includes tool context.
        use crate::agent::session::{PendingApproval, Session, Thread, ThreadState};
        use uuid::Uuid;

        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id);

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

    #[test]
    fn test_queue_cap_rejects_at_capacity() {
        use crate::agent::session::{MAX_PENDING_MESSAGES, Thread, ThreadState};
        use uuid::Uuid;

        let mut thread = Thread::new(Uuid::new_v4());
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

        let mut thread = Thread::new(Uuid::new_v4());
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
        let mut thread = Thread::with_id(thread_id, session_id);
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
        let mut thread = Thread::with_id(thread_id, session_id);
        thread.start_turn("working");
        assert_eq!(thread.state, ThreadState::Processing);

        // Simulate the turn completing between snapshot and re-lock
        thread.complete_turn("done");
        assert_eq!(thread.state, ThreadState::Idle);

        let mut session = Session::new("test-user");
        session.threads.insert(thread_id, thread);

        // Re-check under lock: state is Idle, so queue_message must NOT be called.
        let t = session.threads.get_mut(&thread_id).unwrap();
        assert_ne!(t.state, ThreadState::Processing);
        // Verify nothing was queued — the fall-through path doesn't touch the queue.
        assert!(t.pending_messages.is_empty());
    }

    // Helper function to extract the approval message without needing a full Agent instance
    fn extract_approval_message(
        session: &crate::agent::session::Session,
        thread_id: Uuid,
    ) -> Result<Option<String>, crate::error::Error> {
        let thread = session.threads.get(&thread_id).ok_or_else(|| {
            crate::error::Error::from(crate::error::JobError::NotFound { id: thread_id })
        })?;

        if thread.state == ThreadState::AwaitingApproval {
            let approval_context = thread.pending_approval.as_ref().map(|a| {
                let desc_preview =
                    crate::agent::agent_loop::truncate_for_preview(&a.description, 80);
                (a.tool_name.clone(), desc_preview)
            });

            let msg = match approval_context {
                Some((tool_name, desc_preview)) => format!(
                    "Waiting for approval: {tool_name} — {desc_preview}. Use /interrupt to cancel."
                ),
                None => "Waiting for approval. Use /interrupt to cancel.".to_string(),
            };
            Ok(Some(msg))
        } else {
            Ok(None)
        }
    }
}
