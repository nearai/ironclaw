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
    LiveSkillLearnedNotifier, LlmSkillRefiner, PortSkillWriter, SkillLearnedNotifier,
    SkillLearningInferenceAdapter, SkillLearningTurnEventSink, SkillRefiner, SkillWriter,
};

#[cfg(feature = "root-llm-provider")]
mod learning {
    use std::collections::BTreeSet;
    use std::sync::{Arc, LazyLock};

    use async_trait::async_trait;
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, ThreadId, UserId};
    use ironclaw_llm::{ChatMessage, CompletionRequest, LlmProvider};
    use ironclaw_safety::{Sanitizer, validate_trusted_trigger_prompt};
    use ironclaw_skill_learning::{
        DistillOutcome, DistilledSkill, RefineOutcome, SkillInferenceError, SkillInferencePort,
        distill_skill, refine_skill,
    };
    use ironclaw_skills::{ManagedSkillSource, SkillSummary, parse_skill_md};
    use ironclaw_threads::{
        AppendAssistantDraftRequest, ContextWindow, LoadContextWindowRequest, MessageContent,
        MessageKind, SessionThreadService, ThreadScope,
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

    /// Jaccard-similarity floor (over the combined name/keyword/tag token sets)
    /// above which a freshly distilled skill is treated as a near-duplicate of
    /// an existing learned skill and consolidated into it rather than installed
    /// under a new name. Tuned to merge obvious siblings (the model gives the
    /// same kind of task slightly different names/keywords each run) without
    /// collapsing genuinely distinct skills.
    const SKILL_DEDUP_SIMILARITY_THRESHOLD: f64 = 0.45;
    /// A skill needs at least this many distinctive tokens before dedup runs;
    /// below it, overlap is too noisy to trust.
    const MIN_DEDUP_TOKENS: usize = 2;
    /// Generic tokens stripped before similarity so two skills don't look alike
    /// merely because both say "run"/"use"/"the".
    const DEDUP_STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "from", "that", "this", "your", "you", "run", "use", "get",
        "set", "new", "via",
    ];

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

    /// What to do with an existing learned skill when a near-duplicate candidate
    /// is distilled for the same task.
    #[derive(Debug)]
    pub(crate) enum MergeAction {
        /// Update the existing skill with this refined, install-ready content.
        Replace(String),
        /// Leave the existing skill unchanged — it already subsumes the candidate.
        KeepExisting,
        /// Refinement was unavailable; overwrite the existing skill with the
        /// candidate (the plain dedup-consolidation behavior).
        Overwrite,
    }

    /// Self-improvement seam: merge a freshly distilled candidate into an
    /// existing learned skill for the same task. Composition implements it over
    /// the learning model; tests use a stub.
    #[async_trait]
    pub(crate) trait SkillRefiner: Send + Sync {
        /// Decide how to fold `candidate` into the existing skill `existing`,
        /// whose stored name is `target_name`.
        async fn merge(&self, existing: &str, candidate: &str, target_name: &str) -> MergeAction;
    }

    /// [`SkillRefiner`] over the learning model: asks it to combine the existing
    /// and candidate `SKILL.md` into a strictly better document (accumulated
    /// gotchas, bumped version), then retargets and injection-scans the result
    /// before it can be installed.
    pub(crate) struct LlmSkillRefiner {
        inference: Arc<dyn SkillInferencePort>,
    }

    impl LlmSkillRefiner {
        pub(crate) fn new(inference: Arc<dyn SkillInferencePort>) -> Self {
            Self { inference }
        }
    }

    #[async_trait]
    impl SkillRefiner for LlmSkillRefiner {
        async fn merge(&self, existing: &str, candidate: &str, target_name: &str) -> MergeAction {
            match refine_skill(existing, candidate, self.inference.as_ref()).await {
                Ok(RefineOutcome::Refined(skill)) => {
                    // Never trust the model to preserve the name; retarget so
                    // `update_skill`'s name-consistency check passes.
                    let retargeted = rewrite_skill_name(&skill.skill_md, target_name);
                    // The refined document is new trusted prompt text loaded into
                    // the next run, so injection-scan it before it can install.
                    if validate_trusted_trigger_prompt(&*SKILL_LEARNING_SAFETY, &retargeted)
                        .is_err()
                    {
                        tracing::debug!(
                            skill = %target_name,
                            "skill-learning: refined skill rejected by safety scan; consolidating instead"
                        );
                        return MergeAction::Overwrite;
                    }
                    MergeAction::Replace(retargeted)
                }
                Ok(RefineOutcome::KeepExisting) => MergeAction::KeepExisting,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        skill = %target_name,
                        "skill-learning: refinement failed; consolidating instead"
                    );
                    MergeAction::Overwrite
                }
            }
        }
    }

    /// [`SkillWriter`] over the runtime's scoped skill-management port.
    pub(crate) struct PortSkillWriter {
        port: Arc<RebornLocalSkillManagementPort>,
        refiner: Arc<dyn SkillRefiner>,
    }

    impl PortSkillWriter {
        pub(crate) fn new(
            port: Arc<RebornLocalSkillManagementPort>,
            refiner: Arc<dyn SkillRefiner>,
        ) -> Self {
            Self { port, refiner }
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
            // Consolidate near-duplicates: the distiller gives the same kind of
            // task a slightly different name each run, so without this the
            // user's skill list fills with siblings that never get reused
            // together. When an existing learned skill covers the same ground,
            // evolve it in place under its existing name instead of installing a
            // second one — the self-improvement loop: the model folds the new
            // evidence (more gotchas, clearer steps, a bumped version) into the
            // existing skill. `update_skill` requires the document's frontmatter
            // `name` to equal the target, so the merged content is retargeted.
            if let Some(existing_name) = self.find_duplicate(&scope, name, content).await {
                let existing = self
                    .port
                    .read_content_for_scope(scope.clone(), &existing_name)
                    .await
                    .ok();
                let action = match existing.as_ref() {
                    Some(existing) => {
                        self.refiner
                            .merge(&existing.content, content, &existing_name)
                            .await
                    }
                    None => MergeAction::Overwrite,
                };
                let merged = match action {
                    MergeAction::Replace(refined) => {
                        tracing::debug!(
                            distilled_name = %name,
                            refined_into = %existing_name,
                            "skill-learning: refined existing learned skill from a recurring task"
                        );
                        refined
                    }
                    MergeAction::KeepExisting => {
                        tracing::debug!(
                            distilled_name = %name,
                            kept = %existing_name,
                            "skill-learning: existing learned skill already subsumes candidate"
                        );
                        return Ok(existing_name);
                    }
                    MergeAction::Overwrite => {
                        tracing::debug!(
                            distilled_name = %name,
                            merged_into = %existing_name,
                            "skill-learning: consolidating near-duplicate learned skill"
                        );
                        rewrite_skill_name(content, &existing_name)
                    }
                };
                return self
                    .port
                    .update_for_scope(scope, &existing_name, &merged)
                    .await
                    .map(|_| existing_name)
                    .map_err(|error| error.to_string());
            }
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

    impl PortSkillWriter {
        /// Find an existing learned skill at `scope` that the freshly distilled
        /// `content` is a near-duplicate of, returning its stored name so the
        /// caller can refine it in place. Best-effort: a parse/listing failure
        /// returns `None` (fall through to a normal install).
        async fn find_duplicate(
            &self,
            scope: &ResourceScope,
            name: &str,
            content: &str,
        ) -> Option<String> {
            let parsed = parse_skill_md(content).ok()?;
            let new_tokens = skill_token_set(
                &parsed.manifest.name,
                &parsed.manifest.activation.keywords,
                &parsed.manifest.activation.tags,
            );
            if new_tokens.len() < MIN_DEDUP_TOKENS {
                return None;
            }
            let existing = self.port.list_for_scope(scope.clone()).await.ok()?;
            select_duplicate_skill(name, &new_tokens, &existing)
        }
    }

    /// Pick the existing learned skill most similar to a candidate token set,
    /// if any clears [`SKILL_DEDUP_SIMILARITY_THRESHOLD`]. Only `User`-source
    /// skills are merge targets (never system or registry-installed skills),
    /// and the same-named skill is skipped (that is a plain re-learn, handled by
    /// the install→update-on-conflict path). Pure so it is unit-testable without
    /// a filesystem-backed skill port.
    fn select_duplicate_skill(
        new_name: &str,
        new_tokens: &BTreeSet<String>,
        existing: &[SkillSummary],
    ) -> Option<String> {
        let mut best: Option<(f64, &str)> = None;
        for summary in existing {
            if !matches!(summary.source, ManagedSkillSource::User) || summary.name == new_name {
                continue;
            }
            let tokens = skill_token_set(&summary.name, &summary.keywords, &summary.tags);
            let score = jaccard_similarity(new_tokens, &tokens);
            if score >= SKILL_DEDUP_SIMILARITY_THRESHOLD
                && best
                    .as_ref()
                    .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, summary.name.as_str()));
            }
        }
        best.map(|(_, name)| name.to_string())
    }

    /// Distinctive lowercase token set for a skill, drawn from its name,
    /// keywords, and tags. Splits on non-alphanumeric so `read-file` and
    /// `character-count` contribute `read`/`file`/`character`/`count`; drops
    /// short and generic tokens so similarity reflects real subject overlap.
    fn skill_token_set(name: &str, keywords: &[String], tags: &[String]) -> BTreeSet<String> {
        let mut tokens = BTreeSet::new();
        let sources = std::iter::once(name)
            .chain(keywords.iter().map(String::as_str))
            .chain(tags.iter().map(String::as_str));
        for source in sources {
            for token in source.split(|character: char| !character.is_ascii_alphanumeric()) {
                if token.len() < 3 {
                    continue;
                }
                let lowered = token.to_ascii_lowercase();
                if !DEDUP_STOPWORDS.contains(&lowered.as_str()) {
                    tokens.insert(lowered);
                }
            }
        }
        tokens
    }

    /// Jaccard similarity (|intersection| / |union|) of two token sets. Empty
    /// sets are dissimilar (0.0), never accidentally "identical".
    fn jaccard_similarity(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
        if left.is_empty() || right.is_empty() {
            return 0.0;
        }
        let intersection = left.intersection(right).count();
        let union = left.len() + right.len() - intersection;
        if union == 0 {
            0.0
        } else {
            intersection as f64 / union as f64
        }
    }

    /// Rewrite the frontmatter `name:` of a `SKILL.md` to `new_name` so the
    /// document satisfies `update_skill`'s name-consistency check when a
    /// distilled skill is merged into an existing one. Only the first `name:`
    /// line inside the leading `---` frontmatter block is touched.
    fn rewrite_skill_name(content: &str, new_name: &str) -> String {
        let mut out = String::with_capacity(content.len() + new_name.len());
        let mut seen_open = false;
        let mut in_frontmatter = false;
        let mut replaced = false;
        for line in content.lines() {
            if !replaced && line.trim() == "---" {
                if !seen_open {
                    seen_open = true;
                    in_frontmatter = true;
                } else {
                    in_frontmatter = false;
                }
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_frontmatter && !replaced && line.trim_start().starts_with("name:") {
                let indent_len = line.len() - line.trim_start().len();
                out.push_str(&line[..indent_len]);
                out.push_str("name: ");
                out.push_str(new_name);
                out.push('\n');
                replaced = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
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
            let job = ExtractionJob {
                thread_service: Arc::clone(&self.thread_service),
                inference: Arc::clone(&self.inference),
                skill_writer: Arc::clone(&self.skill_writer),
                notifier: Arc::clone(&self.notifier),
                scope: read_scope,
                thread_id,
                run_id,
                write_tenant,
                write_owner,
                event_scope,
            };
            tokio::spawn(job.run());
            Ok(())
        }
    }

    /// The post-run extraction work, lifted out of [`SkillLearningTurnEventSink::publish`]
    /// so it is a single owned future the sink can `tokio::spawn` AND a test can
    /// drive to completion with `.await` (the spawn is fire-and-forget, so the
    /// durable announce below is otherwise untestable through its caller).
    struct ExtractionJob {
        thread_service: Arc<dyn SessionThreadService>,
        inference: Arc<dyn SkillInferencePort>,
        skill_writer: Arc<dyn SkillWriter>,
        notifier: Arc<dyn SkillLearnedNotifier>,
        /// Read scope (transcript) and write/announce scope are the same — both
        /// derive from the turn event.
        scope: ThreadScope,
        thread_id: ThreadId,
        run_id: TurnRunId,
        write_tenant: TenantId,
        write_owner: UserId,
        event_scope: TurnScope,
    }

    impl ExtractionJob {
        async fn run(self) {
            // Read the model-context (replay) view, NOT list_thread_history:
            // the history projection nulls tool-call metadata for product
            // display, which would hide the very tool actions that make a
            // run worth distilling.
            let window = match self
                .thread_service
                .load_context_window(LoadContextWindowRequest {
                    scope: self.scope.clone(),
                    thread_id: self.thread_id.clone(),
                    max_messages: TRANSCRIPT_READ_LIMIT,
                })
                .await
            {
                Ok(window) => window,
                Err(error) => {
                    tracing::debug!(%error, run_id = ?self.run_id, "skill-learning: could not load transcript");
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
                run_id = ?self.run_id,
                tool_actions,
                message_count,
                "skill-learning: evaluating completed run for extraction"
            );
            if tool_actions < MIN_TOOL_ACTIONS || message_count < MIN_TRANSCRIPT_MESSAGES {
                return;
            }

            let transcript = format_transcript(&window);
            match distill_skill(&transcript, self.inference.as_ref()).await {
                Ok(DistillOutcome::Skill(skill)) => {
                    if let Some(installed_name) = persist_learned_skill(
                        self.skill_writer.as_ref(),
                        &self.write_tenant,
                        &self.write_owner,
                        &skill,
                        self.run_id,
                    )
                    .await
                    {
                        // Durable feedback first: a thread message survives a
                        // page reload and renders from `get_timeline`, so the
                        // user sees the learned-skill notice even when no live
                        // stream was connected at publish time. The live bubble
                        // below is the ephemeral, best-effort fast path.
                        announce_learned_skill(
                            self.thread_service.as_ref(),
                            &self.scope,
                            &self.thread_id,
                            self.run_id,
                            &installed_name,
                        )
                        .await;
                        self.notifier.notify(
                            &self.write_owner,
                            &self.event_scope,
                            self.run_id,
                            &installed_name,
                            LEARNED_SKILL_FEEDBACK,
                        );
                    }
                }
                Ok(DistillOutcome::Skipped(reason)) => {
                    tracing::debug!(
                        run_id = ?self.run_id,
                        ?reason,
                        "skill-learning: model declined to distill a skill"
                    );
                }
                Err(error) => {
                    tracing::debug!(%error, run_id = ?self.run_id, "skill-learning: distillation failed");
                }
            }
        }
    }

    /// Append a durable "learned a new skill" note to the run's thread. Unlike
    /// the live [`SkillLearnedNotifier`] bubble (ephemeral, only delivered to a
    /// connected SSE/WS stream), this finalized assistant message is persisted,
    /// so the user sees the feedback in the conversation on the next timeline
    /// load even if no stream was open ~seconds after the run when distillation
    /// finished. Best-effort: every failure exit is `debug!`-only.
    async fn announce_learned_skill(
        thread_service: &dyn SessionThreadService,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        run_id: TurnRunId,
        skill_name: &str,
    ) {
        let note = format!(
            "🎓 I learned a new skill from this task: **{skill_name}**. \
             I'll apply it automatically on similar requests — manage it under Settings → Skills."
        );
        let draft = match thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: run_id.to_string(),
                content: MessageContent::text(note.clone()),
            })
            .await
        {
            Ok(record) => record,
            Err(error) => {
                tracing::debug!(%error, run_id = ?run_id, "skill-learning: could not append learned-skill note");
                return;
            }
        };
        if let Err(error) = thread_service
            .finalize_assistant_message(
                scope,
                thread_id,
                draft.message_id,
                MessageContent::text(note),
            )
            .await
        {
            tracing::debug!(%error, run_id = ?run_id, "skill-learning: could not finalize learned-skill note");
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
        use ironclaw_threads::{
            EnsureThreadRequest, InMemorySessionThreadService, ThreadHistoryRequest,
        };
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

        // Durable feedback regression: a learned skill must leave a persisted
        // assistant note in the thread so the user sees it from `get_timeline`
        // even when no live SSE/WS stream was connected when distillation
        // finished. (The live bubble is best-effort and ephemeral.)
        #[tokio::test]
        async fn appends_durable_learned_skill_note_to_thread() {
            let service: Arc<dyn SessionThreadService> =
                Arc::new(InMemorySessionThreadService::default());
            let scope = ThreadScope {
                tenant_id: TenantId::new("announce-tenant").expect("tenant"),
                agent_id: AgentId::new("announce-agent").expect("agent"),
                project_id: None,
                owner_user_id: Some(UserId::new("announce-user").expect("user")),
                mission_id: None,
            };
            let thread_id = ThreadId::new("announce-thread").expect("thread");
            service
                .ensure_thread(EnsureThreadRequest {
                    scope: scope.clone(),
                    thread_id: Some(thread_id.clone()),
                    created_by_actor_id: "announce-user".to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .expect("ensure thread");

            announce_learned_skill(
                service.as_ref(),
                &scope,
                &thread_id,
                TurnRunId::new(),
                "file-character-count-roundtrip",
            )
            .await;

            let history = service
                .list_thread_history(ThreadHistoryRequest { scope, thread_id })
                .await
                .expect("history");
            assert!(
                history.messages.iter().any(|message| {
                    matches!(message.kind, MessageKind::Assistant)
                        && message.content.as_deref().is_some_and(|content| {
                            content.contains("file-character-count-roundtrip")
                                && content.contains("learned a new skill")
                        })
                }),
                "a durable assistant note naming the learned skill must be persisted: {:#?}",
                history.messages
            );
        }

        fn summary(
            name: &str,
            keywords: &[&str],
            tags: &[&str],
            source: ManagedSkillSource,
        ) -> SkillSummary {
            SkillSummary {
                name: name.to_string(),
                version: "1".to_string(),
                description: String::new(),
                source,
                keywords: keywords.iter().map(|k| k.to_string()).collect(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                requires_skills: Vec::new(),
            }
        }

        #[test]
        fn jaccard_similarity_basics() {
            let a: BTreeSet<String> = ["file", "count", "read"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let b = a.clone();
            assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
            let disjoint: BTreeSet<String> =
                ["github", "pull"].iter().map(|s| s.to_string()).collect();
            assert_eq!(jaccard_similarity(&a, &disjoint), 0.0);
            assert_eq!(jaccard_similarity(&a, &BTreeSet::new()), 0.0);
        }

        #[test]
        fn skill_token_set_splits_lowercases_and_filters() {
            let tokens = skill_token_set(
                "Read-File",
                &["character-count".to_string(), "the".to_string()],
                &["FS".to_string()],
            );
            // hyphen split + lowercase
            assert!(tokens.contains("read"));
            assert!(tokens.contains("file"));
            assert!(tokens.contains("character"));
            assert!(tokens.contains("count"));
            // stopword dropped, sub-3-char token dropped
            assert!(!tokens.contains("the"));
            assert!(!tokens.contains("fs"));
        }

        #[test]
        fn select_duplicate_skill_merges_sibling_file_count_skills() {
            // The exact demo failure: three near-identical file-character-count
            // skills accreted under different names. A fourth sibling must
            // consolidate into the first instead of becoming a fourth.
            let new_tokens = skill_token_set(
                "file-character-count-roundtrip",
                &[
                    "file".to_string(),
                    "character-count".to_string(),
                    "read-file".to_string(),
                ],
                &["files".to_string()],
            );
            let existing = vec![
                summary(
                    "file-create-read-count-summary",
                    &["file", "read-file", "character-count", "write-file"],
                    &["files"],
                    ManagedSkillSource::User,
                ),
                summary(
                    "github-pr-review",
                    &["github", "pull-request", "review"],
                    &["git"],
                    ManagedSkillSource::User,
                ),
            ];
            assert_eq!(
                select_duplicate_skill("file-character-count-roundtrip", &new_tokens, &existing)
                    .as_deref(),
                Some("file-create-read-count-summary")
            );
        }

        #[test]
        fn select_duplicate_skill_skips_system_same_name_and_unrelated() {
            let new_tokens = skill_token_set(
                "file-character-count-roundtrip",
                &["file".to_string(), "character-count".to_string()],
                &[],
            );
            // A system skill with identical tokens is never a merge target.
            let system_twin = summary(
                "file-character-count-roundtrip-system",
                &["file", "character-count"],
                &[],
                ManagedSkillSource::System,
            );
            // Same-named user skill is a plain re-learn, not a dedup target.
            let same_name = summary(
                "file-character-count-roundtrip",
                &["file", "character-count"],
                &[],
                ManagedSkillSource::User,
            );
            // Unrelated user skill is below threshold.
            let unrelated = summary(
                "send-slack-message",
                &["slack", "message"],
                &["chat"],
                ManagedSkillSource::User,
            );
            assert_eq!(
                select_duplicate_skill(
                    "file-character-count-roundtrip",
                    &new_tokens,
                    &[system_twin, same_name, unrelated]
                ),
                None
            );
        }

        #[test]
        fn rewrite_skill_name_retargets_frontmatter_only() {
            let content = "---\nname: file-character-count-roundtrip\nversion: 1\ndescription: count chars\nactivation:\n  keywords: [file, count]\n---\n\n# Title\n\nname: not-the-frontmatter\nBody.\n";
            let rewritten = rewrite_skill_name(content, "file-create-read-count-summary");
            let parsed = parse_skill_md(&rewritten).expect("rewritten skill parses");
            assert_eq!(parsed.manifest.name, "file-create-read-count-summary");
            // The body line that merely starts with `name:` must be untouched.
            assert!(parsed.prompt_content.contains("name: not-the-frontmatter"));
            assert!(parsed.prompt_content.contains("Body."));
        }

        struct CannedInference {
            response: String,
        }

        #[async_trait]
        impl SkillInferencePort for CannedInference {
            async fn infer(
                &self,
                _system: &str,
                _user: &str,
            ) -> Result<String, SkillInferenceError> {
                Ok(self.response.clone())
            }
        }

        const EXISTING_SKILL: &str = "---\nname: file-count\nversion: 1\ndescription: count chars\nactivation:\n  keywords: [file, count]\n---\n\n# File Count\n\n## Steps\n\n1. read the file\n";
        const REFINED_RESPONSE: &str = "---\nname: file-count\nversion: 2\ndescription: count chars\nactivation:\n  keywords: [file, count, character]\n---\n\n# File Count\n\n## Gotchas\n\n- spaces count too\n";

        fn refiner(response: &str) -> LlmSkillRefiner {
            LlmSkillRefiner::new(Arc::new(CannedInference {
                response: response.to_string(),
            }))
        }

        #[tokio::test]
        async fn refiner_replaces_with_refined_and_bumped_skill() {
            let action = refiner(REFINED_RESPONSE)
                .merge(EXISTING_SKILL, EXISTING_SKILL, "file-count")
                .await;
            match action {
                MergeAction::Replace(content) => {
                    assert!(content.contains("name: file-count"));
                    assert!(content.contains("version: 2"));
                    assert!(content.contains("spaces count too"));
                }
                other => panic!("expected Replace, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn refiner_retargets_a_model_renamed_skill() {
            // The model returns a refined skill under the WRONG name; merge must
            // retarget it to the existing name so `update_skill`'s name check
            // passes instead of erroring.
            let renamed = REFINED_RESPONSE.replace("name: file-count", "name: file-count-renamed");
            match refiner(&renamed)
                .merge(EXISTING_SKILL, EXISTING_SKILL, "file-count")
                .await
            {
                MergeAction::Replace(content) => {
                    let parsed = parse_skill_md(&content).expect("retargeted skill parses");
                    assert_eq!(parsed.manifest.name, "file-count");
                }
                other => panic!("expected Replace, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn refiner_keeps_existing_on_keep_sentinel() {
            assert!(matches!(
                refiner("KEEP")
                    .merge(EXISTING_SKILL, EXISTING_SKILL, "file-count")
                    .await,
                MergeAction::KeepExisting
            ));
        }

        #[tokio::test]
        async fn refiner_overwrites_when_model_output_is_unparseable() {
            // A chatty/garbage response must not poison the skill; fall back to
            // plain consolidation (overwrite with the candidate).
            assert!(matches!(
                refiner("sure, here is the merged skill")
                    .merge(EXISTING_SKILL, EXISTING_SKILL, "file-count")
                    .await,
                MergeAction::Overwrite
            ));
        }
    }
}
