//! The reply projection: the one semantic reducer from run facts to the
//! channel-neutral [`ReplyDocument`] (design doc §4). It lives inside the
//! projection owner because it *is* the projection's live half — the same
//! module family that projects durable turn events and runtime activity for
//! the WebUI projects the in-flight reply for every reply-capable channel.
//!
//! One rebuildable [`ReplyDocument`] per run. While the run is live it is
//! composed from the loop's host milestones — already model-visible
//! sanitized, re-sanitized and bounded here — so a progressive surface can
//! show thinking, the growing answer, tool activity, status and gates. The
//! **terminal** revision is never taken from that ephemeral stream: it is
//! built from durable history (the finalized transcript row, its attachments,
//! and the run's committed status) through [`ReplyProjection::apply_terminal_facts`],
//! which is also how a fresh process rebuilds a reply it never watched.
//!
//! What can never enter the document: raw reasoning (only bounded, redacted
//! segments), tool arguments or results (only display previews), secrets
//! (every text passes `sanitize_model_visible_text`), unbounded text (every
//! field is a bounded newtype). Audience policy is applied on the way out by
//! [`disclose_for_audience`], before publication hands a copy to any sink.
//!
//! This module holds no publication state: which targets have seen which
//! revision lives on the outbound attempt aggregate, driven by the delivery
//! coordinator's publication lane. Runs are cache entries here — the durable
//! facts stay where they are, and [`ReplyProjection::evict`] only frees
//! memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use ironclaw_extension_contracts::reply::{
    REPLY_ANSWER_MAX_BYTES, REPLY_DISPLAY_PREVIEW_MAX_BYTES, REPLY_DISPLAY_TEXT_MAX_BYTES,
    REPLY_MAX_ATTACHMENTS, REPLY_REASONING_SEGMENT_MAX_BYTES, ReplyActivityProvenance,
    ReplyActivityState, ReplyAnswerText, ReplyAttachmentRef, ReplyAttention, ReplyAttentionKind,
    ReplyAudience, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument, ReplyItemId,
    ReplyReasoningText, ReplyReconcilePoint,
};
use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope, TurnStatus};
use ironclaw_loop_contracts::{
    AgentLoopHostError, LoopDriverNoteKind, LoopGateKind, LoopHostMilestone, LoopHostMilestoneKind,
    LoopHostMilestoneSink, sanitize_model_visible_text,
};
use ironclaw_threads::AttachmentRef;

use crate::run_delivery::prompts::RUN_FAILED_MESSAGE;

use super::display_preview::{CapabilityDisplayPreviewSource, CapabilityDisplayPreviewStore};

#[cfg(test)]
mod tests;

/// Defensive ceiling on live-tracked runs. The runtime's concurrent-run limit
/// is far lower; this only keeps a missing terminal milestone from turning
/// per-run bookkeeping into unbounded process memory.
const DEFAULT_MAX_TRACKED_RUNS: usize = 4_096;

/// What the projection tells its observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyProjectionEvent {
    /// The document moved to a new revision; the point says how urgently a
    /// progressive sink should hear about it.
    Revised(ReplyReconcilePoint),
    /// The loop reported completion or failure. The terminal revision is not
    /// composed from that milestone — the owner of the durable facts must now
    /// fetch them and call [`ReplyProjection::apply_terminal_facts`].
    TerminalPending,
}

/// Something that reacts to the projection moving. Crate-internal on
/// purpose: the delivery coordinator's publication lane is the one
/// production consumer, and nothing outside the crate subscribes.
pub(crate) trait ReplyProjectionObserver: Send + Sync {
    fn reply_projection_event(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        event: ReplyProjectionEvent,
    );
}

/// One run's reply as the projection currently holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplySnapshot {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    /// The run's owning actor, when a milestone or the terminal facts named
    /// one. Reply publication keys the publication target on it.
    pub actor: Option<TurnActor>,
    /// Monotonic per run; bumped once per observed change set.
    pub revision: u64,
    pub document: ReplyDocument,
    /// The loop finished but the durable terminal facts were not applied yet.
    pub terminal_pending: bool,
}

