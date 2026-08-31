//! The assistant-owned safe reply projection (design doc §4).
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
//! revision lives on the outbound attempt aggregate, driven by reply
//! publication. Runs are cache entries here — the durable facts stay where
//! they are, and [`ReplyProjection::evict`] only frees memory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use ironclaw_extension_contracts::reply::{
    REPLY_ANSWER_MAX_BYTES, REPLY_DISPLAY_PREVIEW_MAX_BYTES, REPLY_DISPLAY_TEXT_MAX_BYTES,
    REPLY_MAX_ATTACHMENTS, REPLY_REASONING_SEGMENT_MAX_BYTES, ReplyActivityProvenance,
    ReplyActivityState, ReplyAnswerText, ReplyAttachmentRef, ReplyAttention, ReplyAttentionKind,
    ReplyAudience, ReplyChange, ReplyDisplayPreview, ReplyDisplayText, ReplyDocument, ReplyItemId,
    ReplyPhase, ReplyReasoningText, ReplyReconcilePoint, ReplyStatusKind,
};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope, TurnStatus};
use ironclaw_loop_contracts::{
    AgentLoopHostError, LoopDriverNoteKind, LoopGateKind, LoopHostMilestone, LoopHostMilestoneKind,
    LoopHostMilestoneSink, sanitize_model_visible_text,
};
use ironclaw_threads::AttachmentRef;

use crate::run_delivery::prompts::RUN_FAILED_MESSAGE;

#[cfg(test)]
mod tests;

/// Defensive ceiling on live-tracked runs. The runtime's concurrent-run limit
/// is far lower; this only keeps a missing terminal milestone from turning
/// per-run bookkeeping into unbounded process memory.
const DEFAULT_MAX_TRACKED_RUNS: usize = 4_096;

/// What the projection tells its observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyProjectionEvent {
    /// The document moved to a new revision; the point says how urgently a
    /// progressive sink should hear about it.
    Revised(ReplyReconcilePoint),
    /// The loop reported completion or failure. The terminal revision is not
    /// composed from that milestone — the owner of the durable facts must now
    /// fetch them and call [`ReplyProjection::apply_terminal_facts`].
    TerminalPending,
}

/// Something that reacts to the projection moving (reply publication).
pub trait ReplyProjectionObserver: Send + Sync {
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

struct RunReply {
    actor: Option<TurnActor>,
    document: ReplyDocument,
    revision: u64,
    terminal_pending: bool,
    /// The answer text of the model calls that already finished, in order.
    /// `ModelTextDelta` carries the *cumulative* text of the current call,
    /// so the document's answer is this prefix plus that text.
    finished_phases_text: String,
}

impl RunReply {
    fn new(actor: Option<TurnActor>) -> Self {
        Self {
            actor,
            document: ReplyDocument::default(),
            revision: 0,
            terminal_pending: false,
            finished_phases_text: String::new(),
        }
    }

    /// The change that moves the answer to `finished phases + current`: an
    /// append when the new text extends what is shown, a rewrite otherwise
    /// (a model call that restarted its text, or a rewrite under the stream).
    fn answer_change(&self, current_phase_text: &str) -> Option<ReplyChange> {
        let shown = self.document.answer.text.as_str();
        let mut wanted = self.finished_phases_text.clone();
        if !wanted.is_empty()
            && !current_phase_text.is_empty()
            && !wanted.ends_with(char::is_whitespace)
        {
            wanted.push_str("\n\n");
        }
        wanted.push_str(current_phase_text);
        if wanted == shown {
            return None;
        }
        if let Some(suffix) = wanted.strip_prefix(shown)
            && !self.document.answer.truncated
        {
            return Some(ReplyChange::AnswerAppended {
                text: answer_text(suffix)?,
            });
        }
        Some(ReplyChange::AnswerRewritten {
            text: answer_text(&wanted)?,
        })
    }

