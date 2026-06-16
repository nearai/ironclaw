//! Skill learning: turn-end skill distillation for the Reborn runtime.
//!
//! Mirrors the trace-capture sink (`trace_capture.rs`): every successful
//! terminal turn lifecycle event spawns a detached best-effort task that reads
//! the just-finished run's transcript and, when the run is substantive enough,
//! distills a reusable `SKILL.md` via the learning model. The distillation
//! *logic* lives in the `ironclaw_skill_learning` crate; this file owns the
//! composition seam: the eligibility gate, the transcript read, and the
//! inference adapter (and, in a later increment, staging the result for
//! approval + the scoped write).
//!
//! Skill learning requires a learning LLM provider, so the sink and its adapter
//! are gated on `root-llm-provider` (the feature that wires `ironclaw_llm`).
//! [`CompositeTurnEventSink`] is always available.
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
use ironclaw_turns::{TurnError, TurnEventSink, TurnLifecycleEvent};

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

#[cfg(feature = "root-llm-provider")]
pub(crate) use learning::{SkillLearningInferenceAdapter, SkillLearningTurnEventSink};

#[cfg(feature = "root-llm-provider")]
mod learning {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_llm::{ChatMessage, CompletionRequest, LlmProvider};
    use ironclaw_skill_learning::{
        DistillOutcome, SkillInferenceError, SkillInferencePort, distill_skill,
    };
    use ironclaw_threads::{
        ContextWindow, LoadContextWindowRequest, MessageKind, SessionThreadService, ThreadScope,
    };
    use ironclaw_turns::{TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent};

    /// Minimum substance for a completed run to be worth distilling into a
    /// skill, mirroring engine v2's skill-extraction mission gate (>=5 steps and
    /// >=3 tool actions). Transcript message count approximates "steps".
    const MIN_TOOL_ACTIONS: usize = 3;
    const MIN_TRANSCRIPT_MESSAGES: usize = 5;
    /// Recent-transcript bound for the eligibility read.
    const TRANSCRIPT_READ_LIMIT: usize = 64;

    /// Token ceiling for a distilled `SKILL.md` (well above the 15KB skill cap).
    const SKILL_LEARNING_MAX_TOKENS: u32 = 4096;
    /// Low temperature: distillation should be near-deterministic.
    const SKILL_LEARNING_TEMPERATURE: f32 = 0.2;

    /// Turn-end sink that distills a reusable skill from successful, substantive
    /// runs.
    pub(crate) struct SkillLearningTurnEventSink {
        thread_service: Arc<dyn SessionThreadService>,
        inference: Arc<dyn SkillInferencePort>,
    }

    impl SkillLearningTurnEventSink {
        pub(crate) fn new(
            thread_service: Arc<dyn SessionThreadService>,
            inference: Arc<dyn SkillInferencePort>,
        ) -> Self {
            Self {
                thread_service,
                inference,
            }
        }
    }

    #[async_trait]
    impl TurnEventSink for SkillLearningTurnEventSink {
        async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
            // Only successful completions are extraction candidates. Failed or
            // blocked runs are the self-improvement loop's concern.
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
            // capture sink. Skill writes (a later increment) MUST reuse this
            // scope so the learned skill lands where the WebUI lists it and the
            // next run loads it.
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
            let inference = Arc::clone(&self.inference);

            tokio::spawn(async move {
                // Read the model-context (replay) view, NOT list_thread_history:
                // the history projection nulls tool-call metadata for product
                // display, which would hide the very tool actions that make a
                // run worth distilling.
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

                let transcript = format_transcript(&window);
                match distill_skill(&transcript, inference.as_ref()).await {
                    Ok(DistillOutcome::Skill(skill)) => {
                        // TODO(skill-learning, increment 3): stage `skill` for
                        // one-click approval and, on approve, write it via the
                        // scoped skill-management port (event-derived scope).
                        tracing::debug!(
                            run_id = ?run_id,
                            skill = %skill.name,
                            bytes = skill.skill_md.len(),
                            "skill-learning: distilled a candidate skill (staging pending)"
                        );
                    }
                    Ok(DistillOutcome::Skipped(reason)) => {
                        tracing::debug!(
                            run_id = ?run_id,
                            ?reason,
                            "skill-learning: model declined to distill a skill"
                        );
                    }
                    Err(error) => {
                        tracing::debug!(%error, run_id = ?run_id, "skill-learning: distillation failed");
                    }
                }
            });
            Ok(())
        }
    }

    /// Render a context window into a role-labelled transcript for the
    /// distiller. Tool-result rows are prefixed with the real tool name so the
    /// distilled skill can name the exact tools that worked.
    fn format_transcript(window: &ContextWindow) -> String {
        let mut out = String::new();
        for message in &window.messages {
            let role = match message.kind {
                MessageKind::User => "user",
                MessageKind::Assistant => "assistant",
                MessageKind::ToolResultReference => "tool_result",
                MessageKind::System => "system",
                _ => continue,
            };
            if matches!(message.kind, MessageKind::ToolResultReference)
                && let Some(call) = message.tool_result_provider_call.as_ref()
            {
                out.push_str("tool_call: ");
                out.push_str(&call.provider_tool_name);
                out.push('\n');
            }
            out.push_str(role);
            out.push_str(": ");
            out.push_str(&message.content);
            out.push('\n');
        }
        out
    }

    /// Adapts a concrete strong-model [`LlmProvider`] to the logic crate's
    /// [`SkillInferencePort`]. The learning model is passed as a per-request
    /// override (NEAR AI honours it), so distillation runs against a stronger
    /// model than the run's without touching the run's model gateway.
    pub(crate) struct SkillLearningInferenceAdapter {
        provider: Arc<dyn LlmProvider>,
        model: String,
    }

    impl SkillLearningInferenceAdapter {
        pub(crate) fn new(provider: Arc<dyn LlmProvider>, model: String) -> Self {
            Self { provider, model }
        }
    }

    #[async_trait]
    impl SkillInferencePort for SkillLearningInferenceAdapter {
        async fn infer(&self, system: &str, user: &str) -> Result<String, SkillInferenceError> {
            let request =
                CompletionRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)])
                    .with_model(self.model.clone())
                    .with_max_tokens(SKILL_LEARNING_MAX_TOKENS)
                    .with_temperature(SKILL_LEARNING_TEMPERATURE);
            let response = self
                .provider
                .complete(request)
                .await
                .map_err(|error| SkillInferenceError(error.to_string()))?;
            Ok(response.content)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
        use ironclaw_threads::InMemorySessionThreadService;
        use ironclaw_turns::{EventCursor, TurnRunId, TurnScope, TurnStatus};

        struct StubInference;

        #[async_trait]
        impl SkillInferencePort for StubInference {
            async fn infer(
                &self,
                _system: &str,
                _user: &str,
            ) -> Result<String, SkillInferenceError> {
                Ok("SKIP: test stub".to_string())
            }
        }

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
            let sink = SkillLearningTurnEventSink::new(service, Arc::new(StubInference));
            // A failed run is the self-improvement loop's concern, not extraction.
            sink.publish(event(TurnEventKind::Failed, Some("alice")))
                .await
                .expect("failed event is a no-op");
            // A completion with no resolvable owner has nothing to attribute to.
            sink.publish(event(TurnEventKind::Completed, None))
                .await
                .expect("ownerless completion is a no-op");
        }
    }
}