/// The durable facts the terminal revision is built from: the run's committed
/// status plus the finalized transcript row (text and attachment references)
/// when it wrote one. Read from the thread service and the turn kernel by
/// reply publication — never from the milestone stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalReplyFacts {
    pub actor: Option<TurnActor>,
    pub status: TurnStatus,
    /// The run completed with `TurnExecutionOutcome::NothingToReport`: a
    /// terminal revision with an empty answer is still published so every
    /// target closes, but no answer text is invented for it.
    pub nothing_to_report: bool,
    pub answer: Option<String>,
    pub attachments: Vec<AttachmentRef>,
    /// A host-authored, already sanitized failure summary when the kernel
    /// recorded one. Re-sanitized and bounded here regardless.
    pub failure_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunKey {
    scope: TurnScope,
    run_id: TurnRunId,
}

/// What one fold of milestones did to the document — the classification the
/// revision point is derived from.
#[derive(Default)]
struct Folded {
    changed: bool,
    /// An input-required transition or the canonical answer landed: the
    /// revision must be reconciled on its own, never coalesced across.
    control_critical: bool,
    /// The document reached its terminal outcome in this fold.
    terminal: bool,
}

impl Folded {
    fn note(&mut self, changed: bool) {
        self.changed |= changed;
    }

    fn note_control_critical(&mut self, changed: bool) {
        self.changed |= changed;
        self.control_critical |= changed;
    }
}

struct RunReply {
    actor: Option<TurnActor>,
    document: ReplyDocument,
    revision: u64,
    terminal_pending: bool,
}

impl RunReply {
    fn new(actor: Option<TurnActor>) -> Self {
        Self {
            actor,
            document: ReplyDocument::default(),
            revision: 0,
            terminal_pending: false,
        }
    }

    /// The answer is the current model call's cumulative text (that is what
    /// `ModelTextDelta` carries): an append when the new text extends what
    /// is shown, a rewrite otherwise (a call that restarted its text, or a
    /// rewrite under the stream).
    fn fold_answer(&mut self, call_text: &str, folded: &mut Folded) {
        let shown = self.document.answer.text.as_str();
        if call_text == shown {
            return;
        }
        if let Some(suffix) = call_text.strip_prefix(shown)
            && !self.document.answer.truncated
        {
            folded.note(self.document.append_answer(suffix));
        } else {
            folded.note(self.document.rewrite_answer(call_text));
        }
    }

    /// The current call failed: its fragment is neither answer nor narration.
    fn discard_answer(&mut self, folded: &mut Folded) {
        if !self.document.answer.text.as_str().is_empty() {
            folded.note(self.document.rewrite_answer(""));
        }
    }

    /// Anything that is not more reasoning ends the open reasoning segment:
    /// its final text is the segment as it stands.
    fn close_open_reasoning(&mut self, folded: &mut Folded) {
        if self.document.reasoning_open
            && let Some(open) = self.document.reasoning.last().cloned()
        {
            folded.note(self.document.close_reasoning(open));
        }
    }

    fn snapshot(&self, key: &RunKey) -> ReplySnapshot {
        ReplySnapshot {
            scope: key.scope.clone(),
            run_id: key.run_id,
            actor: self.actor.clone(),
            revision: self.revision,
            document: self.document.clone(),
            terminal_pending: self.terminal_pending,
        }
    }

    /// Seal one fold as a revision. Returns the reconcile point the revision
    /// sits on, or `None` when the fold changed nothing.
    fn seal(&mut self, folded: &Folded) -> Option<ReplyReconcilePoint> {
        if !folded.changed {
            return None;
        }
        let opened = self.revision == 0;
        self.revision = self.revision.saturating_add(1);
        Some(if folded.terminal && self.document.is_terminal() {
            ReplyReconcilePoint::Terminal
        } else if opened {
            ReplyReconcilePoint::Opened
        } else if folded.control_critical {
            ReplyReconcilePoint::ControlCritical
        } else {
            ReplyReconcilePoint::Progress
        })
    }
}

/// The projection: the live documents plus the observers to wake.
pub struct ReplyProjection {
    runs: Mutex<HashMap<RunKey, RunReply>>,
    observers: RwLock<Vec<Arc<dyn ReplyProjectionObserver>>>,
    display_previews: RwLock<Option<Arc<CapabilityDisplayPreviewStore>>>,
    max_tracked_runs: usize,
}

