//! Autonomous Trace Commons turn-end capture for the Reborn runtime.
//!
//! Mirrors the v1 binary's turn-end capture (`src/agent/thread_ops.rs::
//! spawn_autonomous_trace_contribution`) and periodic queue flush
//! (`src/agent/agent_loop.rs::spawn_trace_queue_flush_worker`): every terminal
//! turn lifecycle event spawns a detached best-effort task that reads the
//! owner's standing contribution policy, captures the recent thread
//! transcript, redacts and scores it locally, and queues + flushes eligible
//! envelopes. Non-enrolled users pay one policy-file read per turn and
//! nothing else.
//!
//! Capture must never block or fail the turn lifecycle path: the sink is
//! subscribed best-effort and all work happens on a spawned task whose
//! errors are logged at `debug!` only (`info!`/`warn!` corrupt the REPL).
//!
//! Credit-notice delivery (v1 broadcasts via `ChannelManager`) is
//! intentionally not wired here yet: the composition layer has no outbound
//! notification surface. The notice outbox still accumulates on disk and is
//! delivered when the same scope runs under the v1 binary; a Reborn-native
//! delivery path is a follow-up.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_reborn_traces::ConversationMessage;
use ironclaw_reborn_traces::client::{
    TraceClientAutonomousCaptureOutcome, TraceClientAutonomousCaptureRequest, TraceClientHost,
    TraceClientScope,
};
use ironclaw_reborn_traces::contribution::{self as trace, read_trace_policy_for_scope};
use ironclaw_threads::{
    ContextWindow, LoadContextWindowRequest, MessageKind, MessageStatus, SessionThreadError,
    SessionThreadService, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
};
use ironclaw_turns::{TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Recent-transcript bound, mirroring v1 (last 24 messages, max 5 turns).
const CAPTURE_MESSAGE_LIMIT: usize = 24;
const CAPTURE_MAX_TURNS: usize = 5;
/// Immediate flush limit after queueing one envelope (v1 parity).
const CAPTURE_FLUSH_LIMIT: usize = 10;
/// Periodic queue-flush cadence and per-scope limit (v1 parity).
const TRACE_QUEUE_WORKER_INTERVAL: Duration = Duration::from_secs(300);
const TRACE_QUEUE_WORKER_FLUSH_LIMIT: usize = 25;

/// Scopes whose queues the periodic worker flushes. Seeded with the runtime
/// owner and extended with every scope seen at capture time. Queued items for
/// scopes not seen since boot only flush on that scope's next turn — the
/// composition layer has no user directory to enumerate (v1 lists active
/// users from its database).
pub(crate) type ObservedTraceScopes = Arc<Mutex<BTreeSet<String>>>;

/// Narrow history-read seam so tests don't have to fake the full
/// [`SessionThreadService`] surface.
#[async_trait]
pub(crate) trait TraceCaptureHistorySource: Send + Sync {
    async fn thread_history_messages(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError>;
}

struct SessionThreadHistorySource {
    thread_service: Arc<dyn SessionThreadService>,
}

#[async_trait]
impl TraceCaptureHistorySource for SessionThreadHistorySource {
    async fn thread_history_messages(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
        // Read the model-context (replay) view, NOT list_thread_history: the
        // history projection nulls `tool_result_provider_call` for product
        // display, which would strip every tool call from the captured trace
        // and force a text-only (low-value, sub-threshold) envelope.
        let window = self
            .thread_service
            .load_context_window(LoadContextWindowRequest {
                scope: request.scope,
                thread_id: request.thread_id,
                max_messages: CAPTURE_MESSAGE_LIMIT,
            })
            .await?;
        Ok(context_window_to_records(window))
    }
}

/// Map context-window messages back into the record shape the capture adapter
/// consumes, preserving `tool_result_provider_call`. Context-window messages
/// are already model-context-filtered and committed, so the synthesized
/// `status` is `Finalized`.
fn context_window_to_records(window: ContextWindow) -> Vec<ThreadMessageRecord> {
    let thread_id = window.thread_id;
    window
        .messages
        .into_iter()
        .map(|message| ThreadMessageRecord {
            message_id: message.message_id.unwrap_or_else(ThreadMessageId::new),
            thread_id: thread_id.clone(),
            sequence: message.sequence,
            kind: message.kind,
            status: MessageStatus::Finalized,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: message.tool_result_provider_call,
            content: Some(message.content),
            redaction_ref: None,
            // Reconstructed-for-redaction record from a context window; the
            // capture path carries no attachment refs of its own.
            attachments: Vec::new(),
        })
        .collect()
}

pub(crate) struct TraceCaptureTurnEventSink {
    history: Arc<dyn TraceCaptureHistorySource>,
    observed_scopes: ObservedTraceScopes,
}

impl TraceCaptureTurnEventSink {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        observed_scopes: ObservedTraceScopes,
    ) -> Self {
        Self {
            history: Arc::new(SessionThreadHistorySource { thread_service }),
            observed_scopes,
        }
    }

    #[cfg(test)]
    fn with_history_source(
        history: Arc<dyn TraceCaptureHistorySource>,
        observed_scopes: ObservedTraceScopes,
    ) -> Self {
        Self {
            history,
            observed_scopes,
        }
    }
}

#[async_trait]
impl TurnEventSink for TraceCaptureTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if !matches!(event.kind, TurnEventKind::Completed | TurnEventKind::Failed) {
            return Ok(());
        }
        // Never capture without an explicit owner: system/sentinel-scoped
        // turns have no contribution policy and no consent.
        let Some(owner_user_id) = event
            .owner_user_id
            .clone()
            .or_else(|| event.scope.explicit_owner_user_id().cloned())
        else {
            return Ok(());
        };
        // Tenant-scope the persisted trace state key so the same user id in two
        // tenants does not share policy / device-key / credit / profile state.
        let scope = trace::trace_scope_key(event.scope.tenant_id.as_str(), owner_user_id.as_str());
        record_observed_scope(&self.observed_scopes, &scope);
        let history = Arc::clone(&self.history);
        tokio::spawn(async move {
            capture_turn_trace(history, event, scope).await;
        });
        Ok(())
    }
}

