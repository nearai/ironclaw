//! After-turn interaction recording for the Reborn planned loop (mem0 `add`).
//!
//! At the run-end seam (a `Completed` run), the host hands the just-finished run's
//! FULL ordered transcript (every user / assistant / tool message of the turn) to
//! the memory provider's [`MemoryService::record_interaction`], mirroring
//! `mem0.add(messages=[...], metadata)`. The host passes the interaction DATA and
//! lets the provider decide what to record (verbatim, LLM extraction, or nothing)
//! — it makes NO verbatim-vs-extract / provenance / TTL decision here.
//!
//! mem0's session/`run_id` maps to our `scope.thread_id` (the conversation), which
//! the provider derives from the invocation scope; the request's `turn_run_id` is
//! per-turn PROVENANCE only (it names the native per-run transcript file), NOT the
//! mem0 session id.
//!
//! This is a post-terminal, best-effort side effect: the run is ALREADY
//! `Completed` when the recorder runs, so any failure (history read, missing
//! content, provider write) is logged at `debug!` and swallowed. It must never
//! fail the run and must never emit `info!`/`warn!` (this is a background path
//! that would corrupt the REPL/TUI).

use std::sync::Arc;

use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{
    MemoryInteractionMessage, MemoryInteractionRole, MemoryInvocation, MemoryService,
    MemoryServiceRecordRequest,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadMessageRecord,
    ThreadScope,
};
use ironclaw_turns::{TurnActor, TurnRunState};
use tracing::debug;

use crate::thread_scope::ThreadScopeResolver;

/// Records a completed run's full ordered transcript into memory.
///
/// Held as a single dependency (no Arc sprawl): one recorder owns the thread
/// read port, the memory write port, and the base thread scope it owner-rewrites
/// from.
pub struct AfterTurnMemoryRecorder {
    thread_service: Arc<dyn SessionThreadService>,
    memory_writer: Arc<dyn MemoryService>,
    base_thread_scope: ThreadScope,
}

impl AfterTurnMemoryRecorder {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        memory_writer: Arc<dyn MemoryService>,
        base_thread_scope: ThreadScope,
    ) -> Self {
        Self {
            thread_service,
            memory_writer,
            base_thread_scope,
        }
    }

    /// Best-effort record of a `Completed` run's exchange. Never fails the run:
    /// every error path logs at `debug!` and returns.
    pub async fn record_completed_run(&self, state: &TurnRunState) {
        // Without a run actor there is no user identity to scope the memory write
        // to, so there is nothing to record. Degrade silently.
        let Some(actor) = state.actor.as_ref() else {
            debug!("after-turn memory: run has no actor; skipping interaction record");
            return;
        };

        // CRITICAL: read the thread under the SAME owner-rewritten scope the loop
        // host wrote it with. Reading with the raw base scope would hit the wrong
        // `owners/<caller>` subtree and find nothing — the exact hazard the
        // completion-evidence read guards against in `loop_exit_applier`.
        let scope = ThreadScopeResolver::resolve_for_turn(
            &self.base_thread_scope,
            &state.scope,
            state.actor.as_ref(),
        );
        let history = match self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope,
                thread_id: state.scope.thread_id.clone(),
            })
            .await
        {
            Ok(history) => history,
            Err(error) => {
                // silent-ok: after-turn memory is post-terminal; a thread-history
                // read failure must not reopen or fail the already-completed run.
                debug!(error = %error, "after-turn memory: thread history read failed; skipping");
                return;
            }
        };

        let run_id = state.run_id.to_string();
        let actor_user_id = actor.user_id.as_str();
        let agent_id = state.scope.agent_id.as_ref().map(|id| id.as_str());
        let messages = build_transcript(&history.messages, &run_id, actor_user_id, agent_id);
        // Skip only when the transcript carries no user/assistant content — a
        // turn with nothing meaningful to remember (matches "native stores the
        // full turn history"). Tool-only fragments alone are not recorded.
        let has_conversational_message = messages.iter().any(|message| {
            matches!(
                message.role,
                MemoryInteractionRole::User | MemoryInteractionRole::Assistant
            )
        });
        if !has_conversational_message {
            debug!("after-turn memory: no user/assistant content for run; skipping");
            return;
        }

        // Pass the raw interaction DATA; the provider decides what to do with it.
        // `user_id`/`agent_id`/`thread_id` ride the invocation scope. `turn_run_id`
        // + `correlation_id` ride `metadata` as opaque provenance (the provider
        // self-generates timestamps; the host does not add them).
        let correlation_id = CorrelationId::new();
        let invocation = invocation_for_run(state, actor, correlation_id);
        let request = MemoryServiceRecordRequest {
            messages,
            turn_run_id: Some(run_id.clone()),
            metadata: serde_json::json!({
                "turn_run_id": run_id,
                "correlation_id": correlation_id.to_string(),
            }),
        };
        if let Err(error) = self
            .memory_writer
            .record_interaction(invocation, request)
            .await
        {
            // silent-ok: after-turn memory writes are best-effort after completion;
            // a provider failure must not fail an already-completed run.
            debug!(error = %error, "after-turn memory: record_interaction failed; run already complete");
        }
    }
}