impl Default for ReplyProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ReplyProjection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplyProjection")
            .field("max_tracked_runs", &self.max_tracked_runs)
            .finish_non_exhaustive()
    }
}

impl ReplyProjection {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_TRACKED_RUNS)
    }

    pub fn with_capacity(max_tracked_runs: usize) -> Self {
        Self {
            runs: Mutex::new(HashMap::new()),
            observers: RwLock::new(Vec::new()),
            display_previews: RwLock::new(None),
            max_tracked_runs: max_tracked_runs.max(1),
        }
    }

    /// Attach the existing safe capability-preview store once during runtime
    /// assembly. The capability host is assembled after the milestone sink,
    /// so this narrow late bind closes that startup cycle without creating a
    /// second activity or transcript pipeline.
    pub fn bind_display_previews(&self, previews: Arc<CapabilityDisplayPreviewStore>) -> bool {
        let mut bound = self
            .display_previews
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if bound.is_some() {
            return false;
        }
        *bound = Some(previews);
        true
    }

    pub(crate) fn add_observer(&self, observer: Arc<dyn ReplyProjectionObserver>) {
        self.observers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(observer);
    }

    /// Fold one host milestone into the run's document.
    pub fn observe_milestone(&self, milestone: &LoopHostMilestone) {
        let key = RunKey {
            scope: milestone.scope.clone(),
            run_id: milestone.run_id,
        };
        if !milestone_is_projected(&milestone.kind) {
            return;
        }
        let display_previews = self
            .display_previews
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let events = {
            let mut runs = self.lock_runs();
            let run = match runs.get_mut(&key) {
                Some(run) => run,
                None => {
                    if runs.len() >= self.max_tracked_runs {
                        tracing::debug!(
                            target: "ironclaw::reborn::reply_projection",
                            run_id = %milestone.run_id,
                            "reply projection reached its tracked-run capacity; live facets for this run are not projected"
                        );
                        return;
                    }
                    runs.entry(key.clone())
                        .or_insert_with(|| RunReply::new(milestone.actor.clone()))
                }
            };
            if run.actor.is_none() {
                run.actor = milestone.actor.clone();
            }
            let mut folded = Folded::default();
            let terminal_pending = fold_milestone(
                run,
                &milestone.kind,
                display_previews.as_deref(),
                &mut folded,
            );
            let mut events = Vec::with_capacity(2);
            if let Some(point) = run.seal(&folded) {
                events.push(ReplyProjectionEvent::Revised(point));
            }
            if terminal_pending && !run.document.is_terminal() {
                run.terminal_pending = true;
                events.push(ReplyProjectionEvent::TerminalPending);
            }
            events
        };
        for event in events {
            self.notify(&key, event);
        }
    }

    /// Build (or complete) the terminal revision from durable facts. Always
    /// lands — a capacity bound never loses an answer — and is idempotent for
    /// a run whose document is already terminal.
    pub fn apply_terminal_facts(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        facts: TerminalReplyFacts,
    ) -> ReplySnapshot {
        let key = RunKey {
            scope: scope.clone(),
            run_id,
        };
        let (snapshot, event) = {
            let mut runs = self.lock_runs();
            let run = runs
                .entry(key.clone())
                .or_insert_with(|| RunReply::new(facts.actor.clone()));
            if run.actor.is_none() {
                run.actor = facts.actor.clone();
            }
            if run.document.is_terminal() {
                // Idempotent: the terminal fact is durable; a second
                // application (another resume path, a redelivered commit)
                // must not mint a revision the store's fixed terminal
                // revision can never advance to.
                run.terminal_pending = false;
                return run.snapshot(&key);
            }
            let mut folded = Folded::default();
            fold_terminal_facts(run, &facts, &mut folded);
            let event = run.seal(&folded).map(ReplyProjectionEvent::Revised);
            if run.document.is_terminal() {
                run.terminal_pending = false;
            }
            (run.snapshot(&key), event)
        };
        if let Some(event) = event {
            self.notify(&key, event);
        }
        snapshot
    }

    /// Continue a run's revision numbering above `floor`. A publisher that
    /// resumes a run on a fresh process learns from the store which revision
    /// its targets already saw; the terminal revision built here must number
    /// above it, or the resumed publication would look already published.
    /// Creates the run entry when absent; never lowers an existing revision.
    pub fn raise_revision_floor(&self, scope: &TurnScope, run_id: TurnRunId, floor: u64) {
        let key = RunKey {
            scope: scope.clone(),
            run_id,
        };
        let mut runs = self.lock_runs();
        let run = runs.entry(key).or_insert_with(|| RunReply::new(None));
        run.revision = run.revision.max(floor);
    }

    pub fn snapshot(&self, scope: &TurnScope, run_id: TurnRunId) -> Option<ReplySnapshot> {
        let key = RunKey {
            scope: scope.clone(),
            run_id,
        };
        self.lock_runs().get(&key).map(|run| run.snapshot(&key))
    }

    /// Drop the run's in-memory document. Cache eviction only: the durable
    /// facts the terminal revision was built from are untouched.
    pub fn evict(&self, scope: &TurnScope, run_id: TurnRunId) {
        self.lock_runs().remove(&RunKey {
            scope: scope.clone(),
            run_id,
        });
    }

    fn notify(&self, key: &RunKey, event: ReplyProjectionEvent) {
        let observers = self
            .observers
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for observer in observers {
            observer.reply_projection_event(&key.scope, key.run_id, event);
        }
    }

    fn lock_runs(&self) -> std::sync::MutexGuard<'_, HashMap<RunKey, RunReply>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The audience policy applied before a document copy leaves the projection.