fn record_observed_scope(observed_scopes: &ObservedTraceScopes, scope: &str) {
    let mut scopes = match observed_scopes.lock() {
        Ok(scopes) => scopes,
        Err(poisoned) => poisoned.into_inner(),
    };
    scopes.insert(scope.to_string());
}

/// One turn's best-effort capture. Errors never propagate — every exit is a
/// `debug!` line keyed by the pseudonymous contributor ref, never raw content.
pub(crate) async fn capture_turn_trace(
    history: Arc<dyn TraceCaptureHistorySource>,
    event: TurnLifecycleEvent,
    scope: String,
) {
    let scope_ref = trace::local_pseudonymous_contributor_id(&scope);
    let policy = match read_trace_policy_for_scope(Some(scope.as_str())) {
        Ok(policy) => policy,
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture could not read policy");
            return;
        }
    };
    if !policy.enabled {
        return;
    }

    let Some(messages) = load_capture_messages(&history, &event, &scope_ref).await else {
        return;
    };
    if messages.is_empty() {
        return;
    }

    let turn_failed = matches!(event.kind, TurnEventKind::Failed);
    let outcome = TraceClientHost
        .prepare_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
            scope: TraceClientScope::user(scope.clone()),
            // The lifecycle event does not identify the product surface
            // (REPL/WebUI/channel) behind the turn, so the channel is the
            // honest catch-all rather than a guess.
            channel: trace::TraceChannel::Other,
            messages: &messages,
            policy: &policy,
            max_turns: CAPTURE_MAX_TURNS,
            // Reborn thread transcripts carry no structured outcome payload;
            // the lifecycle event's terminal status is authoritative.
            outcome_override: turn_failed.then_some(trace::TaskSuccess::Failure),
        })
        .await;
    match outcome {
        Ok(TraceClientAutonomousCaptureOutcome::Submit(envelope)) => {
            let trace_scope = TraceClientScope::user(scope.clone());
            if let Err(error) = TraceClientHost.queue_envelope_for_scope(&trace_scope, &envelope) {
                tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to queue envelope");
                return;
            }
            if let Err(error) = TraceClientHost
                .flush_scope_queue(&trace_scope, CAPTURE_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, %scope_ref, "Reborn trace queue flush failed; worker retries");
            }
        }
        Ok(TraceClientAutonomousCaptureOutcome::Held {
            kind,
            reason,
            envelope,
        }) => {
            let submission_id = envelope.submission_id;
            // Only manual-review holds (e.g. High residual-PII-risk) are
            // retained for the user to authorize. Policy/value gates (low
            // score, disallowed tools) are not review-worthy and are dropped
            // as before — just logged for diagnostics.
            if !matches!(kind, trace::TraceQueueHoldKind::ManualReview) {
                tracing::debug!(
                    %submission_id,
                    %reason,
                    %scope_ref,
                    "Reborn trace capture held by policy gate (dropped)"
                );
                return;
            }
            // Retain: queue with a ManualReview hold sidecar so the flush
            // worker skips it until it is authorized.
            let trace_scope = TraceClientScope::user(scope.clone());
            if let Err(error) =
                TraceClientHost.queue_held_envelope_for_scope(&trace_scope, &envelope, &reason)
            {
                tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to retain held envelope");
                return;
            }
            tracing::debug!(
                %submission_id,
                %reason,
                %scope_ref,
                "Reborn trace capture held for manual review (retained)"
            );
        }
        Ok(TraceClientAutonomousCaptureOutcome::Skipped) => {}
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to build envelope");
        }
    }
}

