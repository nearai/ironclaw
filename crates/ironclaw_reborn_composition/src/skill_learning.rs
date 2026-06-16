//! Skill learning: turn-end skill-extraction trigger for the Reborn runtime.
//!
//! Mirrors the trace-capture sink (`trace_capture.rs`): every successful
//! terminal turn lifecycle event spawns a detached best-effort task that reads
//! the just-finished run's transcript and decides whether the run is
//! substantive enough to be worth distilling into a reusable skill. The
//! distillation itself (an LLM call against the learning model + staging the
//! resulting `SKILL.md` for approval) lands in a follow-up increment; this
//! file owns the seam and the eligibility gate.
//!
//! Invariants (shared with `trace_capture.rs`):
//! - Never block or fail the turn lifecycle path: the sink is subscribed
//!   best-effort and all work happens on a spawned task whose errors are
//!   logged at `debug!` only (`info!`/`warn!` corrupt the REPL).
//! - Scope is derived from the EVENT (tenant + owner), never from a runtime
//!   default — a wrong tenant writes a skill to a directory the WebUI and the
//!   next run never read (see `docs/plans/2026-06-16-reborn-skill-evolution.md`).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_threads::{LoadContextWindowRequest, MessageKind, SessionThreadService, ThreadScope};
use ironclaw_turns::{TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent};

/// Minimum substance for a completed run to be worth distilling into a skill,
/// mirroring engine v2's skill-extraction mission gate (>=5 steps and >=3 tool
/// actions). Transcript message count approximates "steps".
const MIN_TOOL_ACTIONS: usize = 3;
const MIN_TRANSCRIPT_MESSAGES: usize = 5;
/// Recent-transcript bound for the eligibility read.
const TRANSCRIPT_READ_LIMIT: usize = 64;

/// Composes several [`TurnEventSink`]s into one so the single
/// `turn_event_sink` slot can fan out to multiple best-effort consumers
/// (e.g. trace capture + skill learning). A child sink failure is logged at
/// `debug!` and never prevents the other children or fails the lifecycle path.
pub(crate) struct CompositeTurnEventSink {
    sinks: Vec<Arc<dyn TurnEventSink>>,
}

impl CompositeTurnEventSink {
    pub(crate) fn new(sinks: Vec<Arc<dyn TurnEventSink>>) -> Self {
        Self { sinks }
    }
}

#[async_trait]
impl TurnEventSink for CompositeTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        for sink in &self.sinks {
            if let Err(error) = sink.publish(event.clone()).await {
                tracing::debug!(%error, "composite turn event sink: child sink failed");
            }
        }
        Ok(())
    }
}

/// Turn-end sink that flags successful, substantive runs as skill-extraction
/// candidates.
pub(crate) struct SkillLearningTurnEventSink {
    thread_service: Arc<dyn SessionThreadService>,
}

impl SkillLearningTurnEventSink {
    pub(crate) fn new(thread_service: Arc<dyn SessionThreadService>) -> Self {
        Self { thread_service }
    }
}

#[async_trait]
impl TurnEventSink for SkillLearningTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        // Only successful completions are extraction candidates. Failed/blocked
        // runs are the self-improvement loop's concern, not skill extraction.
        if !matches!(event.kind, TurnEventKind::Completed) {
            return Ok(());
        }
        // System/sentinel-scoped turns have no owner to attribute a skill to.
        let Some(owner_user_id) = event
            .owner_user_id
            .clone()
            .or_else(|| event.scope.explicit_owner_user_id().cloned())
        else {
            return Ok(());
        };
        let Some(agent_id) = event.scope.agent_id.clone() else {
            return Ok(());
        };

        // Derive the read/write scope from the EVENT, mirroring the trace
        // capture sink. Skill writes (a later increment) MUST reuse this scope
        // so the learned skill lands where the WebUI lists it and the next run
        // loads it.
        let scope = ThreadScope {
            tenant_id: event.scope.tenant_id.clone(),
            agent_id,
            project_id: event.scope.project_id.clone(),
            owner_user_id: Some(owner_user_id),
            mission_id: None,
        };
        let thread_id = event.scope.thread_id.clone();
        let run_id = event.run_id;
        let thread_service = Arc::clone(&self.thread_service);

        tokio::spawn(async move {
            // Read the model-context (replay) view, NOT list_thread_history:
            // the history projection nulls tool-call metadata for product
            // display, which would hide the very tool actions that make a run
            // worth distilling.
            let window = match thread_service
                .load_context_window(LoadContextWindowRequest {
                    scope,
                    thread_id,
                    max_messages: TRANSCRIPT_READ_LIMIT,
                })
                .await
            {
                Ok(window) => window,
                Err(error) => {
                    tracing::debug!(%error, run_id = ?run_id, "skill-learning: could not load transcript");
                    return;
                }
            };

            let tool_actions = window
                .messages
                .iter()
                .filter(|message| matches!(message.kind, MessageKind::ToolResultReference))
                .count();
            let message_count = window.messages.len();
            if tool_actions < MIN_TOOL_ACTIONS || message_count < MIN_TRANSCRIPT_MESSAGES {
                return;
            }

            // TODO(skill-learning, increment 2): distill a SKILL.md from this
            // transcript via the learning model and stage it for one-click
            // approval (write via the scoped skill-management port).
            tracing::debug!(
                run_id = ?run_id,
                tool_actions,
                message_count,
                "skill-learning: run is a skill-extraction candidate (distillation pending)"
            );
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
    use ironclaw_threads::InMemorySessionThreadService;
    use ironclaw_turns::{EventCursor, TurnRunId, TurnScope, TurnStatus};

    fn event(kind: TurnEventKind, owner: Option<&str>) -> TurnLifecycleEvent {
        let owner_user_id = owner.map(|owner| UserId::new(owner).expect("test user id"));
        TurnLifecycleEvent {
            cursor: EventCursor::default(),
            scope: TurnScope::new_with_owner(
                TenantId::new("skill-learning-test-tenant").expect("tenant"),
                Some(AgentId::new("skill-learning-test-agent").expect("agent")),
                None,
                ThreadId::new("skill-learning-test-thread").expect("thread"),
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

    #[tokio::test]
    async fn ignores_non_completed_and_ownerless_completions() {
        let service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let sink = SkillLearningTurnEventSink::new(service);
        // A failed run is the self-improvement loop's concern, not extraction.
        sink.publish(event(TurnEventKind::Failed, Some("alice")))
            .await
            .expect("failed event is a no-op");
        // A completion with no resolvable owner has nothing to attribute a skill to.
        sink.publish(event(TurnEventKind::Completed, None))
            .await
            .expect("ownerless completion is a no-op");
    }
}