/// A shared conversation never sees reasoning, and never sees a connect or
/// authorization link (bearer material a private DM may carry).
pub fn disclose_for_audience(document: &ReplyDocument, audience: ReplyAudience) -> ReplyDocument {
    let mut disclosed = document.clone();
    match audience {
        ReplyAudience::Private => {}
        ReplyAudience::Shared => {
            disclosed.reasoning.clear();
            disclosed.reasoning_open = false;
            // Input/output previews are owner-facing (commands, paths, hosts);
            // a shared room sees the activity row, never its previews.
            for activity in disclosed.activities.iter_mut() {
                activity.detail = None;
                activity.output_preview = None;
            }
            if let Some(attention) = disclosed.attention.as_mut() {
                attention.action_url = None;
            }
        }
    }
    disclosed
}

/// The milestone-sink decorator: records the milestone durably through the
/// inner sink first, then composes it. A refused milestone composes nothing,
/// so the projection never runs ahead of the durable record.
pub struct ReplyProjectionMilestoneSink {
    inner: Arc<dyn LoopHostMilestoneSink>,
    projection: Arc<ReplyProjection>,
}

impl ReplyProjectionMilestoneSink {
    pub fn new(inner: Arc<dyn LoopHostMilestoneSink>, projection: Arc<ReplyProjection>) -> Self {
        Self { inner, projection }
    }
}

#[async_trait]
impl LoopHostMilestoneSink for ReplyProjectionMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.publish_loop_milestone(milestone.clone()).await?;
        self.projection.observe_milestone(&milestone);
        Ok(())
    }
}

// ── Composition ──────────────────────────────────────────────────────────

/// Whether a milestone says anything user-visible. Prompt bundles,
/// checkpoints, hooks, compaction bookkeeping, and batch counters do not.
fn milestone_is_projected(kind: &LoopHostMilestoneKind) -> bool {
    !matches!(
        kind,
        LoopHostMilestoneKind::PromptBundleBuilt { .. }
            | LoopHostMilestoneKind::FailureRecovered { .. }
            | LoopHostMilestoneKind::CapabilityBatchStarted { .. }
            | LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
            | LoopHostMilestoneKind::CheckpointCreated { .. }
            | LoopHostMilestoneKind::CompactionStarted { .. }
            | LoopHostMilestoneKind::CompactionCompleted { .. }
            | LoopHostMilestoneKind::CompactionFailed { .. }
            | LoopHostMilestoneKind::CompactionLeakDetected { .. }
            | LoopHostMilestoneKind::AssistantReplyFinalized { .. }
            | LoopHostMilestoneKind::HookDispatched { .. }
            | LoopHostMilestoneKind::HookDecisionEmitted { .. }
            | LoopHostMilestoneKind::HookFailed { .. }
    )
}

