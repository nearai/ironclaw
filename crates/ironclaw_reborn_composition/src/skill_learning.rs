//! Skill learning: turn-end skill distillation for the Reborn runtime.
//!
//! Mirrors the trace-capture sink (`trace_capture.rs`): every successful
//! terminal turn lifecycle event spawns a supervised best-effort task that reads
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
//!   best-effort and all work happens on a spawned task owned by the runtime
//!   shutdown path; task errors are logged at `debug!` only (`info!`/`warn!`
//!   corrupt the REPL).
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
    SkillLearningExtractionTasks, SkillLearningInferenceAdapter, SkillLearningTurnEventSink,
    SkillRefiner, SkillWriter,
};

#[cfg(feature = "root-llm-provider")]
mod learning {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, LazyLock, Mutex};

    use async_trait::async_trait;
    use ironclaw_host_api::{InvocationId, ResourceScope, TenantId, ThreadId, UserId};
    use ironclaw_llm::{ChatMessage, CompletionRequest, LlmProvider};
    use ironclaw_safety::{Sanitizer, validate_trusted_trigger_prompt};
    use ironclaw_skill_learning::{
        DistillOutcome, DistilledSkill, RefineOutcome, SkillInferenceError, SkillInferencePort,
        distill_skill, refine_skill,
    };
    use ironclaw_skills::{
        LearnedSkillProvenance, ManagedSkillSource, SkillManagementErrorKind, SkillOrigin,
        SkillSummary, parse_skill_md, set_skill_auto_activate, set_skill_origin,
    };
    use ironclaw_threads::{
        AppendAssistantDraftRequest, ContextWindow, LoadContextWindowRequest, MessageContent,
        MessageKind, SessionThreadService, ThreadHistoryRequest, ThreadScope,
    };
    use ironclaw_turns::{
        TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent, TurnRunId, TurnScope,
    };
    use tokio::task::JoinHandle;

    use crate::lifecycle::{RebornLocalSkillManagementError, RebornLocalSkillManagementPort};
    use crate::projection::LiveProjectionPublisher;

    /// Cheap pre-filter: skip the (paid) distillation LLM call on runs that
    /// obviously can't yield a reusable skill (pure chat, a one/two-shot lookup).
    /// Modeled on Hermes's "5+ tool calls" trigger and tuned down one notch: a
    /// genuinely multi-step task still clears 4, while one/two-shot lookups no
    /// longer each pay for a ~16K-token distillation call. The *real* quality gate
    /// is still the learning model's own `SKIP` judgement; this is the cheapest
    /// lever for cutting distillation spend.
    const MIN_TOOL_ACTIONS: usize = 4;
    const MIN_TRANSCRIPT_MESSAGES: usize = 3;
    /// Recent-transcript bound for the eligibility read.
    const TRANSCRIPT_READ_LIMIT: usize = 64;

    /// Output ceiling for a distilled `SKILL.md`. Generous: the learning model
    /// may be a reasoning model that spends tokens on reasoning before emitting
    /// the `SKILL.md`, so a tight cap would truncate the document.
    const SKILL_LEARNING_MAX_TOKENS: u32 = 16384;

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

    /// Stamp the `origin: learned` frontmatter marker into `content`. Every skill
    /// this writer installs or evolves is machine-learned by definition; the marker
    /// travels with the skill (export, tenant share) and lets the selector scope
    /// the global auto-activate-learned switch to learned skills and the UI badge a
    /// skill's origin. Applied at each live-write point — not once on entry —
    /// because the refiner model rewrites the document and may drop the line.
    fn mark_learned(content: &str) -> String {
        set_skill_origin(content, SkillOrigin::Learned)
    }

    /// Outcome of a learning-sink skill write, so the sink can notify the user
    /// truthfully: only `Installed`/`Evolved` actually changed an active skill.
    #[derive(Debug)]
    pub(crate) enum SkillWriteOutcome {
        /// A brand-new learned skill was installed and will auto-apply.
        Installed(String),
        /// "Hold for review" is on: a brand-new learned skill was saved but NOT
        /// auto-activated, marked pending the user's approval.
        Pending(String),
        /// An existing machine-owned skill was evolved in place. `active` carries the
        /// skill's preserved auto-activation state so the sink can word the user
        /// notice honestly (a skill the user turned off must not be announced as
        /// "active").
        Evolved { name: String, active: bool },
        /// The target is human-owned/edited; the distillation was stashed as a
        /// proposal for review and the live skill was NOT changed.
        Proposed(String),
        /// Nothing was written — a bundle, a human-built skill, or the model
        /// judged the existing skill already sufficient.
        Skipped,
    }

    /// Scoped skill write seam. Composition implements it over the real
    /// `RebornLocalSkillManagementPort`; tests use a stub. Keeps the sink
    /// testable without a filesystem.
    #[async_trait]
    pub(crate) trait SkillWriter: Send + Sync {
        /// Install a new learned skill, evolve an existing machine-owned one, or
        /// — when the target is human-owned/edited — stash a proposal instead of
        /// overwriting. The returned [`SkillWriteOutcome`] tells the caller
        /// whether a skill actually changed, so it notifies the user honestly.
        async fn install_or_update(
            &self,
            scope: ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<SkillWriteOutcome, String>;
    }

    /// What to do with an existing learned skill when a near-duplicate candidate
    /// is distilled for the same task.
    #[derive(Debug)]
    pub(crate) enum MergeAction {
        /// Update the existing skill with this refined, install-ready content.
        Replace(String),
        /// Leave the existing skill unchanged — it already subsumes the
        /// candidate, OR refinement failed and the accumulated skill must not be
        /// discarded for a single-run candidate (see [`SkillRefiner::merge`]).
        KeepExisting,
        /// The existing skill's stored content could not be read, so there is no
        /// accumulated document to preserve; write the candidate under the
        /// existing name. Reserved for that genuinely-no-content case — a refiner
        /// error or a rejected refinement keeps the existing skill instead.
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
                        // A false positive on the MERGED doc must not cost the
                        // accumulated skill: keep the existing one rather than
                        // overwrite it with the raw single-run candidate.
                        tracing::debug!(
                            skill = %target_name,
                            "skill-learning: refined skill rejected by safety scan; keeping existing skill instead"
                        );
                        return MergeAction::KeepExisting;
                    }
                    MergeAction::Replace(retargeted)
                }
                Ok(RefineOutcome::KeepExisting) => MergeAction::KeepExisting,
                Err(error) => {
                    // A transient model hiccup or an unparseable response is
                    // exactly when the single-run candidate is least trustworthy,
                    // so preserve the evolved skill instead of regressing it to a
                    // fresh distillation.
                    tracing::debug!(
                        %error,
                        skill = %target_name,
                        "skill-learning: refinement failed; keeping existing skill instead"
                    );
                    MergeAction::KeepExisting
                }
            }
        }
    }

    /// [`SkillWriter`] over the runtime's scoped skill-management port.
    pub(crate) struct PortSkillWriter {
        pub(super) port: Arc<RebornLocalSkillManagementPort>,
        refiner: Arc<dyn SkillRefiner>,
        /// "Hold new skills for review" master switch. When `true`, a freshly
        /// learned skill is installed but NOT auto-activated and marked pending
        /// (awaiting the user's approval) instead of going live immediately.
        /// Only affects brand-new skills — evolutions of already-approved
        /// machine-owned skills still apply, so the flywheel keeps turning.
        require_review: Arc<AtomicBool>,
    }

    impl PortSkillWriter {
        pub(crate) fn new(
            port: Arc<RebornLocalSkillManagementPort>,
            refiner: Arc<dyn SkillRefiner>,
            require_review: Arc<AtomicBool>,
        ) -> Self {
            Self {
                port,
                refiner,
                require_review,
            }
        }
    }

    #[async_trait]
    impl SkillWriter for PortSkillWriter {
        async fn install_or_update(
            &self,
            scope: ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<SkillWriteOutcome, String> {
            // Self-improvement loop: when this task has been learned before
            // (the same skill name, or a renamed sibling covering the same
            // ground), evolve the existing skill in place instead of overwriting
            // it with a fresh distillation or accreting a near-duplicate. The
            // model folds the new evidence (more gotchas, clearer steps, a bumped
            // version) into the existing skill. `update_skill` requires the
            // document's frontmatter `name` to equal the target, so the merged
            // content is retargeted.
            if let Some(existing_name) = self.find_merge_target(&scope, name, content).await {
                // Gate (Path A): only auto-overwrite a skill the machine itself
                // learned, that no human has edited since, and that is a single
                // file. Otherwise the human owns it — stash the candidate as a
                // proposal and leave the live SKILL.md untouched.
                if !self.is_auto_overwritable(&scope, &existing_name).await {
                    let proposed = self.stash_proposal(&scope, &existing_name, content).await;
                    tracing::debug!(
                        distilled_name = %name,
                        target = %existing_name,
                        proposed,
                        "skill-learning: merge target is human-owned or a bundle; not overwritten"
                    );
                    return Ok(if proposed {
                        SkillWriteOutcome::Proposed(existing_name)
                    } else {
                        SkillWriteOutcome::Skipped
                    });
                }
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
                        return Ok(SkillWriteOutcome::Skipped);
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
                return self.apply_evolution(&scope, &existing_name, &merged).await;
            }
            // Brand-new skill (no merge target). Under "hold for review", stage
            // it (saved, not auto-activated, marked pending) instead of going
            // live. Evolutions of already-approved skills above are unaffected.
            if self.require_review.load(Ordering::Relaxed) {
                return self.install_pending(&scope, name, content).await;
            }
            let stamped = mark_learned(content);
            match self
                .port
                .install_for_scope(scope.clone(), Some(name), &stamped)
                .await
            {
                Ok(result) => {
                    // New learned skill — record the machine baseline (over the
                    // stamped content actually written) so a later run can tell
                    // whether a human has since edited it.
                    self.record_baseline(&scope, &result.name, &stamped, false)
                        .await;
                    Ok(SkillWriteOutcome::Installed(result.name))
                }
                // install is create-only; only a name CONFLICT means a same-named
                // skill exists that find_merge_target did not match (e.g. a parse/
                // listing hiccup). Gate (Path B): overwrite only a machine-owned,
                // untouched, single-file skill; otherwise stash a proposal.
                Err(RebornLocalSkillManagementError::Skill(error))
                    if error.kind() == SkillManagementErrorKind::Conflict =>
                {
                    if self.is_auto_overwritable(&scope, name).await {
                        self.apply_evolution(&scope, name, content).await
                    } else if self.stash_proposal(&scope, name, content).await {
                        Ok(SkillWriteOutcome::Proposed(name.to_string()))
                    } else {
                        Ok(SkillWriteOutcome::Skipped)
                    }
                }
                // Any other failure class (filesystem-denied, validation,
                // resource) is NOT a name conflict, so falling through to an
                // overwrite would clobber a live skill on a transient error.
                // Fail loud instead.
                Err(error) => {
                    tracing::debug!(
                        skill_name = %name,
                        %error,
                        "skill-learning: install failed (non-conflict); not overwriting"
                    );
                    Err(error.to_string())
                }
            }
        }
    }

    impl PortSkillWriter {
        /// Find the existing learned skill the freshly distilled `content` should
        /// evolve, returning its stored name so the caller can refine in place.
        /// Two cases, both routed through refinement (not a plain overwrite):
        /// an exact-name re-learn of a known skill (the common case — the
        /// distiller derives the name from the task, so it often repeats), or a
        /// renamed sibling that's similar enough. Best-effort: a parse/listing
        /// failure returns `None` (fall through to a fresh install).
        async fn find_merge_target(
            &self,
            scope: &ResourceScope,
            name: &str,
            content: &str,
        ) -> Option<String> {
            let parsed = parse_skill_md(content).ok()?;
            let existing = self.port.list_for_scope(scope.clone()).await.ok()?;
            // Exact-name re-learn of an existing learned skill → refine it.
            if existing.iter().any(|summary| {
                matches!(summary.source, ManagedSkillSource::User) && summary.name == name
            }) {
                return Some(name.to_string());
            }
            // A renamed sibling covering the same ground.
            let new_tokens = skill_token_set(
                &parsed.manifest.name,
                &parsed.manifest.activation.keywords,
                &parsed.manifest.activation.tags,
            );
            if new_tokens.len() < MIN_DEDUP_TOKENS {
                return None;
            }
            select_duplicate_skill(name, &new_tokens, &existing)
        }

        /// True iff `name` is a skill the learning sink wrote (a provenance sidecar
        /// exists — the sink is its only writer), that no human has edited since
        /// (live content still matches the recorded baseline), and that is not a
        /// multi-file bundle. Any uncertainty — missing provenance, read error,
        /// unparseable live content — resolves to `false` (fail safe toward "the
        /// human owns it, do not overwrite"). The single gate both
        /// `install_or_update` write paths consult. The security authority is the
        /// host-private content hash (`matches_live_content`), never the skill's
        /// own frontmatter `origin`, so a forged origin cannot invite an overwrite.
        async fn is_auto_overwritable(&self, scope: &ResourceScope, name: &str) -> bool {
            let Ok(Some(provenance)) = self
                .port
                .read_provenance_for_scope(scope.clone(), name)
                .await
            else {
                return false;
            };
            if self
                .port
                .is_bundle_for_scope(scope.clone(), name)
                .await
                .unwrap_or(true)
            {
                return false;
            }
            let Ok(live) = self.port.read_content_for_scope(scope.clone(), name).await else {
                return false;
            };
            provenance.matches_live_content(&live.content)
        }

        /// Record the machine baseline (origin + body hash + activation snapshot)
        /// for a skill the learning sink just wrote, replacing any stale proposal.
        /// `pending_review` carries the hold state forward — a held skill that is
        /// re-learned must stay held. Best-effort: a sidecar failure must not fail
        /// the already-successful skill write; on failure the prior sidecar (still
        /// pending if it was) remains, which fails safe.
        async fn record_baseline(
            &self,
            scope: &ResourceScope,
            name: &str,
            written: &str,
            pending_review: bool,
        ) {
            match LearnedSkillProvenance::for_machine_content(written) {
                Ok(mut provenance) => {
                    provenance.pending_review = pending_review;
                    if let Err(error) = self
                        .port
                        .write_provenance_for_scope(scope.clone(), name, &provenance)
                        .await
                    {
                        tracing::debug!(skill = %name, %error, "skill-learning: failed to record provenance baseline");
                    }
                }
                Err(error) => {
                    tracing::debug!(skill = %name, ?error, "skill-learning: could not compute provenance baseline");
                }
            }
        }

        /// Apply evolved `content` to an existing auto-overwritable (machine-owned,
        /// untouched, single-file) skill. If that skill is currently held for
        /// review, the evolution must NOT un-hold or activate it: stage it
        /// inactive, keep it pending, and return `Pending` so the sink stays
        /// silent. Otherwise evolve it live and return `Evolved`. Both
        /// `install_or_update` overwrite paths funnel through here, so the
        /// held-skill guard cannot be bypassed by one path while the other forgets
        /// it.
        async fn apply_evolution(
            &self,
            scope: &ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<SkillWriteOutcome, String> {
            // Re-validate the overwrite authority HERE, right before the write.
            // `is_auto_overwritable` was checked in `install_or_update` BEFORE the
            // refiner's (slow) LLM merge, so a human could have edited the skill
            // during that window (TOCTOU). Read provenance + live content once and
            // require the live skill still match the machine baseline; a missing
            // sidecar or a mismatch means a human now owns it — stash the
            // candidate as a proposal instead of clobbering the edit. Read
            // failures propagate (fail closed, no overwrite).
            let provenance = self
                .port
                .read_provenance_for_scope(scope.clone(), name)
                .await
                .map_err(|error| error.to_string())?;
            let live = self
                .port
                .read_content_for_scope(scope.clone(), name)
                .await
                .map_err(|error| error.to_string())?;
            let Some(provenance) =
                provenance.filter(|prov| prov.matches_live_content(&live.content))
            else {
                let proposed = self.stash_proposal(scope, name, content).await;
                tracing::debug!(
                    skill = %name,
                    proposed,
                    "skill-learning: live skill diverged during refine; not overwriting"
                );
                return Ok(if proposed {
                    SkillWriteOutcome::Proposed(name.to_string())
                } else {
                    SkillWriteOutcome::Skipped
                });
            };
            if provenance.pending_review {
                // Re-learning a held skill before the user has reviewed it must
                // keep the hold: stage inactive, stay pending, emit no
                // "auto-applies" notice. (Holds even if `require_review` was since
                // switched off — an already-held skill is still unreviewed.)
                let staged = mark_learned(&set_skill_auto_activate(content, false));
                self.port
                    .update_for_scope(scope.clone(), name, &staged)
                    .await
                    .map_err(|error| error.to_string())?;
                self.record_baseline(scope, name, &staged, true).await;
                Ok(SkillWriteOutcome::Pending(name.to_string()))
            } else {
                // Preserve the live skill's activation across evolution: an
                // evolution improves CONTENT, it must NOT silently flip a skill the
                // user turned off back on. A parse failure on the (baseline-
                // matching) live content defaults to OFF rather than activating.
                let keep_active = parse_skill_md(&live.content)
                    .map(|parsed| parsed.manifest.auto_activate)
                    .unwrap_or(false);
                let evolved = mark_learned(&set_skill_auto_activate(content, keep_active));
                self.port
                    .update_for_scope(scope.clone(), name, &evolved)
                    .await
                    .map_err(|error| error.to_string())?;
                self.record_baseline(scope, name, &evolved, false).await;
                Ok(SkillWriteOutcome::Evolved {
                    name: name.to_string(),
                    active: keep_active,
                })
            }
        }

        /// Stash a distilled candidate as a pending proposal for human review,
        /// WITHOUT touching the live `SKILL.md`. Returns `true` iff a proposal was
        /// actually stashed. Only meaningful for a machine-learned, single-file
        /// skill a human has edited; for bundles and purely human-built skills it
        /// is a no-op and returns `false` (we neither overwrite nor pester).
        /// Best-effort.
        async fn stash_proposal(&self, scope: &ResourceScope, name: &str, candidate: &str) -> bool {
            let Some(mut provenance) = self
                .port
                .read_provenance_for_scope(scope.clone(), name)
                .await
                .ok()
                .flatten()
            else {
                return false;
            };
            if self
                .port
                .is_bundle_for_scope(scope.clone(), name)
                .await
                .unwrap_or(true)
            {
                return false;
            }
            provenance.proposed_content = Some(candidate.to_string());
            match self
                .port
                .write_provenance_for_scope(scope.clone(), name, &provenance)
                .await
            {
                Ok(()) => true,
                Err(error) => {
                    tracing::debug!(skill = %name, %error, "skill-learning: failed to stash proposal");
                    false
                }
            }
        }

        /// Install a freshly-learned skill under "hold for review": saved, but
        /// auto-activation turned off and a `pending_review` provenance marker
        /// set, so it does not go live until the user approves it. An install
        /// conflict (the rare best-effort `None` from `find_merge_target` with
        /// the name nonetheless taken) is returned as an error rather than
        /// overwriting a possibly human-owned skill.
        async fn install_pending(
            &self,
            scope: &ResourceScope,
            name: &str,
            content: &str,
        ) -> Result<SkillWriteOutcome, String> {
            // A pending skill must not auto-fire before it has been reviewed.
            let staged = mark_learned(&set_skill_auto_activate(content, false));
            let result = self
                .port
                .install_for_scope(scope.clone(), Some(name), &staged)
                .await
                .map_err(|error| error.to_string())?;
            // The pending_review marker IS part of the pending install: without
            // it, review/list/approve can't find the held skill, so the UI would
            // announce a pending skill the approval API can't see. Make the
            // sidecar write mandatory — propagate its failure rather than leave a
            // saved-but-unmarked skill.
            let mut provenance =
                LearnedSkillProvenance::for_machine_content(&staged).map_err(|e| e.to_string())?;
            provenance.pending_review = true;
            self.port
                .write_provenance_for_scope(scope.clone(), &result.name, &provenance)
                .await
                .map_err(|error| error.to_string())?;
            Ok(SkillWriteOutcome::Pending(result.name))
        }
    }

    /// Pick the existing learned skill most similar to a candidate token set,
    /// if any clears [`SKILL_DEDUP_SIMILARITY_THRESHOLD`]. Only `User`-source
    /// skills are merge targets (never system or registry-installed skills);
    /// the same-named skill is skipped here because an exact-name re-learn is
    /// resolved earlier in [`PortSkillWriter::find_merge_target`]. Pure so it is
    /// unit-testable without a filesystem-backed skill port.
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
        extraction_tasks: Arc<SkillLearningExtractionTasks>,
        /// "Self-learning" master switch (shared with the WebUI skills facade).
        /// When `false`, `publish` returns early before reading the transcript or
        /// spawning an extraction job, so a completed run distills nothing.
        learning_enabled: Arc<AtomicBool>,
    }

    impl SkillLearningTurnEventSink {
        pub(crate) fn new(
            thread_service: Arc<dyn SessionThreadService>,
            inference: Arc<dyn SkillInferencePort>,
            skill_writer: Arc<dyn SkillWriter>,
            notifier: Arc<dyn SkillLearnedNotifier>,
            extraction_tasks: Arc<SkillLearningExtractionTasks>,
            learning_enabled: Arc<AtomicBool>,
        ) -> Self {
            Self {
                thread_service,
                inference,
                skill_writer,
                notifier,
                extraction_tasks,
                learning_enabled,
            }
        }
    }

    /// Runtime-owned handle set for post-turn extraction jobs. Jobs remain
    /// fire-and-forget from the lifecycle publisher's perspective, but the
    /// runtime keeps their handles so shutdown can stop model calls and skill
    /// writes before dropping the services they depend on.
    #[derive(Default)]
    pub(crate) struct SkillLearningExtractionTasks {
        handles: Mutex<Vec<JoinHandle<()>>>,
    }

    impl SkillLearningExtractionTasks {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        fn spawn(&self, job: ExtractionJob) {
            let handle = tokio::spawn(job.run());
            let mut handles = match self.handles.lock() {
                Ok(handles) => handles,
                Err(poisoned) => poisoned.into_inner(),
            };
            handles.retain(|handle| !handle.is_finished());
            handles.push(handle);
        }

        pub(crate) async fn shutdown(&self) {
            let handles = {
                let mut handles = match self.handles.lock() {
                    Ok(handles) => handles,
                    Err(poisoned) => poisoned.into_inner(),
                };
                std::mem::take(&mut *handles)
            };
            for handle in &handles {
                handle.abort();
            }
            for handle in handles {
                if let Err(error) = handle.await {
                    if error.is_panic() {
                        tracing::error!(
                            %error,
                            "skill-learning extraction task panicked during shutdown"
                        );
                    } else {
                        tracing::debug!(
                            %error,
                            "skill-learning extraction task cancelled during shutdown"
                        );
                    }
                }
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
            // "Self-learning" master switch: when off, distill nothing — return
            // before reading the transcript or spawning an extraction job, so a
            // Settings toggle takes effect on the very next completed run.
            if !self.learning_enabled.load(Ordering::Relaxed) {
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
            self.extraction_tasks.spawn(job);
            Ok(())
        }
    }

    /// The post-run extraction work, lifted out of [`SkillLearningTurnEventSink::publish`]
    /// so it is a single owned future the sink can supervise AND a test can
    /// drive to completion with `.await` (the lifecycle publish path does not
    /// await it, so the durable announce below is otherwise untestable through
    /// its caller).
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

            // Eligibility counts tool actions from THIS run only, not the whole
            // thread window: `window` is the recent thread slice with no run
            // filter, so a trivial follow-up turn after a tool-heavy task would
            // otherwise re-pass the gate on the previous run's stale tool results
            // and re-distill it (wasted inference + a stale-transcript refine that
            // can regress an evolved skill). `window` is still used below as the
            // multi-turn distillation CONTEXT (intentional). The per-run COUNT
            // comes from the history projection, which keeps message `kind` +
            // `turn_run_id` and only nulls the tool metadata the transcript needs
            // (hence the separate window read above). The producer writes
            // `turn_run_id = run_id.to_string()`, matching `self.run_id`.
            let run_id_str = self.run_id.to_string();
            let history = match self
                .thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: self.scope.clone(),
                    thread_id: self.thread_id.clone(),
                })
                .await
            {
                Ok(history) => history,
                Err(error) => {
                    tracing::debug!(%error, run_id = ?self.run_id, "skill-learning: could not load history for run-scoped eligibility");
                    return;
                }
            };
            let tool_actions = history
                .messages
                .iter()
                .filter(|message| {
                    matches!(message.kind, MessageKind::ToolResultReference)
                        && message.turn_run_id.as_deref() == Some(run_id_str.as_str())
                })
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
                    let outcome = persist_learned_skill(
                        self.skill_writer.as_ref(),
                        &self.write_tenant,
                        &self.write_owner,
                        &skill,
                        self.run_id,
                    )
                    .await;
                    // Word the user notice honestly per outcome: a fresh install
                    // is active; an evolution preserves the skill's on/off (so a
                    // skill the user turned off is NOT announced as active); a held
                    // skill (require_review default on) is "saved but off, pending
                    // review". Proposed/Skipped left the live skill untouched, so
                    // they stay silent. The durable note and the live bubble carry
                    // the same wording so they agree.
                    let user_note: Option<(String, String)> = match outcome {
                        Some(SkillWriteOutcome::Installed(name)) => {
                            let note = format!(
                                "🎓 I learned a new skill from this task: **{name}**. \
                                 It's active — I'll use it on similar requests when auto-activation is on; manage it under Settings → Skills."
                            );
                            Some((name, note))
                        }
                        Some(SkillWriteOutcome::Evolved { name, active: true }) => {
                            let note = format!(
                                "🎓 I improved the **{name}** skill from this task. \
                                 It's active — I'll use it on similar requests when auto-activation is on; manage it under Settings → Skills."
                            );
                            Some((name, note))
                        }
                        Some(SkillWriteOutcome::Evolved {
                            name,
                            active: false,
                        }) => {
                            let note = format!(
                                "🎓 I improved the **{name}** skill from this task. \
                                 It stays off (you turned it off) — turn it on under Settings → Skills."
                            );
                            Some((name, note))
                        }
                        Some(SkillWriteOutcome::Pending(name)) => {
                            let note = format!(
                                "🎓 I learned a new skill from this task: **{name}**. \
                                 It's saved but off, pending your review — turn it on under Settings → Skills."
                            );
                            Some((name, note))
                        }
                        Some(SkillWriteOutcome::Proposed(name)) => {
                            tracing::debug!(
                                run_id = ?self.run_id,
                                skill = %name,
                                "skill-learning: stashed a proposed update to a human-edited skill"
                            );
                            None
                        }
                        Some(SkillWriteOutcome::Skipped) | None => None,
                    };
                    if let Some((name, note)) = user_note {
                        // Durable feedback first: a thread message survives a page
                        // reload and renders from `get_timeline`. The live bubble
                        // (notify) is the ephemeral best-effort fast path; both
                        // carry the same wording.
                        announce_learned_skill(
                            self.thread_service.as_ref(),
                            &self.scope,
                            &self.thread_id,
                            self.run_id,
                            &note,
                        )
                        .await;
                        self.notifier.notify(
                            &self.write_owner,
                            &self.event_scope,
                            self.run_id,
                            &name,
                            &note,
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
        note: &str,
    ) {
        let note = note.to_string();
        let draft = match thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                // A DISTINCT turn-run id, NOT the run's own id: the durable
                // store dedups assistant drafts by `turn_run_id` and returns the
                // existing one, so reusing `run_id` hands back the run's already
                // finalized reply and the finalize below fails `MessageNotDraft`.
                turn_run_id: format!("skill-learned:{run_id}"),
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
    /// run's (tenant, owner) scope. Returns the [`SkillWriteOutcome`] on success
    /// so the caller can word the user notice honestly per state (installed /
    /// pending / evolved-active-or-off / proposed / skipped).
    /// Best-effort: every failure exit is `debug!`-only.
    async fn persist_learned_skill(
        writer: &dyn SkillWriter,
        tenant: &TenantId,
        owner: &UserId,
        skill: &DistilledSkill,
        run_id: TurnRunId,
    ) -> Option<SkillWriteOutcome> {
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
            Ok(outcome) => {
                tracing::debug!(
                    run_id = ?run_id,
                    skill = %skill.name,
                    "skill-learning: learned-skill write completed"
                );
                Some(outcome)
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

    /// Adapts the run's [`LlmProvider`] to the logic crate's
    /// [`SkillInferencePort`], so distillation runs against the user's CURRENT
    /// model on whatever backend they configured — the same model the run used.
    /// There is no separate learning model and no per-request model override:
    /// matching Hermes, the model that does the work also writes the skill.
    pub(crate) struct SkillLearningInferenceAdapter {
        provider: Arc<dyn LlmProvider>,
    }

    impl SkillLearningInferenceAdapter {
        pub(crate) fn new(provider: Arc<dyn LlmProvider>) -> Self {
            Self { provider }
        }
    }

    #[async_trait]
    impl SkillInferencePort for SkillLearningInferenceAdapter {
        async fn infer(&self, system: &str, user: &str) -> Result<String, SkillInferenceError> {
            let request =
                CompletionRequest::new(vec![ChatMessage::system(system), ChatMessage::user(user)])
                    // No temperature override: reasoning models (e.g. gpt-5.x)
                    // reject any non-default temperature with HTTP 400.
                    // No model override either: distillation uses the provider's
                    // configured model — the run's current model.
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
            AppendToolResultReferenceRequest, EnsureThreadRequest, InMemorySessionThreadService,
            MessageStatus, ThreadHistoryRequest, ToolResultSafeSummary,
        };
        use ironclaw_turns::{EventCursor, TurnStatus};
        use std::sync::atomic::{AtomicUsize, Ordering};

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
            ) -> Result<SkillWriteOutcome, String> {
                Ok(SkillWriteOutcome::Installed("stub".to_string()))
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
                Arc::new(SkillLearningExtractionTasks::new()),
                Arc::new(AtomicBool::new(true)),
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

        // Self-learning master switch: with it OFF, even an eligible completed run
        // must not be distilled — the sink returns before spawning extraction.
        #[tokio::test]
        async fn self_learning_off_skips_extraction_for_an_eligible_run() {
            let service: Arc<dyn SessionThreadService> =
                Arc::new(InMemorySessionThreadService::default());
            let scope = ThreadScope {
                tenant_id: TenantId::new("learn-off-tenant").expect("tenant"),
                agent_id: AgentId::new("learn-off-agent").expect("agent"),
                project_id: None,
                owner_user_id: Some(UserId::new("learn-off-user").expect("user")),
                mission_id: None,
            };
            let thread_id = ThreadId::new("learn-off-thread").expect("thread");
            let run_id = TurnRunId::new();
            service
                .ensure_thread(EnsureThreadRequest {
                    scope: scope.clone(),
                    thread_id: Some(thread_id.clone()),
                    created_by_actor_id: "learn-off-user".to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .expect("ensure thread");
            // Seed an ELIGIBLE run: >= MIN_TOOL_ACTIONS tool results for the run
            // plus enough transcript messages.
            for result_ref in ["r1", "r2", "r3", "r4"] {
                service
                    .append_tool_result_reference(AppendToolResultReferenceRequest {
                        scope: scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: run_id.to_string(),
                        result_ref: format!("result:{result_ref}"),
                        safe_summary: ToolResultSafeSummary::new("tool ran").expect("summary"),
                        provider_call: None,
                        model_observation: None,
                    })
                    .await
                    .expect("seed tool result");
            }
            let filler = service
                .append_assistant_draft(AppendAssistantDraftRequest {
                    scope: scope.clone(),
                    thread_id: thread_id.clone(),
                    turn_run_id: run_id.to_string(),
                    content: MessageContent::text("done"),
                })
                .await
                .expect("seed filler");
            service
                .finalize_assistant_message(
                    &scope,
                    &thread_id,
                    filler.message_id,
                    MessageContent::text("done"),
                )
                .await
                .expect("finalize filler");

            // Control: the run IS eligible — driving extraction directly reaches
            // inference (proves the gate below is what stops it, not ineligibility).
            let control_calls = Arc::new(AtomicUsize::new(0));
            ExtractionJob {
                thread_service: Arc::clone(&service),
                inference: Arc::new(RecordingInference {
                    calls: Arc::clone(&control_calls),
                }),
                skill_writer: Arc::new(StubWriter),
                notifier: Arc::new(StubNotifier),
                scope: scope.clone(),
                thread_id: thread_id.clone(),
                run_id,
                write_tenant: scope.tenant_id.clone(),
                write_owner: scope.owner_user_id.clone().expect("owner"),
                event_scope: TurnScope::new_with_owner(
                    scope.tenant_id.clone(),
                    Some(scope.agent_id.clone()),
                    None,
                    thread_id.clone(),
                    scope.owner_user_id.clone(),
                ),
            }
            .run()
            .await;
            assert!(
                control_calls.load(Ordering::Relaxed) >= 1,
                "the seeded run is eligible — extraction reaches inference"
            );

            // Gate: with self-learning OFF, publishing the SAME run does not even
            // spawn extraction, so inference is never called.
            let gated_calls = Arc::new(AtomicUsize::new(0));
            let sink = SkillLearningTurnEventSink::new(
                Arc::clone(&service),
                Arc::new(RecordingInference {
                    calls: Arc::clone(&gated_calls),
                }),
                Arc::new(StubWriter),
                Arc::new(StubNotifier),
                Arc::new(SkillLearningExtractionTasks::new()),
                Arc::new(AtomicBool::new(false)),
            );
            sink.publish(TurnLifecycleEvent {
                cursor: EventCursor::default(),
                scope: TurnScope::new_with_owner(
                    scope.tenant_id.clone(),
                    Some(scope.agent_id.clone()),
                    None,
                    thread_id.clone(),
                    scope.owner_user_id.clone(),
                ),
                occurred_at: None,
                owner_user_id: scope.owner_user_id.clone(),
                run_id,
                status: TurnStatus::Completed,
                kind: TurnEventKind::Completed,
                blocked_gate: None,
                sanitized_reason: None,
            })
            .await
            .expect("publish is a no-op when self-learning is off");
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            assert_eq!(
                gated_calls.load(Ordering::Relaxed),
                0,
                "self-learning off must skip extraction entirely — no distillation"
            );
        }

        // Durable feedback regression: a learned skill must leave a persisted
        // assistant note in the thread so the user sees it from `get_timeline`
        // even when no live SSE/WS stream was connected when distillation
        // finished. (The live bubble is best-effort and ephemeral.)
        //
        // The thread is seeded with the run's OWN finalized assistant reply under
        // the run's `turn_run_id` first, because the durable store dedups
        // assistant drafts by `turn_run_id`: reusing the run id hands back that
        // finalized reply and the announce's finalize fails `MessageNotDraft`.
        // The earlier fresh-thread version of this test missed that divergence
        // (it appended to an empty thread); a live run surfaced it.
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
            let run_id = TurnRunId::new();
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

            // The run's own finalized assistant reply, under `run_id`.
            let reply = service
                .append_assistant_draft(AppendAssistantDraftRequest {
                    scope: scope.clone(),
                    thread_id: thread_id.clone(),
                    turn_run_id: run_id.to_string(),
                    content: MessageContent::text("the run's own reply"),
                })
                .await
                .expect("seed reply draft");
            service
                .finalize_assistant_message(
                    &scope,
                    &thread_id,
                    reply.message_id,
                    MessageContent::text("the run's own reply"),
                )
                .await
                .expect("finalize seeded reply");

            // The caller now words the note per write-outcome and passes it in;
            // announce just persists it. Use a representative install note.
            let note = "🎓 I learned a new skill from this task: \
                 **file-character-count-roundtrip**. It's active — I'll use it on \
                 similar requests when auto-activation is on; manage it under \
                 Settings → Skills.";
            announce_learned_skill(service.as_ref(), &scope, &thread_id, run_id, note).await;

            let history = service
                .list_thread_history(ThreadHistoryRequest { scope, thread_id })
                .await
                .expect("history");
            let note = history
                .messages
                .iter()
                .find(|message| {
                    matches!(message.kind, MessageKind::Assistant)
                        && message.content.as_deref().is_some_and(|content| {
                            content.contains("file-character-count-roundtrip")
                                && content.contains("learned a new skill")
                        })
                })
                .expect("a durable learned-skill note must be persisted");
            // The note is a SEPARATE finalized message, not the run's reply.
            assert_ne!(
                note.message_id, reply.message_id,
                "the note must be its own message, not the run's reply"
            );
            assert!(
                matches!(note.status, MessageStatus::Finalized),
                "the note must finalize (not stay a draft): {:?}",
                note.status
            );
        }

        struct RecordingInference {
            calls: Arc<AtomicUsize>,
        }

        #[async_trait]
        impl SkillInferencePort for RecordingInference {
            async fn infer(
                &self,
                _system: &str,
                _user: &str,
            ) -> Result<String, SkillInferenceError> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                // The return value is irrelevant: this test only asserts WHETHER
                // the eligibility gate let inference run for the given run.
                Ok("SKIP: test stub".to_string())
            }
        }

        // P2a regression: extraction eligibility must count tool actions from the
        // COMPLETED run only, not the whole thread window. The window is the
        // recent thread slice (no run filter), so a trivial follow-up turn after a
        // tool-heavy task would otherwise re-pass the gate on the previous run's
        // stale tool results and re-distill it.
        #[tokio::test]
        async fn eligibility_counts_tool_actions_for_the_completed_run_only() {
            async fn seed_tool_result(
                service: &dyn SessionThreadService,
                scope: &ThreadScope,
                thread_id: &ThreadId,
                run_id: &TurnRunId,
                result_ref: &str,
            ) {
                service
                    .append_tool_result_reference(AppendToolResultReferenceRequest {
                        scope: scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: run_id.to_string(),
                        result_ref: format!("result:{result_ref}"),
                        safe_summary: ToolResultSafeSummary::new("tool ran").expect("summary"),
                        provider_call: None,
                        model_observation: None,
                    })
                    .await
                    .expect("seed tool result");
            }

            async fn seed_filler_message(
                service: &dyn SessionThreadService,
                scope: &ThreadScope,
                thread_id: &ThreadId,
                run_id: &TurnRunId,
            ) {
                let draft = service
                    .append_assistant_draft(AppendAssistantDraftRequest {
                        scope: scope.clone(),
                        thread_id: thread_id.clone(),
                        turn_run_id: run_id.to_string(),
                        content: MessageContent::text("ok"),
                    })
                    .await
                    .expect("seed filler draft");
                service
                    .finalize_assistant_message(
                        scope,
                        thread_id,
                        draft.message_id,
                        MessageContent::text("ok"),
                    )
                    .await
                    .expect("finalize filler");
            }

            fn job(
                service: Arc<dyn SessionThreadService>,
                scope: &ThreadScope,
                thread_id: &ThreadId,
                run_id: TurnRunId,
                inference: Arc<dyn SkillInferencePort>,
            ) -> ExtractionJob {
                ExtractionJob {
                    thread_service: service,
                    inference,
                    skill_writer: Arc::new(StubWriter),
                    notifier: Arc::new(StubNotifier),
                    scope: scope.clone(),
                    thread_id: thread_id.clone(),
                    run_id,
                    write_tenant: scope.tenant_id.clone(),
                    write_owner: scope.owner_user_id.clone().expect("owner"),
                    event_scope: TurnScope::new_with_owner(
                        scope.tenant_id.clone(),
                        Some(scope.agent_id.clone()),
                        None,
                        thread_id.clone(),
                        scope.owner_user_id.clone(),
                    ),
                }
            }

            let service: Arc<dyn SessionThreadService> =
                Arc::new(InMemorySessionThreadService::default());
            let scope = ThreadScope {
                tenant_id: TenantId::new("p2a-tenant").expect("tenant"),
                agent_id: AgentId::new("p2a-agent").expect("agent"),
                project_id: None,
                owner_user_id: Some(UserId::new("p2a-user").expect("user")),
                mission_id: None,
            };
            let thread_id = ThreadId::new("p2a-thread").expect("thread");
            service
                .ensure_thread(EnsureThreadRequest {
                    scope: scope.clone(),
                    thread_id: Some(thread_id.clone()),
                    created_by_actor_id: "p2a-user".to_string(),
                    title: None,
                    metadata_json: None,
                })
                .await
                .expect("ensure thread");

            let prior_run = TurnRunId::new();
            let current_run = TurnRunId::new();

            // A tool-heavy PRIOR run, then a trivial follow-up (the CURRENT run).
            seed_tool_result(service.as_ref(), &scope, &thread_id, &prior_run, "r1").await;
            seed_tool_result(service.as_ref(), &scope, &thread_id, &prior_run, "r2").await;
            seed_filler_message(service.as_ref(), &scope, &thread_id, &current_run).await;

            // The window has enough messages and >= MIN_TOOL_ACTIONS tool results
            // overall, but NONE belong to the current run → no distillation.
            let trivial_calls = Arc::new(AtomicUsize::new(0));
            job(
                Arc::clone(&service),
                &scope,
                &thread_id,
                current_run,
                Arc::new(RecordingInference {
                    calls: Arc::clone(&trivial_calls),
                }),
            )
            .run()
            .await;
            assert_eq!(
                trivial_calls.load(Ordering::Relaxed),
                0,
                "a trivial follow-up turn must not re-distill the previous run's stale tool work"
            );

            // Control: a run that DID its own tool work stays eligible. Seed
            // MIN_TOOL_ACTIONS (4) results so the current run clears the floor.
            seed_tool_result(service.as_ref(), &scope, &thread_id, &current_run, "r3").await;
            seed_tool_result(service.as_ref(), &scope, &thread_id, &current_run, "r4").await;
            seed_tool_result(service.as_ref(), &scope, &thread_id, &current_run, "r5").await;
            seed_tool_result(service.as_ref(), &scope, &thread_id, &current_run, "r6").await;
            let substantive_calls = Arc::new(AtomicUsize::new(0));
            job(
                Arc::clone(&service),
                &scope,
                &thread_id,
                current_run,
                Arc::new(RecordingInference {
                    calls: Arc::clone(&substantive_calls),
                }),
            )
            .run()
            .await;
            assert!(
                substantive_calls.load(Ordering::Relaxed) >= 1,
                "a run with its own tool actions must reach distillation"
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
                auto_activate: true,
                origin: SkillOrigin::Learned,
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
        async fn refiner_keeps_existing_when_model_output_is_unparseable() {
            // A chatty/garbage response is exactly when the single-run candidate
            // is least trustworthy, so the accumulated evolved skill must be kept
            // — NOT overwritten with the raw candidate (the silent-data-loss path
            // ilblackdragon flagged).
            assert!(matches!(
                refiner("sure, here is the merged skill")
                    .merge(EXISTING_SKILL, EXISTING_SKILL, "file-count")
                    .await,
                MergeAction::KeepExisting
            ));
        }
    }
}
