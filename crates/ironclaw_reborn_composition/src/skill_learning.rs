//! Skill learning: turn-end skill distillation for the Reborn runtime.
//!
//! Mirrors the trace-capture sink (`trace_capture.rs`): every successful
//! terminal turn lifecycle event spawns a detached best-effort task that reads
//! the just-finished run's transcript and, when the run is substantive enough,
//! distills a reusable `SKILL.md` via the learning model, safety-scans it,
//! installs it for the run's owner, and notifies the UI. The distillation
//! *logic* lives in the `ironclaw_skill_learning` crate; this file owns the
//! composition seam: the eligibility gate, the transcript read, the inference
//! adapter, the scoped write, and the learned-skill live notification.
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
//! - Distilled content is injection-scanned before it is installed (it becomes
//!   trusted prompt text loaded into the next run).

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
pub(crate) use learning::{
    LiveSkillLearnedNotifier, PortSkillWriter, SkillLearnedNotifier, SkillLearningInferenceAdapter,
    SkillLearningTurnEventSink, SkillWriter,
};

#[cfg(feature = "root-llm-provider")]
mod learning {
    use std::sync::{Arc, LazyLock};

    use async_trait::async_trait;
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, UserId};
    use ironclaw_llm::{ChatMessage, CompletionRequest, LlmProvider};
    use ironclaw_safety::{Sanitizer, validate_trusted_trigger_prompt};
    use ironclaw_skill_learning::{
        DistillOutcome, DistilledSkill, SkillInferenceError, SkillInferencePort, distill_skill,
    };
    use ironclaw_threads::{
        ContextWindow, LoadContextWindowRequest, MessageKind, SessionThreadService, ThreadScope,
    };
    use ironclaw_turns::{
        TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent, TurnRunId, TurnScope,
    };

    use crate::lifecycle::RebornLocalSkillManagementPort;
    use crate::projection::LiveProjectionPublisher;

    /// Cheap pre-filter: skip the (paid) distillation LLM call on runs that
    /// obviously can't yield a reusable skill (pure chat, a single lookup). The
    /// *real* quality gate is the learning model's own `SKIP` judgement, so this
    /// is kept lenient — an efficient agent may complete a skill-worthy,
    /// multi-step task in only two tool calls (e.g. `shell` mkdir + batch write).
    const MIN_TOOL_ACTIONS: usize = 2;
    const MIN_TRANSCRIPT_MESSAGES: usize = 3;
    /// Recent-transcript bound for the eligibility read.
    const TRANSCRIPT_READ_LIMIT: usize = 64;

    /// Output ceiling for a distilled `SKILL.md`. Generous: the learning model
    /// may be a reasoning model that spends tokens on reasoning before emitting
    /// the `SKILL.md`, so a tight cap would truncate the document.
    const SKILL_LEARNING_MAX_TOKENS: u32 = 16384;

    /// User-facing note shown on the learned-skill bubble.
    const LEARNED_SKILL_FEEDBACK: &str =
        "Learned this skill from the task you just completed — review it under Settings -> Skills.";

    /// Injection scanner applied to distilled skill content before install,
    /// mirroring the WebUI facade's `validate_skill_content_safety`.
    static SKILL_LEARNING_SAFETY: LazyLock<Sanitizer> = LazyLock::new(Sanitizer::new);

    /// Scoped skill write seam. Composition implements it over the real
    /// `RebornLocalSkillManagementPort`; tests use a stub. Keeps the sink
    /// testable without a filesystem.
    #[async_trait]
    pub(crate) trait SkillWriter: Send + Sync {
        /// Install the skill for `scope`, falling back to an in-place update
        /// when a skill of that name already exists (re-learning). Returns the
        /// stored skill name.
        async fn install_or_update(
            &self,
            scope: ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<String, String>;
    }

    /// [`SkillWriter`] over the runtime's scoped skill-management port.
    pub(crate) struct PortSkillWriter {
        port: Arc<RebornLocalSkillManagementPort>,
    }

    impl PortSkillWriter {
        pub(crate) fn new(port: Arc<RebornLocalSkillManagementPort>) -> Self {
            Self { port }
        }
    }

    #[async_trait]
    impl SkillWriter for PortSkillWriter {
        async fn install_or_update(
            &self,
            scope: ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<String, String> {
            match self
                .port
                .install_for_scope(scope.clone(), Some(name), content)
                .await
            {
                Ok(result) => Ok(result.name),
                // install is create-only; a name conflict means we are
                // re-learning an existing skill, so update it in place.
                Err(_) => self
                    .port
                    .update_for_scope(scope, name, content)
                    .await
                    .map(|_| name.to_string())
                    .map_err(|error| error.to_string()),
            }
        }
    }

    /// Live "learned a new skill" notification seam. Composition implements it
    /// over the projection publisher; tests use a stub.
    pub(crate) trait SkillLearnedNotifier: Send + Sync {
        fn notify(
            &self,
            owner: &UserId,
            scope: &TurnScope,
            run_id: TurnRunId,
            skill_name: &str,
            feedback: &str,
        );
    }

    /// [`SkillLearnedNotifier`] over the runtime's live projection publisher —
    /// emits a `SkillActivation` projection item rendered as a chat bubble.
    pub(crate) struct LiveSkillLearnedNotifier {
        publisher: Arc<LiveProjectionPublisher>,
    }

    impl LiveSkillLearnedNotifier {
        pub(crate) fn new(publisher: Arc<LiveProjectionPublisher>) -> Self {
            Self { publisher }
        }
    }

    impl SkillLearnedNotifier for LiveSkillLearnedNotifier {
        fn notify(
            &self,
            owner: &UserId,
            scope: &TurnScope,
            run_id: TurnRunId,
            skill_name: &str,
            feedback: &str,
        ) {
            self.publisher
                .publish_skill_learned(Some(owner), scope, run_id, skill_name, feedback);
        }
    }

    /// Turn-end sink that distills a reusable skill from successful, substantive
    /// runs, installs it for the run's owner, and notifies the UI.
    pub(crate) struct SkillLearningTurnEventSink {
        thread_service: Arc<dyn SessionThreadService>,
        inference: Arc<dyn SkillInferencePort>,
        skill_writer: Arc<dyn SkillWriter>,
        notifier: Arc<dyn SkillLearnedNotifier>,
    }

    impl SkillLearningTurnEventSink {
        pub(crate) fn new(
            thread_service: Arc<dyn SessionThreadService>,
            inference: Arc<dyn SkillInferencePort>,
            skill_writer: Arc<dyn SkillWriter>,
            notifier: Arc<dyn SkillLearnedNotifier>,
        ) -> Self {
            Self {
                thread_service,
                inference,
                skill_writer,
                notifier,
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

            // Read scope (transcript) and write scope (skill) both derive from
            // the EVENT, so the learned skill lands where the WebUI lists it and
            // the next run loads it.
            let read_scope = ThreadScope {
                tenant_id: event.scope.tenant_id.clone(),
                agent_id,
                project_id: event.scope.project_id.clone(),
                owner_user_id: Some(owner_user_id.clone()),
                mission_id: None,
            };
            let thread_id = event.scope.thread_id.clone();
            let run_id = event.run_id;
            let write_tenant = event.scope.tenant_id.clone();
            let write_owner = owner_user_id;
            // The full turn scope is needed to publish the learned-skill bubble
            // back to this thread's live stream.
            let event_scope = event.scope.clone();
            let thread_service = Arc::clone(&self.thread_service);
            let inference = Arc::clone(&self.inference);
            let skill_writer = Arc::clone(&self.skill_writer);
            let notifier = Arc::clone(&self.notifier);

            tokio::spawn(async move {
                // Read the model-context (replay) view, NOT list_thread_history:
                // the history projection nulls tool-call metadata for product
                // display, which would hide the very tool actions that make a
                // run worth distilling.
                let window = match thread_service
                    .load_context_window(LoadContextWindowRequest {
                        scope: read_scope,
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
                tracing::debug!(
                    run_id = ?run_id,
                    tool_actions,
                    message_count,
                    "skill-learning: evaluating completed run for extraction"
                );
                if tool_actions < MIN_TOOL_ACTIONS || message_count < MIN_TRANSCRIPT_MESSAGES {
                    return;
                }

                let transcript = format_transcript(&window);
                match distill_skill(&transcript, inference.as_ref()).await {
                    Ok(DistillOutcome::Skill(skill)) => {
                        if let Some(installed_name) = persist_learned_skill(
                            skill_writer.as_ref(),
                            &write_tenant,
                            &write_owner,
                            &skill,
                            run_id,
                        )
                        .await
                        {
                            // Best-effort live bubble on the run's thread stream.
                            notifier.notify(
                                &write_owner,
                                &event_scope,
                                run_id,
                                &installed_name,
                                LEARNED_SKILL_FEEDBACK,
                            );
                        }
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

    /// Safety-scan a distilled skill and, if it passes, install it for the
    /// run's (tenant, owner) scope. Returns the stored skill name on success.
    /// Best-effort: every failure exit is `debug!`-only.
    async fn persist_learned_skill(
        writer: &dyn SkillWriter,
        tenant: &TenantId,
        owner: &UserId,
        skill: &DistilledSkill,
        run_id: TurnRunId,
    ) -> Option<String> {
        // The distilled content becomes trusted prompt text loaded into the
        // next run, so injection-scan it first (High/Critical rejects).
        if let Err(rejection) =
            validate_trusted_trigger_prompt(&*SKILL_LEARNING_SAFETY, &skill.skill_md)
        {
            tracing::debug!(
                reason = rejection.reason(),
                run_id = ?run_id,
                skill = %skill.name,
                "skill-learning: distilled skill rejected by safety scan; not installed"
            );
            return None;
        }

        // Scope from the EVENT: start from the local default then override the
        // tenant with the run's, so the write lands where the WebUI/next run
        // read it (NOT the `default` tenant).
        let mut scope = match ResourceScope::local_default(owner.clone(), InvocationId::new()) {
            Ok(scope) => scope,
            Err(error) => {
                tracing::debug!(%error, run_id = ?run_id, "skill-learning: could not build write scope");
                return None;
            }
        };
        scope.tenant_id = tenant.clone();

        match writer
            .install_or_update(scope, &skill.name, &skill.skill_md)
            .await
        {
            Ok(name) => {
                tracing::debug!(
                    run_id = ?run_id,
                    skill = %name,
                    "skill-learning: installed learned skill (live)"
                );
                Some(name)
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    run_id = ?run_id,
                    skill = %skill.name,
                    "skill-learning: could not install learned skill"
                );
                None
            }
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
                    // No temperature override: reasoning models (e.g. gpt-5.x)
                    // reject any non-default temperature with HTTP 400.
                    .with_max_tokens(SKILL_LEARNING_MAX_TOKENS);
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
        use ironclaw_host_api::{AgentId, ThreadId};
        use ironclaw_threads::InMemorySessionThreadService;
        use ironclaw_turns::{EventCursor, TurnStatus};

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

        struct StubWriter;

        #[async_trait]
        impl SkillWriter for StubWriter {
            async fn install_or_update(
                &self,
                _scope: ResourceScope,
                _name: &str,
                _content: &str,
            ) -> Result<String, String> {
                Ok("stub".to_string())
            }
        }

        struct StubNotifier;

        impl SkillLearnedNotifier for StubNotifier {
            fn notify(
                &self,
                _owner: &UserId,
                _scope: &TurnScope,
                _run_id: TurnRunId,
                _skill_name: &str,
                _feedback: &str,
            ) {
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
            let sink = SkillLearningTurnEventSink::new(
                service,
                Arc::new(StubInference),
                Arc::new(StubWriter),
                Arc::new(StubNotifier),
            );
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