/// Fold one milestone into the run's document via the document's bounded
/// mutators. Returns whether the loop finished (the terminal facts are now
/// worth fetching); everything else is recorded on `folded`.
fn fold_milestone(
    run: &mut RunReply,
    kind: &LoopHostMilestoneKind,
    display_previews: Option<&CapabilityDisplayPreviewStore>,
    folded: &mut Folded,
) -> bool {
    // More reasoning keeps the open segment growing; anything else closes it.
    if !matches!(kind, LoopHostMilestoneKind::ModelReasoningDelta { .. }) {
        run.close_open_reasoning(folded);
    }
    // The loop went on after the last model call: that call's text was
    // narration, not the answer — the document moves it out of the answer
    // (`reset_answer`) before the arm below records what the loop did next,
    // so a capability row lands after the narration that announced it.
    if loop_continued_past_the_model_call(kind) {
        folded.note(run.document.reset_answer());
    }
    match kind {
        LoopHostMilestoneKind::IterationStarted { iteration } => {
            if *iteration <= 1 {
                folded.note(
                    run.document
                        .note_phase(ironclaw_extension_contracts::reply::ReplyPhase::Preparing),
                );
            } else {
                // A later iteration means the loop resumed: whatever it was
                // parked on has been answered.
                folded.note_control_critical(run.document.clear_attention());
            }
        }
        LoopHostMilestoneKind::ModelStarted { .. } => {
            folded.note_control_critical(run.document.clear_attention());
            folded.note(
                run.document
                    .note_phase(ironclaw_extension_contracts::reply::ReplyPhase::Thinking),
            );
        }
        LoopHostMilestoneKind::ModelReasoningDelta { safe_delta } => {
            let text = sanitize_model_visible_text(safe_delta.as_str());
            if let Some(text) = reasoning_text(&text) {
                folded.note(run.document.append_reasoning(&text));
            }
        }
        // Cumulative text of the current model call → append or rewrite
        // relative to what the document already shows.
        LoopHostMilestoneKind::ModelTextDelta { safe_text } => {
            let text = sanitize_model_visible_text(safe_text.as_str());
            let stripped = strip_control(&text);
            if !stripped.is_empty() {
                folded.note(
                    run.document
                        .note_phase(ironclaw_extension_contracts::reply::ReplyPhase::Working),
                );
            }
            run.fold_answer(&stripped, folded);
        }
        // A completed call stays the answer until the loop proves otherwise
        // (see `loop_continued_past_the_model_call`) or the run ends.
        LoopHostMilestoneKind::ModelCompleted { .. } => {}
        // The failed call's fragment is not finished text: drop it so a
        // retried call (a fresh `ModelStarted`) starts from nothing instead
        // of freezing "Wor" into the reply.
        LoopHostMilestoneKind::ModelFailed { .. } => run.discard_answer(folded),
        LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id,
        } => {
            if let Some((id, title)) =
                item_id(&activity_id.to_string()).zip(display_text(capability_id.as_str()))
            {
                folded.note(run.document.activity_started(
                    id,
                    title,
                    activity_input_preview(display_previews, *activity_id),
                ));
            }
        }
        LoopHostMilestoneKind::CapabilityCompleted {
            activity_id,
            capability_id,
            provider,
            runtime,
            output_bytes,
        } => {
            let Some(id) = item_id(&activity_id.to_string()) else {
                return false;
            };
            let input_preview = activity_input_preview(display_previews, *activity_id);
            let output_preview = activity_output_preview(display_previews, *activity_id);
            ensure_activity_title(run, &id, capability_id.as_str(), input_preview, folded);
            folded.note(run.document.activity_finished(
                id,
                ReplyActivityState::Completed,
                output_preview,
                Some(ReplyActivityProvenance {
                    provider: display_text(provider.as_str()),
                    runtime: display_text(runtime.as_str()),
                    output_bytes: Some(*output_bytes),
                }),
            ));
        }
        LoopHostMilestoneKind::CapabilityFailed {
            activity_id,
            capability_id,
            provider,
            runtime,
            reason_kind,
            safe_summary,
        } => {
            let Some(id) = item_id(&activity_id.to_string()) else {
                return false;
            };
            let Some(kind) = display_text(reason_kind.as_str()) else {
                return false;
            };
            ensure_activity_title(
                run,
                &id,
                capability_id.as_str(),
                activity_input_preview(display_previews, *activity_id),
                folded,
            );
            folded.note(
                run.document.activity_finished(
                    id,
                    ReplyActivityState::Failed { kind },
                    safe_summary
                        .as_ref()
                        .and_then(|summary| {
                            ironclaw_event_log::sanitize_error_summary(summary.as_str())
                        })
                        .and_then(|summary| display_preview(&summary)),
                    Some(ReplyActivityProvenance {
                        provider: provider
                            .as_ref()
                            .and_then(|provider| display_text(provider.as_str())),
                        runtime: runtime.and_then(|runtime| display_text(runtime.as_str())),
                        output_bytes: None,
                    }),
                ),
            );
        }
        LoopHostMilestoneKind::DriverNote { kind, safe_summary } => {
            let text = sanitize_model_visible_text(safe_summary.as_str());
            if !text.trim().is_empty()
                && let Some(text) = display_text(&text)
            {
                folded.note(run.document.set_status(text, Some(status_kind(*kind))));
            }
        }
        LoopHostMilestoneKind::GateBlocked { gate_kind, .. } => {
            if let Some(attention) = attention_for_gate(*gate_kind, None) {
                folded.note_control_critical(run.document.require_attention(attention));
            }
        }
        LoopHostMilestoneKind::Blocked { gate_ref, .. } => {
            // The gate ref arrives one milestone after the kind; carry it onto
            // the attention the publisher enriches at publish time. The kind
            // is re-derived from the ref's own prefix when the loop did not
            // announce it separately.
            if let Some(attention) = attention_for_gate_ref(gate_ref.as_str()) {
                folded.note_control_critical(run.document.require_attention(attention));
            }
        }
        LoopHostMilestoneKind::Completed { .. } | LoopHostMilestoneKind::Failed { .. } => {
            folded.note_control_critical(run.document.clear_attention());
            return true;
        }
        LoopHostMilestoneKind::PromptBundleBuilt { .. }
        | LoopHostMilestoneKind::FailureRecovered { .. }
        | LoopHostMilestoneKind::CapabilityBatchStarted { .. }
        | LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
        | LoopHostMilestoneKind::CheckpointCreated { .. }
        | LoopHostMilestoneKind::CompactionStarted { .. }
        | LoopHostMilestoneKind::CompactionCompleted { .. }
        | LoopHostMilestoneKind::CompactionFailed { .. }
        | LoopHostMilestoneKind::CompactionLeakDetected { .. }
        | LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        | LoopHostMilestoneKind::HookDispatched { .. }
        | LoopHostMilestoneKind::HookDecisionEmitted { .. }
        | LoopHostMilestoneKind::HookFailed { .. } => {}
    }
    false
}