    /// A model call ended (or another began): whatever the answer shows is
    /// now finished text; the next call's cumulative text lands after it.
    fn close_text_phase(&mut self) {
        self.finished_phases_text = self.document.answer.text.as_str().to_string();
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

    /// Apply a change set as one revision. Returns the reconcile point the
    /// revision sits on, or `None` when nothing was applied (a change the
    /// reducer ignored, e.g. after terminal).
    fn apply(&mut self, changes: &[ReplyChange]) -> Option<ReplyReconcilePoint> {
        if changes.is_empty() {
            return None;
        }
        let before = self.document.applied_changes;
        let mut control_critical = false;
        let mut terminal = false;
        for change in changes {
            self.document.apply(change);
            control_critical |= change.is_control_critical();
            terminal |= change.is_terminal();
        }
        if self.document.applied_changes == before {
            return None;
        }
        let opened = self.revision == 0;
        self.revision = self.revision.saturating_add(1);
        Some(if terminal && self.document.is_terminal() {
            ReplyReconcilePoint::Terminal
        } else if opened {
            ReplyReconcilePoint::Opened
        } else if control_critical {
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
            max_tracked_runs: max_tracked_runs.max(1),
        }
    }

    pub fn add_observer(&self, observer: Arc<dyn ReplyProjectionObserver>) {
        self.observers
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(observer);
    }

    /// Compose one host milestone into the run's document.
    pub fn observe_milestone(&self, milestone: &LoopHostMilestone) {
        let key = RunKey {
            scope: milestone.scope.clone(),
            run_id: milestone.run_id,
        };
        let Some(composed) = compose(&milestone.kind) else {
            return;
        };
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
            let mut changes = composed.changes;
            // A finish for a row this document never saw start (the invoke
            // milestone was lost, or only the terminal one reached us) still
            // needs its title: the capability id, not the activity id.
            if let LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id,
                ..
            }
            | LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id,
                ..
            } = &milestone.kind
                && !run
                    .document
                    .activities
                    .iter()
                    .any(|row| row.id.as_str() == activity_id.to_string())
                && let Some(id) = item_id(&activity_id.to_string())
                && let Some(title) = display_text(capability_id.as_str())
            {
                changes.insert(
                    0,
                    ReplyChange::ActivityStarted {
                        id,
                        title,
                        detail: None,
                    },
                );
            }
            match &milestone.kind {
                // Cumulative text of the current model call → append or
                // rewrite relative to what the document already shows.
                LoopHostMilestoneKind::ModelTextDelta { safe_text } => {
                    let text = sanitize_model_visible_text(safe_text.as_str());
                    if let Some(change) = run.answer_change(&text) {
                        changes.push(change);
                    }
                }
                LoopHostMilestoneKind::ModelStarted { .. }
                | LoopHostMilestoneKind::ModelCompleted { .. } => run.close_text_phase(),
                _ => {}
            }
            // Anything that is not more reasoning ends the open reasoning
            // segment: its final text is the segment as it stands.
            if composed.closes_reasoning
                && run.document.reasoning_open
                && let Some(open) = run.document.reasoning.last().cloned()
            {
                changes.insert(0, ReplyChange::ReasoningSummary { text: open });
            }
            let mut events = Vec::with_capacity(2);
            if let Some(point) = run.apply(&changes) {
                events.push(ReplyProjectionEvent::Revised(point));
            }
            if composed.terminal_pending && !run.document.is_terminal() {
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
        let changes = terminal_changes(&facts);
        let (snapshot, event) = {
            let mut runs = self.lock_runs();
            let run = runs
                .entry(key.clone())
                .or_insert_with(|| RunReply::new(facts.actor.clone()));
            if run.actor.is_none() {
                run.actor = facts.actor.clone();
            }
            let event = if changes.is_empty() {
                None
            } else {
                let point = run.apply(&changes);
                if run.document.is_terminal() {
                    run.terminal_pending = false;
                }
                point.map(ReplyProjectionEvent::Revised)
            };
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

/// What one milestone means for the document.
struct Composed {
    changes: Vec<ReplyChange>,
    /// The milestone ends any open reasoning segment.
    closes_reasoning: bool,
    /// The loop finished; the terminal facts are now worth fetching.
    terminal_pending: bool,
}

impl Composed {
    fn changes(changes: Vec<ReplyChange>) -> Self {
        Self {
            changes,
            closes_reasoning: true,
            terminal_pending: false,
        }
    }
}

/// The document changes one milestone means. `None` for milestones that say
/// nothing user-visible (prompt bundles, checkpoints, hooks, compaction
/// bookkeeping, batch counters).
fn compose(kind: &LoopHostMilestoneKind) -> Option<Composed> {
    let changes = match kind {
        LoopHostMilestoneKind::IterationStarted { iteration } => {
            if *iteration <= 1 {
                vec![ReplyChange::PhaseChanged {
                    phase: ReplyPhase::Preparing,
                }]
            } else {
                // A later iteration means the loop resumed: whatever it was
                // parked on has been answered.
                vec![ReplyChange::AttentionCleared]
            }
        }
        LoopHostMilestoneKind::ModelStarted { .. } => vec![
            ReplyChange::AttentionCleared,
            ReplyChange::PhaseChanged {
                phase: ReplyPhase::Thinking,
            },
        ],
        LoopHostMilestoneKind::ModelReasoningDelta { safe_delta } => {
            let text = sanitize_model_visible_text(safe_delta.as_str());
            return Some(Composed {
                changes: vec![ReplyChange::ReasoningAppended {
                    text: reasoning_text(&text)?,
                }],
                closes_reasoning: false,
                terminal_pending: false,
            });
        }
        // The answer change depends on what the document already shows;
        // `observe_milestone` computes it with the run's state.
        LoopHostMilestoneKind::ModelTextDelta { .. } => Vec::new(),
        LoopHostMilestoneKind::ModelCompleted { .. } => Vec::new(),
        LoopHostMilestoneKind::ModelFailed { .. } => Vec::new(),
        LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id,
        } => vec![ReplyChange::ActivityStarted {
            id: item_id(&activity_id.to_string())?,
            title: display_text(capability_id.as_str())?,
            detail: None,
        }],
        LoopHostMilestoneKind::CapabilityCompleted {
            activity_id,
            provider,
            runtime,
            output_bytes,
            ..
        } => vec![ReplyChange::ActivityFinished {
            id: item_id(&activity_id.to_string())?,
            state: ReplyActivityState::Completed,
            output_preview: None,
            provenance: Some(ReplyActivityProvenance {
                provider: display_text(provider.as_str()),
                runtime: display_text(runtime.as_str()),
                output_bytes: Some(*output_bytes),
            }),
        }],
        LoopHostMilestoneKind::CapabilityFailed {
            activity_id,
            provider,
            runtime,
            reason_kind,
            safe_summary,
            ..
        } => vec![ReplyChange::ActivityFinished {
            id: item_id(&activity_id.to_string())?,
            state: ReplyActivityState::Failed {
                kind: display_text(reason_kind.as_str())?,
            },
            output_preview: safe_summary
                .as_ref()
                .and_then(|summary| ironclaw_event_log::sanitize_error_summary(summary.as_str()))
                .and_then(|summary| display_preview(&summary)),
            provenance: Some(ReplyActivityProvenance {
                provider: provider
                    .as_ref()
                    .and_then(|provider| display_text(provider.as_str())),
                runtime: runtime.and_then(|runtime| display_text(runtime.as_str())),
                output_bytes: None,
            }),
        }],
        LoopHostMilestoneKind::DriverNote { kind, safe_summary } => {
            let text = sanitize_model_visible_text(safe_summary.as_str());
            if text.trim().is_empty() {
                return None;
            }
            vec![ReplyChange::StatusSummary {
                text: display_text(&text)?,
                work: Some(status_kind(*kind)),
            }]
        }
        LoopHostMilestoneKind::GateBlocked { gate_kind, .. } => {
            vec![ReplyChange::AttentionRequired {
                attention: attention_for_gate(*gate_kind, None)?,
            }]
        }
        LoopHostMilestoneKind::Blocked { gate_ref, .. } => {
            // The gate ref arrives one milestone after the kind; carry it onto
            // the attention the publisher enriches at publish time. The kind
            // is re-derived from the ref's own prefix when the loop did not
            // announce it separately.
            vec![ReplyChange::AttentionRequired {
                attention: attention_for_gate_ref(gate_ref.as_str())?,
            }]
        }
        LoopHostMilestoneKind::Completed { .. } | LoopHostMilestoneKind::Failed { .. } => {
            return Some(Composed {
                changes: vec![ReplyChange::AttentionCleared],
                closes_reasoning: true,
                terminal_pending: true,
            });
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
        | LoopHostMilestoneKind::HookFailed { .. } => return None,
    };
    Some(Composed::changes(changes))
}

/// The terminal changes durable facts mean. Empty when the run is not
/// terminal yet (facts fetched too early).
fn terminal_changes(facts: &TerminalReplyFacts) -> Vec<ReplyChange> {
    let mut changes = vec![ReplyChange::AttentionCleared];
    match facts.status {
        TurnStatus::Completed => {
            let text = facts
                .answer
                .as_deref()
                .map(sanitize_model_visible_text)
                .unwrap_or_default();
            if let Some(text) = answer_text(&text).or_else(|| answer_text("")) {
                changes.push(ReplyChange::AnswerFinalized {
                    text,
                    attachments: facts
                        .attachments
                        .iter()
                        .filter_map(attachment_ref)
                        .take(REPLY_MAX_ATTACHMENTS)
                        .collect(),
                });
            }
            changes.push(ReplyChange::Completed);
        }
        TurnStatus::Failed | TurnStatus::RecoveryRequired => {
            let summary = facts
                .failure_summary
                .as_deref()
                .map(sanitize_model_visible_text)
                .filter(|summary| !summary.trim().is_empty())
                .unwrap_or_else(|| RUN_FAILED_MESSAGE.to_string());
            if let Some(summary) =
                display_text(&summary).or_else(|| display_text(RUN_FAILED_MESSAGE))
            {
                changes.push(ReplyChange::Failed { summary });
            } else {
                // Unreachable in practice (the neutral copy is valid display
                // text); a total function still needs an outcome, and a
                // cancel is the honest one for "ended without a summary".
                changes.push(ReplyChange::Cancelled);
            }
        }
        TurnStatus::Cancelled => changes.push(ReplyChange::Cancelled),
        TurnStatus::Queued
        | TurnStatus::Running
        | TurnStatus::BlockedApproval
        | TurnStatus::BlockedAuth
        | TurnStatus::BlockedResource
        | TurnStatus::BlockedDependentRun
        | TurnStatus::BlockedExternalTool
        | TurnStatus::CancelRequested => return Vec::new(),
    }
    changes
}

fn status_kind(kind: LoopDriverNoteKind) -> ReplyStatusKind {
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
    &text[..end]
}

fn strip_control(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

fn display_text(value: &str) -> Option<ReplyDisplayText> {
    let stripped = strip_control(value);
    ReplyDisplayText::new(char_boundary_prefix(
        &stripped,
        REPLY_DISPLAY_TEXT_MAX_BYTES,
    ))
    .ok()
}

fn display_preview(value: &str) -> Option<ReplyDisplayPreview> {
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