async fn load_capture_messages(
    history: &Arc<dyn TraceCaptureHistorySource>,
    event: &TurnLifecycleEvent,
    scope_ref: &str,
) -> Option<Vec<ConversationMessage>> {
    let Some(agent_id) = event.scope.agent_id.clone() else {
        tracing::debug!(%scope_ref, "Reborn trace capture skipped: turn scope has no agent id");
        return None;
    };
    let owner_user_id = event
        .owner_user_id
        .clone()
        .or_else(|| event.scope.explicit_owner_user_id().cloned());
    let request = ThreadHistoryRequest {
        scope: ThreadScope {
            tenant_id: event.scope.tenant_id.clone(),
            agent_id,
            project_id: event.scope.project_id.clone(),
            owner_user_id,
            mission_id: None,
        },
        thread_id: event.scope.thread_id.clone(),
    };
    match history.thread_history_messages(request).await {
        Ok(records) => Some(conversation_messages_from_records(&records)),
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture could not load thread history");
            None
        }
    }
}

/// Adapt Reborn thread transcript records into the neutral conversation
/// shape the trace capture pipeline consumes.
///
/// User/assistant text rows become `user`/`assistant` messages. Tool-result
/// rows that carry `tool_result_provider_call` replay metadata are
/// reconstructed into a single `tool_calls` message per run of consecutive
/// rows, positioned between the user message and the assistant response — the
/// shape `capture_turns_from_conversation_messages` expects (its per-turn
/// lookahead consumes exactly one `tool_calls` message, so consecutive rows
/// must collapse into one). Tool-result rows without provider metadata carry
/// nothing reconstructable and stay dropped. Raw tool payloads inside the
/// `tool_calls` message are consent-gated downstream by the envelope builder
/// (`include_tool_payloads`); tool names always flow through so the value
/// scorecard sees `required_tools`/replayability. Redacted and superseded
/// rows never leave the thread store.
fn conversation_messages_from_records(records: &[ThreadMessageRecord]) -> Vec<ConversationMessage> {
    let now = Utc::now();
    let mut messages: Vec<ConversationMessage> = Vec::new();
    let mut pending_tool_calls: Vec<serde_json::Value> = Vec::new();

    for record in records {
        if !matches!(
            record.status,
            MessageStatus::Accepted
                | MessageStatus::Submitted
                | MessageStatus::Finalized
                | MessageStatus::Interrupted
        ) {
            continue;
        }
        match record.kind {
            MessageKind::User | MessageKind::Assistant => {
                flush_tool_calls(&mut pending_tool_calls, &mut messages, now);
                let role = if matches!(record.kind, MessageKind::User) {
                    "user"
                } else {
                    "assistant"
                };
                let Some(content) = record.content.clone() else {
                    continue;
                };
                if content.trim().is_empty() {
                    continue;
                }
                messages.push(ConversationMessage {
                    id: uuid::Uuid::new_v4(),
                    role: role.to_string(),
                    content,
                    // Thread message records carry no timestamps; capture time
                    // is informational only (turn started_at metadata).
                    created_at: now,
                });
            }
            MessageKind::ToolResultReference => {
                if let Some(call) = record.tool_result_provider_call.as_ref() {
                    pending_tool_calls
                        .push(tool_call_capture_json(call, record.content.as_deref()));
                }
            }
            MessageKind::System
            | MessageKind::Summary
            | MessageKind::CheckpointReference
            | MessageKind::CapabilityDisplayPreview => {}
        }
    }
    flush_tool_calls(&mut pending_tool_calls, &mut messages, now);

    if messages.len() > CAPTURE_MESSAGE_LIMIT {
        messages = messages.split_off(messages.len() - CAPTURE_MESSAGE_LIMIT);
    }
    messages
}