/// Milestones that prove the loop went on after a model call — so that call
/// was not the run's final assistant message and its text was narration.
/// A gate counts: it is raised for a tool call the same model call produced.
fn loop_continued_past_the_model_call(kind: &LoopHostMilestoneKind) -> bool {
    matches!(
        kind,
        LoopHostMilestoneKind::ModelStarted { .. }
            | LoopHostMilestoneKind::CapabilityInvoked { .. }
            | LoopHostMilestoneKind::CapabilityCompleted { .. }
            | LoopHostMilestoneKind::CapabilityFailed { .. }
            | LoopHostMilestoneKind::GateBlocked { .. }
            | LoopHostMilestoneKind::Blocked { .. }
    )
}

/// A finish for a row this document never saw start (the invoke milestone was
/// lost, or only the terminal one reached us) still needs its title: the
/// capability id, not the activity id.
fn ensure_activity_title(
    run: &mut RunReply,
    id: &ReplyItemId,
    capability: &str,
    detail: Option<ReplyDisplayPreview>,
    folded: &mut Folded,
) {
    if run
        .document
        .activities
        .iter()
        .any(|row| row.id.as_str() == id.as_str())
    {
        return;
    }
    if let Some(title) = display_text(capability) {
        folded.note(run.document.activity_started(id.clone(), title, detail));
    }
}