/// Build the full ordered turn transcript for `run_id` from the thread messages:
/// every message tagged with this `turn_run_id`, in sequence order, mapped to its
/// `User` / `Assistant` / `Tool` role (other kinds — `System`, summaries,
/// checkpoint/preview references — are skipped). Per-message `name` carries the
/// actor label (mem0 message `name`): the user's `user_id`, the `agent_id` for an
/// assistant, `None` for a tool message.
///
/// Captures EVERY finalized assistant message (all steps + the final answer), not
/// just the first — the audit-H1 fix over the prior `.find()` of a single
/// assistant. Returns the messages in order; the caller decides whether the
/// transcript is worth recording.
fn build_transcript(
    messages: &[ThreadMessageRecord],
    run_id: &str,
    actor_user_id: &str,
    agent_id: Option<&str>,
) -> Vec<MemoryInteractionMessage> {
    let mut records: Vec<&ThreadMessageRecord> = messages
        .iter()
        .filter(|message| message.turn_run_id.as_deref() == Some(run_id))
        .collect();
    // Sequence order is the stable transcript order, independent of the read
    // backend's iteration order.
    records.sort_by_key(|message| message.sequence);

    records
        .into_iter()
        .filter_map(|message| {
            // Map kind -> interaction role; skip System (and any other non-turn
            // kind such as summaries or checkpoint/preview references).
            let (role, name) = match message.kind {
                MessageKind::User => (MemoryInteractionRole::User, Some(actor_user_id.to_string())),
                // Every finalized assistant step, including the FINAL answer. A
                // non-finalized draft is superseded by its finalized row, so only
                // finalized assistants enter the transcript.
                MessageKind::Assistant if message.status == MessageStatus::Finalized => (
                    MemoryInteractionRole::Assistant,
                    agent_id.map(str::to_string),
                ),
                MessageKind::ToolResultReference => (MemoryInteractionRole::Tool, None),
                _ => return None,
            };
            // Skip messages with no usable content (e.g. a redacted or blank-only
            // row), but pass the ORIGINAL content through unchanged — "LLM data is
            // never deleted": the provider decides verbatim-vs-extract, so the host
            // must not trim the transcript before recording it.
            let content = message
                .content
                .as_deref()
                .filter(|content| !content.trim().is_empty())?;
            Some(MemoryInteractionMessage {
                role,
                content: content.to_string(),
                name,
            })
        })
        .collect()
}

