//! After-turn interaction recording for the Reborn planned loop (mem0 `add`).
//!
//! At the run-end seam (a `Completed` run), the host hands the just-finished
//! user -> assistant exchange to the memory provider's
//! [`MemoryService::record_interaction`], mirroring
//! `mem0.add(messages=[user, assistant], user_id, run_id, metadata)`. The host
//! passes the interaction DATA and lets the provider decide what to record
//! (verbatim, LLM extraction, or nothing) — it makes NO verbatim-vs-extract /
//! provenance / TTL decision here.
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
    MessageKind, MessageStatus, SessionThreadService, ThreadHistory, ThreadHistoryRequest,
    ThreadScope,
};
use ironclaw_turns::{TurnActor, TurnRunState};
use tracing::debug;

use crate::thread_scope::ThreadScopeResolver;

/// Records the completed `[user, assistant]` exchange of a run into memory.
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
                debug!(error = %error, "after-turn memory: thread history read failed; skipping");
                return;
            }
        };

        let run_id = state.run_id.to_string();
        let Some(messages) = build_exchange(&history, &run_id) else {
            debug!("after-turn memory: no user/assistant exchange for run; skipping");
            return;
        };

        // Pass the raw interaction DATA; the provider decides what to do with it.
        // `user_id`/`agent_id`/`thread_id` ride the invocation scope.
        let invocation = invocation_for_run(state, actor);
        let request = MemoryServiceRecordRequest {
            messages,
            run_id: Some(run_id),
            metadata: serde_json::json!({}),
        };
        if let Err(error) = self
            .memory_writer
            .record_interaction(invocation, request)
            .await
        {
            debug!(error = %error, "after-turn memory: record_interaction failed; run already complete");
        }
    }
}

/// Build the `[user, assistant]` exchange for `run_id` from the thread history,
/// or `None` when either side (or its content) is missing.
fn build_exchange(history: &ThreadHistory, run_id: &str) -> Option<Vec<MemoryInteractionMessage>> {
    let user_content = history
        .messages
        .iter()
        .find(|message| {
            message.kind == MessageKind::User && message.turn_run_id.as_deref() == Some(run_id)
        })
        .and_then(|message| message.content.as_deref())?;
    let assistant_content = history
        .messages
        .iter()
        .find(|message| {
            message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.turn_run_id.as_deref() == Some(run_id)
        })
        .and_then(|message| message.content.as_deref())?;
    Some(vec![
        MemoryInteractionMessage {
            role: MemoryInteractionRole::User,
            content: user_content.to_string(),
        },
        MemoryInteractionMessage {
            role: MemoryInteractionRole::Assistant,
            content: assistant_content.to_string(),
        },
    ])
}

/// Build the memory invocation for a run: the thread is kept (short-term lane),
/// `user_id` is the run's actor. Mirrors `invocation_for_context_request` in
/// `ironclaw_host_runtime::memory_context`.
fn invocation_for_run(state: &TurnRunState, actor: &TurnActor) -> MemoryInvocation {
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
        correlation_id: CorrelationId::new(),
    }
}