fn activity_input_preview(
    previews: Option<&CapabilityDisplayPreviewStore>,
    activity_id: ironclaw_host_api::turn::CapabilityActivityId,
) -> Option<ReplyDisplayPreview> {
    let input = previews?.running_input(InvocationId::from_uuid(activity_id.as_uuid()))?;
    input
        .input_summary
        .or(input.subtitle)
        .and_then(|summary| display_preview(&summary))
}

fn activity_output_preview(
    previews: Option<&CapabilityDisplayPreviewStore>,
    activity_id: ironclaw_host_api::turn::CapabilityActivityId,
) -> Option<ReplyDisplayPreview> {
    let record = previews?.record_for_invocation(InvocationId::from_uuid(activity_id.as_uuid()))?;
    record
        .output_preview
        .or(record.output_summary)
        .and_then(|summary| display_preview(&summary))
}

/// The mutations durable terminal facts mean. A not-yet-terminal status
/// (facts fetched too early) folds nothing.
fn fold_terminal_facts(run: &mut RunReply, facts: &TerminalReplyFacts, folded: &mut Folded) {
    match facts.status {
        TurnStatus::Completed => {
            folded.note_control_critical(run.document.clear_attention());
            let canonical = facts
                .answer
                .as_deref()
                .map(sanitize_model_visible_text)
                .unwrap_or_default();
            // The transcript row finalizes only the run's FINAL assistant
            // message — the same text the progressive answer holds, since
            // every earlier call's text was demoted to thinking when the
            // loop went on. Finalize IN PLACE whenever the shown text ends
            // with the canonical one (a trailing-whitespace difference, or
            // the empty canonical of a run with nothing to report):
            // replacing it with its own tail would break every stream
            // presentation's prefix-extension invariant — a stream sink's
            // terminal reconcile would see a rewrite and present the answer
            // a second time beside the stream. A canonical text the shown
            // text does not end with is a genuine rewrite and replaces it;
            // so does any text once the progressive bound truncated, because
            // the canonical row is the only complete copy.
            let shown = run.document.answer.text.as_str();
            let text = if !run.document.answer.truncated && shown.ends_with(canonical.as_str()) {
                shown.to_string()
            } else {
                canonical
            };
            if let Some(text) = answer_text(&text).or_else(|| answer_text("")) {
                folded.note_control_critical(
                    run.document.finalize_answer(
                        text,
                        facts
                            .attachments
                            .iter()
                            .filter_map(attachment_ref)
                            .take(REPLY_MAX_ATTACHMENTS)
                            .collect(),
                    ),
                );
            }
            folded.note(run.document.complete());
            folded.terminal = true;
        }
        TurnStatus::Failed | TurnStatus::RecoveryRequired => {
            folded.note_control_critical(run.document.clear_attention());
            let summary = facts
                .failure_summary
                .as_deref()
                .map(sanitize_model_visible_text)
                .filter(|summary| !summary.trim().is_empty())
                .unwrap_or_else(|| RUN_FAILED_MESSAGE.to_string());
            if let Some(summary) =
                display_text(&summary).or_else(|| display_text(RUN_FAILED_MESSAGE))
            {
                folded.note(run.document.fail(summary));
            } else {
                // Unreachable in practice (the neutral copy is valid display
                // text); a total function still needs an outcome, and a
                // cancel is the honest one for "ended without a summary".
                folded.note(run.document.cancel());
            }
            folded.terminal = true;
        }
        TurnStatus::Cancelled => {
            folded.note_control_critical(run.document.clear_attention());
            folded.note(run.document.cancel());
            folded.terminal = true;
        }
        TurnStatus::Queued
        | TurnStatus::Running
        | TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool
        | TurnStatus::CancelRequested => {}
    }
}