/// Build the memory invocation for a run: the thread is kept (short-term lane),
/// `user_id` is the run's actor. Mirrors `invocation_for_context_request` in
/// `ironclaw_host_runtime::memory_context`.
fn invocation_for_run(
    state: &TurnRunState,
    actor: &TurnActor,
    correlation_id: CorrelationId,
) -> MemoryInvocation {
    MemoryInvocation {
        scope: ResourceScope {
            tenant_id: state.scope.tenant_id.clone(),
            user_id: actor.user_id.clone(),
            agent_id: state.scope.agent_id.clone(),
            project_id: state.scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(state.scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        correlation_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_threads::ThreadMessageId;

    fn record(
        sequence: u64,
        kind: MessageKind,
        status: MessageStatus,
        turn_run_id: &str,
        content: &str,
    ) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: ironclaw_host_api::ThreadId::new("thread-after-turn-test")
                .expect("valid thread id"),
            sequence,
            kind,
            status,
            created_at: None,
            updated_at: None,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: Some(turn_run_id.to_string()),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content.to_string()),
            attachments: Vec::new(),
            redaction_ref: None,
        }
    }

    /// H1 regression + mem0 parity: the after-turn transcript must capture the
    /// FULL ordered run transcript — every user/assistant/tool message tagged with
    /// this run, in sequence order, INCLUDING the final assistant and every
    /// intermediate step (not just the first finalized assistant, the prior
    /// `.find()` bug). `System` messages and messages from other runs are
    /// excluded; each message carries its actor `name`.
    #[test]
    fn build_transcript_captures_full_ordered_run_including_final_assistant() {
        let run = "run-under-test";
        let messages = vec![
            record(
                1,
                MessageKind::User,
                MessageStatus::Accepted,
                run,
                "please do X",
            ),
            record(
                2,
                MessageKind::Assistant,
                MessageStatus::Finalized,
                run,
                "step one: thinking",
            ),
            record(
                3,
                MessageKind::ToolResultReference,
                MessageStatus::Finalized,
                run,
                "tool output Y",
            ),
            record(
                4,
                MessageKind::Assistant,
                MessageStatus::Finalized,
                run,
                "final answer Z",
            ),
            // Excluded: a System message in the same run...
            record(
                5,
                MessageKind::System,
                MessageStatus::Finalized,
                run,
                "system note",
            ),
            // ...and a message belonging to a different run.
            record(
                6,
                MessageKind::Assistant,
                MessageStatus::Finalized,
                "other-run",
                "other run reply",
            ),
        ];

        let transcript = build_transcript(&messages, run, "user-abc", Some("agent-def"));

        let shape: Vec<(MemoryInteractionRole, &str, Option<&str>)> = transcript
            .iter()
            .map(|message| {
                (
                    message.role,
                    message.content.as_str(),
                    message.name.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (MemoryInteractionRole::User, "please do X", Some("user-abc")),
                (
                    MemoryInteractionRole::Assistant,
                    "step one: thinking",
                    Some("agent-def"),
                ),
                (MemoryInteractionRole::Tool, "tool output Y", None),
                (
                    MemoryInteractionRole::Assistant,
                    "final answer Z",
                    Some("agent-def"),
                ),
            ],
            "transcript must be the full ordered run: user, every finalized \
             assistant (including the FINAL one) and tool messages, each with its \
             actor name; System and other-run messages excluded"
        );
    }

    /// CR review ("LLM data is never deleted"): `build_transcript` filters out
    /// blank-only messages but must record the surviving content VERBATIM — it
    /// must not trim leading/trailing whitespace before handing the transcript to
    /// the provider (which alone decides verbatim-vs-extract).
    #[test]
    fn build_transcript_preserves_content_verbatim_and_filters_blanks() {
        let run = "run-verbatim";
        let messages = vec![
            record(
                1,
                MessageKind::User,
                MessageStatus::Accepted,
                run,
                "  surrounding whitespace kept  ",
            ),
            // A blank-only message has no usable content and is dropped entirely
            // (not recorded as an empty string).
            record(
                2,
                MessageKind::Assistant,
                MessageStatus::Finalized,
                run,
                "   \n  ",
            ),
            record(
                3,
                MessageKind::Assistant,
                MessageStatus::Finalized,
                run,
                "\tindented reply\n",
            ),
        ];

        let transcript = build_transcript(&messages, run, "user-abc", Some("agent-def"));

        let contents: Vec<&str> = transcript.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            vec!["  surrounding whitespace kept  ", "\tindented reply\n"],
            "surviving content must be byte-for-byte verbatim (no trim); blank-only \
             messages are filtered out entirely"
        );
    }
}