/// Emit accumulated tool-call entries as one `tool_calls` message, if any.
fn flush_tool_calls(
    pending: &mut Vec<serde_json::Value>,
    messages: &mut Vec<ConversationMessage>,
    now: chrono::DateTime<Utc>,
) {
    if pending.is_empty() {
        return;
    }
    let content = serde_json::Value::Array(std::mem::take(pending)).to_string();
    messages.push(ConversationMessage {
        id: uuid::Uuid::new_v4(),
        role: "tool_calls".to_string(),
        content,
        created_at: now,
    });
}

/// Build one `parse_capture_tool_calls`-shaped entry from a provider tool-call
/// reference and its (optional) result content. The result text is carried as
/// `result_preview`; the envelope builder redacts it unless the contribution
/// policy consents to tool payloads.
fn tool_call_capture_json(
    call: &ironclaw_threads::ProviderToolCallReferenceEnvelope,
    result: Option<&str>,
) -> serde_json::Value {
    let mut entry = serde_json::Map::new();
    entry.insert(
        "name".to_string(),
        serde_json::Value::String(call.provider_tool_name.clone()),
    );
    if let Some(result) = result.filter(|content| !content.trim().is_empty()) {
        entry.insert(
            "result_preview".to_string(),
            serde_json::Value::String(result.to_string()),
        );
    }
    if let Some(rationale) = call.reasoning.as_ref().filter(|r| !r.trim().is_empty()) {
        entry.insert(
            "rationale".to_string(),
            serde_json::Value::String(rationale.clone()),
        );
    }
    serde_json::Value::Object(entry)
}

pub(crate) struct TraceQueueFlushWorkerHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl TraceQueueFlushWorkerHandle {
    pub(crate) async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(error) = self.handle.await {
            tracing::debug!(%error, "Reborn trace queue flush worker did not shut down cleanly");
        }
    }
}