fn status_kind(kind: LoopDriverNoteKind) -> ironclaw_extension_contracts::reply::ReplyStatusKind {
    use ironclaw_extension_contracts::reply::ReplyStatusKind;
    match kind {
        LoopDriverNoteKind::Planning => ReplyStatusKind::Planning,
        LoopDriverNoteKind::Waiting => ReplyStatusKind::Waiting,
        LoopDriverNoteKind::Retrying => ReplyStatusKind::Retrying,
        LoopDriverNoteKind::Context | LoopDriverNoteKind::EventSubscriptionTerminated => {
            ReplyStatusKind::Context
        }
    }
}

fn attention_for_gate(kind: LoopGateKind, gate_ref: Option<&str>) -> Option<ReplyAttention> {
    let (attention_kind, headline) = match kind {
        LoopGateKind::Approval => (ReplyAttentionKind::Approval, "Approval needed"),
        LoopGateKind::Auth => (ReplyAttentionKind::Auth, "Authentication needed"),
        // `LoopGateKind` is `#[non_exhaustive]` upstream: any gate kind this
        // module does not know is still "the run is parked", reported as a
        // generic wait rather than dropped.
        _ => (ReplyAttentionKind::Resource, "Waiting to continue"),
    };
    Some(ReplyAttention {
        kind: attention_kind,
        headline: display_text(headline)?,
        body: None,
        action_url: None,
        gate_ref: gate_ref.and_then(display_text),
    })
}

fn attention_for_gate_ref(gate_ref: &str) -> Option<ReplyAttention> {
    let kind = if gate_ref.starts_with("gate:auth") || gate_ref.contains(":auth:") {
        LoopGateKind::Auth
    } else if gate_ref.starts_with("gate:resource") || gate_ref.contains(":resource:") {
        LoopGateKind::ResourceWait
    } else {
        LoopGateKind::Approval
    };
    attention_for_gate(kind, Some(gate_ref))
}

fn attachment_ref(attachment: &AttachmentRef) -> Option<ReplyAttachmentRef> {
    Some(ReplyAttachmentRef {
        id: item_id(&attachment.id)?,
        filename: display_text(attachment.filename.as_deref()?)?,
        mime_type: display_text(&attachment.mime_type)?,
        size_bytes: attachment.size_bytes?,
    })
}

// ── Bounded constructors ─────────────────────────────────────────────────
//
// Every text the projection emits is bounded by construction: control
// characters are stripped (line structure kept), the byte bound is cut at a
// character boundary, and the contract constructor has the last word. A
// value that still does not fit the contract (empty after stripping, an id
// outside the identifier grammar) is dropped with a debug log rather than
// coerced into a different identity or a different meaning.

fn char_boundary_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end] // safety: `end` walked back to a char boundary above.
}

fn strip_control(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

pub(crate) fn display_text(value: &str) -> Option<ReplyDisplayText> {
    let stripped = strip_control(value);
    ReplyDisplayText::new(char_boundary_prefix(
        &stripped,
        REPLY_DISPLAY_TEXT_MAX_BYTES,
    ))
    .ok()
}

pub(crate) fn display_preview(value: &str) -> Option<ReplyDisplayPreview> {
    let stripped = strip_control(value);
    ReplyDisplayPreview::new(char_boundary_prefix(
        &stripped,
        REPLY_DISPLAY_PREVIEW_MAX_BYTES,
    ))
    .ok()
}

fn reasoning_text(value: &str) -> Option<ReplyReasoningText> {
    let stripped = strip_control(value);
    ReplyReasoningText::new(char_boundary_prefix(
        &stripped,
        REPLY_REASONING_SEGMENT_MAX_BYTES,
    ))
    .ok()
}

fn answer_text(value: &str) -> Option<ReplyAnswerText> {
    let stripped = strip_control(value);
    ReplyAnswerText::new(char_boundary_prefix(&stripped, REPLY_ANSWER_MAX_BYTES)).ok()
}

fn item_id(value: &str) -> Option<ReplyItemId> {
    match ReplyItemId::new(value) {
        Ok(id) => Some(id),
        Err(error) => {
            tracing::debug!(
                target: "ironclaw::reborn::reply_projection",
                %error,
                "reply item id rejected by the contract; the change is not projected"
            );
            None
        }
    }
}