/// Periodic queue flush, mirroring v1's 300s worker: retries envelopes whose
/// immediate flush failed (network blips, endpoint downtime) for every scope
/// observed since boot.
pub(crate) fn spawn_trace_queue_flush_worker(
    observed_scopes: ObservedTraceScopes,
) -> TraceQueueFlushWorkerHandle {
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TRACE_QUEUE_WORKER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick fires immediately; consume it so the first flush
        // happens one full interval after boot.
        interval.tick().await;
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = interval.tick() => {}
            }
            let scopes: Vec<String> = {
                let scopes = match observed_scopes.lock() {
                    Ok(scopes) => scopes,
                    Err(poisoned) => poisoned.into_inner(),
                };
                scopes.iter().cloned().collect()
            };
            if scopes.is_empty() {
                continue;
            }
            if let Err(error) = TraceClientHost
                .flush_queue_worker_tick(scopes.clone(), TRACE_QUEUE_WORKER_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, "Reborn trace queue worker tick failed");
            }
            // Prune drained scopes so the observed set stays bounded by actual
            // pending backlog, not by every caller ever seen on this runtime. A
            // scope with no flushable queue entries is dropped; its next turn
            // re-adds it via `record_observed_scope`. Scopes that still hold
            // pending work (e.g. a flush that hit the per-tick limit, or an
            // endpoint that's down) are retained so the next tick retries them.
            {
                let mut observed = match observed_scopes.lock() {
                    Ok(observed) => observed,
                    Err(poisoned) => poisoned.into_inner(),
                };
                observed.retain(|scope| trace::trace_scope_has_pending_queue(scope.as_str()));
            }
        }
    });
    TraceQueueFlushWorkerHandle { cancel, handle }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{CapabilityId, UserId};
    use ironclaw_threads::{ProviderToolCallReferenceEnvelope, ThreadMessageId};
    use ironclaw_turns::{EventCursor, TurnRunId, TurnScope, TurnStatus};
    use uuid::Uuid;

    use super::*;

    struct FixedHistorySource {
        records: Vec<ThreadMessageRecord>,
    }

    #[async_trait]
    impl TraceCaptureHistorySource for FixedHistorySource {
        async fn thread_history_messages(
            &self,
            _request: ThreadHistoryRequest,
        ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
            Ok(self.records.clone())
        }
    }

    struct FailingHistorySource;

    #[async_trait]
    impl TraceCaptureHistorySource for FailingHistorySource {
        async fn thread_history_messages(
            &self,
            _request: ThreadHistoryRequest,
        ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
            Err(SessionThreadError::UnknownThread {
                thread_id: test_thread_id(),
            })
        }
    }

    fn record(kind: MessageKind, status: MessageStatus, content: &str) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: test_thread_id(),
            sequence: 0,
            kind,
            status,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content.to_string()),
            redaction_ref: None,
            attachments: Vec::new(),
        }
    }

    fn test_thread_id() -> ironclaw_host_api::ThreadId {
        ironclaw_host_api::ThreadId::new("trace-capture-test-thread").expect("thread id")
    }

    fn terminal_event(kind: TurnEventKind, owner: Option<&str>) -> TurnLifecycleEvent {
        let owner_user_id =
            owner.map(|owner| UserId::new(owner).expect("test owner user id is valid"));
        TurnLifecycleEvent {
            cursor: EventCursor::default(),
            scope: TurnScope::new_with_owner(
                ironclaw_host_api::TenantId::new("trace-capture-test-tenant").expect("tenant"),
                Some(ironclaw_host_api::AgentId::new("trace-capture-test-agent").expect("agent")),
                None,
                test_thread_id(),
                owner_user_id.clone(),
            ),
            occurred_at: None,
            owner_user_id,
            run_id: TurnRunId::new(),
            status: match kind {
                TurnEventKind::Failed => TurnStatus::Failed,
                _ => TurnStatus::Completed,
            },
            kind,
            blocked_gate: None,
            sanitized_reason: None,
        }
    }

    fn enabled_policy() -> trace::StandingTraceContributionPolicy {
        trace::StandingTraceContributionPolicy {
            enabled: true,
            // Loopback endpoint on a closed port: the immediate flush attempt
            // fails fast and locally (no external traffic), leaving the
            // envelope queued for assertion.
            ingestion_endpoint: Some("https://127.0.0.1:1/v1/traces".to_string()),
            min_submission_score: 0.0,
            require_manual_approval_when_pii_detected: false,
            auto_submit_high_value_traces: true,
            ..trace::StandingTraceContributionPolicy::default()
        }
    }

    fn unique_scope(label: &str) -> String {
        format!("reborn-trace-capture-{label}-{}", Uuid::new_v4())
    }

    fn queue_dir(scope: &str) -> std::path::PathBuf {
        trace::trace_contribution_dir_for_scope(Some(scope)).join("queue")
    }

    fn queued_entries(scope: &str) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(queue_dir(scope))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|path| {
                        // Envelope entries only — exclude `.held.json` hold
                        // sidecars the flush path may write next to them.
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.ends_with(".json") && !name.ends_with(".held.json")
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn cleanup_scope(scope: &str) {
        let dir = trace::trace_contribution_dir_for_scope(Some(scope));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conversation_messages_keep_user_and_assistant_text_only() {
        let records = vec![
            record(MessageKind::User, MessageStatus::Accepted, "hello"),
            record(MessageKind::System, MessageStatus::Finalized, "system row"),
            record(
                MessageKind::ToolResultReference,
                MessageStatus::Finalized,
                "ref-only",
            ),
            record(MessageKind::Assistant, MessageStatus::Finalized, "hi"),
            record(MessageKind::Assistant, MessageStatus::Redacted, "redacted"),
            record(MessageKind::Assistant, MessageStatus::Superseded, "stale"),
        ];
        let messages = conversation_messages_from_records(&records);
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "hi");
    }

    fn provider_call_reference(tool_name: &str) -> ProviderToolCallReferenceEnvelope {
        ProviderToolCallReferenceEnvelope {
            provider_id: "openai".to_string(),
            provider_model_id: "gpt".to_string(),
            provider_turn_id: "turn-1".to_string(),
            provider_call_id: "call-1".to_string(),
            provider_tool_name: tool_name.to_string(),
            capability_id: CapabilityId::new(format!("builtin.{tool_name}"))
                .expect("capability id"),
            arguments: serde_json::json!({ "url": "https://example.com" }),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }

    fn tool_result_record(tool_name: &str, result: &str) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: test_thread_id(),
            sequence: 0,
            kind: MessageKind::ToolResultReference,
            status: MessageStatus::Finalized,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: Some("ref-1".to_string()),
            tool_result_provider_call: Some(provider_call_reference(tool_name)),
            content: Some(result.to_string()),
            redaction_ref: None,
            attachments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn capture_history_source_preserves_tool_call_provider_metadata() {
        use ironclaw_threads::{
            AppendToolResultReferenceRequest, EnsureThreadRequest, InMemorySessionThreadService,
            ToolResultSafeSummary,
        };

        // The capture history source must read through a metadata-preserving
        // path (load_context_window), NOT list_thread_history — the latter
        // nulls tool_result_provider_call for product display, which starves
        // the capture adapter of every tool call and forces a text-only trace.
        let service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let scope = ThreadScope {
            tenant_id: ironclaw_host_api::TenantId::new("trace-cap-src-tenant").expect("tenant"),
            agent_id: ironclaw_host_api::AgentId::new("trace-cap-src-agent").expect("agent"),
            project_id: None,
            owner_user_id: Some(UserId::new("trace-cap-src-user").expect("user")),
            mission_id: None,
        };
        let thread = service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(
                    ironclaw_host_api::ThreadId::new("trace-cap-src-thread").expect("thread"),
                ),
                created_by_actor_id: "actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
        service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: scope.clone(),
                thread_id: thread.thread_id.clone(),
                turn_run_id: "run-1".into(),
                result_ref: "result:demo".into(),
                safe_summary: ToolResultSafeSummary::new("safe tool result").expect("summary"),
                provider_call: Some(provider_call_reference("web_fetch")),
                model_observation: None,
            })
            .await
            .expect("append tool result");

        let source = SessionThreadHistorySource {
            thread_service: Arc::clone(&service),
        };
        let records = source
            .thread_history_messages(ThreadHistoryRequest {
                scope,
                thread_id: thread.thread_id,
            })
            .await
            .expect("history read");

        let tool_row = records
            .iter()
            .find(|r| matches!(r.kind, MessageKind::ToolResultReference))
            .expect("tool-result row must be present in capture history");
        assert!(
            tool_row.tool_result_provider_call.is_some(),
            "capture history must preserve tool_result_provider_call"
        );
    }

    #[test]
    fn tool_result_reference_with_provider_call_becomes_tool_calls_message() {
        // The Reborn capture adapter must reconstruct tool-call turns from the
        // `tool_result_provider_call` replay metadata so the downstream value
        // scorecard sees `required_tools`/replayability, not just text. The
        // envelope builder expects a `role:"tool_calls"` message sitting
        // between the user message and the assistant response.
        let records = vec![
            record(MessageKind::User, MessageStatus::Accepted, "fetch the page"),
            tool_result_record("web_fetch", "200 OK body"),
            record(
                MessageKind::Assistant,
                MessageStatus::Finalized,
                "here it is",
            ),
        ];
        let messages = conversation_messages_from_records(&records);

        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "tool_calls", "assistant"]);

        let calls: serde_json::Value =
            serde_json::from_str(&messages[1].content).expect("tool_calls content is JSON");
        let call = &calls.as_array().expect("tool_calls is a JSON array")[0];
        assert_eq!(call["name"], "web_fetch");
        assert_eq!(call["result_preview"], "200 OK body");
    }

    #[test]
    fn consecutive_tool_result_references_collapse_into_one_tool_calls_message() {
        // The builder's per-turn lookahead consumes exactly one `tool_calls`
        // message between user and assistant, so consecutive tool-result rows
        // must collapse into a single message carrying every call, or only the
        // first would be scored.
        let records = vec![
            record(MessageKind::User, MessageStatus::Accepted, "research it"),
            tool_result_record("web_search", "hits"),
            tool_result_record("web_fetch", "page body"),
            record(MessageKind::Assistant, MessageStatus::Finalized, "summary"),
        ];
        let messages = conversation_messages_from_records(&records);

        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "tool_calls", "assistant"]);

        let calls: serde_json::Value =
            serde_json::from_str(&messages[1].content).expect("tool_calls content is JSON");
        let names: Vec<&str> = calls
            .as_array()
            .expect("array")
            .iter()
            .map(|c| c["name"].as_str().expect("name"))
            .collect();
        assert_eq!(names, vec!["web_search", "web_fetch"]);
    }

    #[test]
    fn tool_result_reference_without_provider_call_is_still_dropped() {
        // A ref-only tool result with no provider replay metadata carries no
        // reconstructable tool call, so it stays dropped (no empty tool_calls
        // message that would mislead the scorer).
        let records = vec![
            record(MessageKind::User, MessageStatus::Accepted, "hi"),
            record(
                MessageKind::ToolResultReference,
                MessageStatus::Finalized,
                "ref-only",
            ),
            record(MessageKind::Assistant, MessageStatus::Finalized, "hello"),
        ];
        let messages = conversation_messages_from_records(&records);
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    #[test]
    fn conversation_messages_bound_to_recent_window() {
        let records: Vec<ThreadMessageRecord> = (0..(CAPTURE_MESSAGE_LIMIT + 10))
            .map(|i| {
                record(
                    MessageKind::User,
                    MessageStatus::Accepted,
                    &format!("message {i}"),
                )
            })
            .collect();
        let messages = conversation_messages_from_records(&records);
        assert_eq!(messages.len(), CAPTURE_MESSAGE_LIMIT);
        assert_eq!(messages[0].content, "message 10");
    }

    #[tokio::test]
    async fn capture_queues_envelope_for_enrolled_scope() {
        let scope = unique_scope("enrolled");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "do the thing"),
                record(MessageKind::Assistant, MessageStatus::Finalized, "done"),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;

        // No ingestion endpoint is configured, so the immediate flush fails
        // locally and the envelope must remain queued for the worker.
        let entries = queued_entries(&scope);
        assert_eq!(entries.len(), 1, "exactly one envelope queued");
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
        assert_eq!(envelope["outcome"]["task_success"], "success");
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn captured_tool_using_turn_carries_tools_into_envelope_replay() {
        // Integration guard over adapter + envelope builder: a tool-using turn
        // must surface the tool in the queued envelope's replay metadata
        // (required_tools + replayable). These are the dormant value-score
        // levers that text-only capture never lit, so this is what lets an
        // agentic turn clear the submission-score gate in production.
        let scope = unique_scope("tool-using");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "fetch the page"),
                tool_result_record("web_fetch", "200 OK body"),
                record(
                    MessageKind::Assistant,
                    MessageStatus::Finalized,
                    "here it is",
                ),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;

        let entries = queued_entries(&scope);
        assert_eq!(entries.len(), 1, "exactly one envelope queued");
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");

        let required_tools = envelope["replay"]["required_tools"]
            .as_array()
            .expect("required_tools is an array");
        assert!(
            required_tools.iter().any(|t| t == "web_fetch"),
            "tool name must reach replay.required_tools, got {required_tools:?}"
        );
        assert_eq!(
            envelope["replay"]["replayable"], true,
            "a tool-using turn must be marked replayable"
        );
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_retains_manual_review_hold_for_high_pii_trace() {
        // A High residual-PII-risk trace (blocked secret detected) under a
        // manual-approval policy must be RETAINED as a ManualReview hold for
        // the user to authorize, not dropped.
        let scope = unique_scope("manual-review");
        let mut policy = enabled_policy();
        policy.require_manual_approval_when_pii_detected = true;
        policy.include_message_text = true; // so the secret is scanned
        trace::write_trace_policy_for_scope(Some(&scope), &policy).expect("write policy");

        // The AWS access key triggers a Critical leak detection -> blocked
        // secret -> High residual PII risk -> ManualReview hold.
        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(
                    MessageKind::User,
                    MessageStatus::Accepted,
                    "deploy using AKIAIOSFODNN7EXAMPLE then report back",
                ),
                record(MessageKind::Assistant, MessageStatus::Finalized, "deployed"),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;

        let holds = trace::read_trace_queue_holds_for_scope(Some(&scope)).expect("read holds");
        assert_eq!(
            holds.len(),
            1,
            "high-PII trace retained as exactly one hold"
        );
        assert_eq!(holds[0].kind, trace::TraceQueueHoldKind::ManualReview);
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_drops_policy_gated_hold_without_retaining() {
        // A low-value (sub-threshold score) hold is a PolicyGate, not
        // review-worthy: it must NOT be retained, so the held-review surface
        // stays free of low-value traces.
        let scope = unique_scope("policy-gated");
        let mut policy = enabled_policy();
        policy.min_submission_score = 1.0; // any real trace scores below this
        trace::write_trace_policy_for_scope(Some(&scope), &policy).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "hi there"),
                record(MessageKind::Assistant, MessageStatus::Finalized, "hello"),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;

        let holds = trace::read_trace_queue_holds_for_scope(Some(&scope)).expect("read holds");
        assert!(holds.is_empty(), "policy-gated hold must not be retained");
        assert!(
            queued_entries(&scope).is_empty(),
            "policy-gated trace must not be queued"
        );
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_marks_failed_turns_as_failure_outcome() {
        let scope = unique_scope("failed-turn");
        // auto_submit_failed_traces is on by default in enabled_policy()'s
        // base, so the failed turn is still eligible.
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "do the thing"),
                record(
                    MessageKind::Assistant,
                    MessageStatus::Finalized,
                    "attempt output",
                ),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Failed, Some(&scope)),
            scope.clone(),
        )
        .await;

        let entries = queued_entries(&scope);
        assert_eq!(entries.len(), 1, "failed turn envelope queued");
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
        assert_eq!(envelope["outcome"]["task_success"], "failure");
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_skips_when_policy_missing_or_disabled() {
        let scope = unique_scope("not-enrolled");
        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![record(MessageKind::User, MessageStatus::Accepted, "hello")],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;
        assert!(
            !queue_dir(&scope).exists(),
            "no queue dir for non-enrolled scope"
        );
    }

    #[tokio::test]
    async fn capture_survives_history_read_failure() {
        let scope = unique_scope("history-error");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");
        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FailingHistorySource);
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;
        assert!(
            !queue_dir(&scope).exists(),
            "history failure queues nothing"
        );
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn sink_ignores_non_terminal_and_ownerless_events() {
        let scopes: ObservedTraceScopes = Arc::new(Mutex::new(BTreeSet::new()));
        let sink = TraceCaptureTurnEventSink::with_history_source(
            Arc::new(FixedHistorySource {
                records: Vec::new(),
            }),
            Arc::clone(&scopes),
        );
        sink.publish(terminal_event(TurnEventKind::Submitted, Some("someone")))
            .await
            .expect("non-terminal event accepted");
        sink.publish(terminal_event(TurnEventKind::Completed, None))
            .await
            .expect("ownerless event accepted");
        assert!(
            scopes.lock().expect("scope set lock").is_empty(),
            "neither event records a capture scope"
        );
    }

    #[tokio::test]
    async fn sink_records_scope_and_spawns_capture_for_terminal_events() {
        // The owner here is a non-runtime-owner caller (this sink serves many
        // WebUI users). Capture must attribute the trace to the EVENT's
        // tenant+owner composite — `trace_scope_key(tenant, owner)` — NOT the
        // bare owner id and NOT any runtime-wide owner. Enroll, observe, and
        // assert the queue all under that composite key.
        let owner = unique_scope("sink-spawn-owner");
        let capture_key = trace::trace_scope_key("trace-capture-test-tenant", &owner);
        trace::write_trace_policy_for_scope(Some(&capture_key), &enabled_policy())
            .expect("write policy");
        let scopes: ObservedTraceScopes = Arc::new(Mutex::new(BTreeSet::new()));
        let sink = TraceCaptureTurnEventSink::with_history_source(
            Arc::new(FixedHistorySource {
                records: vec![
                    record(MessageKind::User, MessageStatus::Accepted, "hello"),
                    record(MessageKind::Assistant, MessageStatus::Finalized, "hi"),
                ],
            }),
            Arc::clone(&scopes),
        );
        sink.publish(terminal_event(TurnEventKind::Completed, Some(&owner)))
            .await
            .expect("terminal event accepted");
        {
            let observed = scopes.lock().expect("scope set lock");
            assert!(
                observed.contains(&capture_key),
                "terminal event records the tenant-scoped composite key for the flush worker"
            );
            assert!(
                !observed.contains(&owner),
                "the bare owner id must NOT be used as the trace scope key"
            );
        }
        // The capture task is detached; poll briefly for the queued envelope.
        for _ in 0..100 {
            if !queued_entries(&capture_key).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            queued_entries(&capture_key).len(),
            1,
            "spawned capture queues the envelope under the tenant-scoped key"
        );
        // Nothing was written under the bare owner id.
        assert!(
            queued_entries(&owner).is_empty(),
            "no trace state may be written under the un-tenant-scoped owner id"
        );
        cleanup_scope(&capture_key);
        cleanup_scope(&owner);
    }
}
