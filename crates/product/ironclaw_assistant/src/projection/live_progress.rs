use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use chrono::Utc;
use ironclaw_event_log::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_event_projections::{
    CapabilityActivityStatus, ProjectionCursor as EventProjectionCursor,
    ProjectionScope as EventProjectionScope,
};
use ironclaw_event_streams::{
    InMemoryProjectionUpdateSource, ProductProjectionEnvelope, ProjectionStreamError,
    ThreadLiveProjectionItem, ThreadLiveProjectionUpdate, ThreadLiveWorkSummaryPhase,
};
use ironclaw_host_api::{
    ids::{CapabilityId, ExtensionId, InvocationId, UserId},
    runtime::RuntimeKind,
};
use ironclaw_loop_contracts::sanitize_model_visible_text;
use ironclaw_loop_host::{SkillActivationObservedEvent, SkillActivationObserver};
use ironclaw_product_contracts::outbound::{
    CapabilityActivityStatusView, CapabilityActivityView, CapabilityActivityViewInput,
    PROJECTION_SKILL_ACTIVATION_MAX_ITEMS, PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
    PROJECTION_SKILL_NAME_MAX_BYTES, ProductProjectionItem, ProductWorkSummaryPhase,
};
use ironclaw_turns::{TurnRunId, TurnScope};

// Live progress uses a synthetic cursor because it is an ephemeral UI hint,
// not a durable runtime event. This sink must remain the only producer on this
// `InMemoryProjectionUpdateSource`: mixing durable `ThreadUpdates` into the
// same live broadcast would put low append-log cursors and high synthetic
// cursors behind the same `last_delivered_cursor` ordering gate.
const LIVE_PROGRESS_CURSOR_BASE: u64 = 1 << 62;
#[derive(Debug)]
pub(super) struct LiveSkillActivationObserver {
    publisher: Arc<LiveProjectionPublisher>,
}

pub struct LiveProjectionPublisher {
    update_source: Arc<InMemoryProjectionUpdateSource>,
    actor_user_id: UserId,
    // Shared by publishers from the same projection services so live cursors
    // stay monotonic across progress, skill, and other projection updates.
    next_sequence: Arc<AtomicU64>,
    no_active_subscriber_logged: AtomicBool,
}

impl std::fmt::Debug for LiveProjectionPublisher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveProjectionPublisher")
            .field("actor_user_id", &self.actor_user_id)
            .finish_non_exhaustive()
    }
}

impl LiveSkillActivationObserver {
    pub(super) fn new(publisher: Arc<LiveProjectionPublisher>) -> Self {
        Self { publisher }
    }
}

impl LiveProjectionPublisher {
    pub(super) fn new(
        update_source: Arc<InMemoryProjectionUpdateSource>,
        actor_user_id: UserId,
        next_sequence: Arc<AtomicU64>,
    ) -> Self {
        Self {
            update_source,
            actor_user_id,
            next_sequence,
            no_active_subscriber_logged: AtomicBool::new(false),
        }
    }

    fn next_live_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn publish_live_item(
        &self,
        owner: Option<&UserId>,
        scope: &TurnScope,
        sequence: u64,
        item: ThreadLiveProjectionItem,
    ) {
        let cursor = EventProjectionCursor::for_scope(
            self.projection_scope(owner, scope),
            EventCursor::new(LIVE_PROGRESS_CURSOR_BASE.saturating_add(sequence)),
        );
        let update = ThreadLiveProjectionUpdate {
            cursor,
            thread_id: scope.thread_id.clone(),
            items: vec![item],
        };
        match self
            .update_source
            .publish(ProductProjectionEnvelope::ThreadLiveUpdate(update))
        {
            Ok(_) => {
                self.no_active_subscriber_logged
                    .store(false, Ordering::Relaxed);
            }
            Err(ProjectionStreamError::Source) => {
                if !self
                    .no_active_subscriber_logged
                    .swap(true, Ordering::Relaxed)
                {
                    tracing::debug!(
                        "live progress projection buffered without an active subscriber"
                    );
                }
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "failed to publish live progress projection"
                );
            }
        }
    }

    /// Publish a "learned a new skill" live item to the run's thread stream,
    /// reusing the [`ThreadLiveProjectionItem::SkillActivation`] projection
    /// (rendered as a chat bubble). Called post-run by the skill-learning sink:
    /// the in-run [`SkillActivationObserver`] only fires at prompt-build for
    /// skill *selection*, so a learned-skill notification has no producer
    /// otherwise. Best-effort — drops silently if names/feedback sanitize empty.
    pub fn publish_skill_learned(
        &self,
        owner: Option<&UserId>,
        scope: &TurnScope,
        run_id: TurnRunId,
        skill_name: &str,
        feedback: &str,
    ) {
        let name = sanitize_bounded_model_visible_text(skill_name, PROJECTION_SKILL_NAME_MAX_BYTES);
        let note =
            sanitize_bounded_model_visible_text(feedback, PROJECTION_SKILL_FEEDBACK_MAX_BYTES);
        if name.is_empty() && note.is_empty() {
            return;
        }
        let sequence = self.next_live_sequence();
        self.publish_live_item(
            owner,
            scope,
            sequence,
            ThreadLiveProjectionItem::SkillActivation {
                id: skill_activation_id(run_id, sequence),
                run_id,
                skill_names: if name.is_empty() {
                    Vec::new()
                } else {
                    vec![name]
                },
                feedback: if note.is_empty() {
                    Vec::new()
                } else {
                    vec![note]
                },
            },
        );
    }

    /// Build the projection scope for a live item. The stream key is keyed
    /// to the per-run `owner` (the authenticated caller) when one is
    /// threaded through, falling back to the runtime owner only for host
    /// paths that bind no actor. This MUST match the per-request actor the
    /// SSE/WS subscribe side uses
    /// (`projection::runtime_projection_scope`) — otherwise a turn run by
    /// an SSO user whose id differs from the runtime owner would publish
    /// live progress to the operator's stream instead of the user's.
    fn projection_scope(&self, owner: Option<&UserId>, scope: &TurnScope) -> EventProjectionScope {
        let owner = owner.unwrap_or(&self.actor_user_id);
        EventProjectionScope {
            stream: EventStreamKey::new(
                scope.tenant_id.clone(),
                owner.clone(),
                scope.agent_id.clone(),
            ),
            read_scope: ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: None,
                thread_id: Some(scope.thread_id.clone()),
                process_id: None,
            },
        }
    }
}

pub(super) fn product_items_for_live_update(
    display_previews: &dyn super::display_preview::CapabilityDisplayPreviewSource,
    update: &ThreadLiveProjectionUpdate,
) -> Vec<ProductProjectionItem> {
    update
        .items
        .iter()
        .filter_map(|item| match item {
            ThreadLiveProjectionItem::Text {
                id,
                run_id,
                body,
                narration,
            } => Some(ProductProjectionItem::Text {
                id: id.clone(),
                run_id: Some(*run_id),
                body: body.clone(),
                finalized: false,
                narration: *narration,
            }),
            ThreadLiveProjectionItem::Thinking { id, run_id, body } => {
                Some(ProductProjectionItem::Thinking {
                    id: id.clone(),
                    run_id: Some(*run_id),
                    body: body.clone(),
                })
            }
            ThreadLiveProjectionItem::CapabilityActivity {
                run_id,
                invocation_id,
                capability_id,
                status,
                provider,
                runtime,
                output_bytes,
                error_kind,
                error_detail,
                input_summary,
            } => {
                let running = display_previews.running_input(*invocation_id);
                match CapabilityActivityView::new(CapabilityActivityViewInput {
                    invocation_id: *invocation_id,
                    turn_run_id: Some(*run_id),
                    thread_id: Some(update.thread_id.clone()),
                    capability_id: capability_id.clone(),
                    status: live_capability_activity_status(*status),
                    provider: provider.clone(),
                    runtime: *runtime,
                    process_id: None,
                    output_bytes: *output_bytes,
                    error_kind: error_kind.clone(),
                    error_detail: error_detail.clone(),
                    subtitle: running.as_ref().and_then(|input| input.subtitle.clone()),
                    input_summary: input_summary
                        .clone()
                        .or_else(|| running.and_then(|input| input.input_summary)),
                    updated_at: Utc::now(),
                    activity_order: None,
                }) {
                    Ok(activity) => Some(ProductProjectionItem::CapabilityActivity(activity)),
                    Err(error) => {
                        tracing::debug!(
                            error = %error,
                            invocation_id = %invocation_id,
                            capability_id = %capability_id,
                            "live capability activity rejected by product adapter boundary"
                        );
                        None
                    }
                }
            }
            ThreadLiveProjectionItem::WorkSummary {
                id,
                run_id,
                phase,
                body,
            } => Some(ProductProjectionItem::WorkSummary {
                id: id.clone(),
                run_id: *run_id,
                phase: live_work_summary_phase_to_product_phase(*phase),
                body: body.clone(),
            }),
            ThreadLiveProjectionItem::SkillActivation {
                id,
                run_id,
                skill_names,
                feedback,
            } => Some(ProductProjectionItem::SkillActivation {
                id: id.clone(),
                run_id: *run_id,
                skill_names: skill_names.clone(),
                feedback: feedback.clone(),
            }),
        })
        .collect()
}

fn live_work_summary_phase_to_product_phase(
    phase: ThreadLiveWorkSummaryPhase,
) -> ProductWorkSummaryPhase {
    match phase {
        ThreadLiveWorkSummaryPhase::Planning => ProductWorkSummaryPhase::Planning,
        ThreadLiveWorkSummaryPhase::Waiting => ProductWorkSummaryPhase::Waiting,
        ThreadLiveWorkSummaryPhase::Retrying => ProductWorkSummaryPhase::Retrying,
        ThreadLiveWorkSummaryPhase::Context => ProductWorkSummaryPhase::Context,
    }
}

impl SkillActivationObserver for LiveSkillActivationObserver {
    fn observe_skill_activation(&self, event: SkillActivationObservedEvent) {
        let skill_names = event
            .activations
            .iter()
            .map(|activation| {
                sanitize_bounded_model_visible_text(
                    &activation.name,
                    PROJECTION_SKILL_NAME_MAX_BYTES,
                )
            })
            .filter(|name| !name.is_empty())
            .take(PROJECTION_SKILL_ACTIVATION_MAX_ITEMS)
            .collect::<Vec<_>>();
        let feedback = event
            .feedback
            .iter()
            .map(|note| {
                sanitize_bounded_model_visible_text(note, PROJECTION_SKILL_FEEDBACK_MAX_BYTES)
            })
            .filter(|note| !note.is_empty())
            .take(PROJECTION_SKILL_ACTIVATION_MAX_ITEMS)
            .collect::<Vec<_>>();
        if skill_names.is_empty() && feedback.is_empty() {
            return;
        }
        let sequence = self.publisher.next_live_sequence();
        self.publisher.publish_live_item(
            event.run_context.actor().map(|actor| &actor.user_id),
            &event.run_context.scope,
            sequence,
            ThreadLiveProjectionItem::SkillActivation {
                id: skill_activation_id(event.run_context.run_id, sequence),
                run_id: event.run_context.run_id,
                skill_names,
                feedback,
            },
        );
    }
}

fn live_capability_activity_status(
    status: CapabilityActivityStatus,
) -> CapabilityActivityStatusView {
    match status {
        CapabilityActivityStatus::Started => CapabilityActivityStatusView::Started,
        CapabilityActivityStatus::Running => CapabilityActivityStatusView::Running,
        CapabilityActivityStatus::Completed => CapabilityActivityStatusView::Completed,
        CapabilityActivityStatus::Failed => CapabilityActivityStatusView::Failed,
        CapabilityActivityStatus::Killed => CapabilityActivityStatusView::Killed,
    }
}

/// Stable per-(run, reasoning-segment) item id, so a growing open segment
/// and its eventual closed replacement upsert one browser item instead of
/// appending duplicates.
fn thinking_id(run_id: TurnRunId, segment_index: u64) -> String {
    format!("thinking:{run_id}:{segment_index}")
}

/// Stable per-(run, answer-phase) item id: one live text item per model
/// call, so a call the loop went on past keeps its own item (re-homed as
/// narration) while the next call's text streams as a new one. The format
/// the browser keyed on before replies were published progressively.
fn text_phase_id(run_id: TurnRunId, phase: u64) -> String {
    format!("text:{run_id}:{phase}")
}

fn work_summary_id(run_id: TurnRunId, sequence: u64) -> String {
    format!("work-summary:{run_id}:{sequence}")
}

fn skill_activation_id(run_id: TurnRunId, sequence: u64) -> String {
    format!("skill-activation:{run_id}:{sequence}")
}

fn sanitize_bounded_model_visible_text(value: &str, max_bytes: usize) -> String {
    let sanitized = sanitize_model_visible_text(value);
    let trimmed = sanitized.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].trim_end().to_string()
}

// ── Reply-publication edge ───────────────────────────────────────────────
//
// The product projection reply sink hands each reconciled revision here. The
// document is desired state, so the publisher diffs it against a bounded,
// host-persisted checkpoint and republishes only the facets that changed;
// stable item ids make even a full republish idempotent for the browser.

/// Checkpoint version the projection publisher writes. A checkpoint of any
/// other version is ignored (everything is republished), never misread.
const PROJECTION_REPLY_CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ProjectionReplyCheckpoint {
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    answer_len: usize,
    /// Fingerprint of the published answer text: a rewrite that keeps the
    /// same length must still republish.
    #[serde(default)]
    answer_fingerprint: u64,
    #[serde(default)]
    answer_finalized: bool,
    /// The answer phase the three fields above describe; a new phase starts
    /// its own text item from nothing.
    #[serde(default)]
    answer_phase: u64,
    /// How many narration entries (calls the loop went on past) have been
    /// republished under their phase id with the narration flag.
    #[serde(default)]
    narration_published: usize,
    /// How many CLOSED reasoning segments have been published with their
    /// final text (the open tail is tracked by fingerprint below, because
    /// `append_reasoning` grows it in place without changing the count).
    #[serde(default)]
    reasoning_segments: usize,
    /// Fingerprint of the open tail segment as last published; 0 when no
    /// open tail was published.
    #[serde(default)]
    open_reasoning_fingerprint: u64,
    /// The highest activity `updated_ordinal` already published.
    #[serde(default)]
    activity_ordinal: u64,
    #[serde(default)]
    status: Option<String>,
    /// How many status lines were published; mints unique work-summary ids.
    #[serde(default)]
    status_publications: u64,
}

impl ProjectionReplyCheckpoint {
    fn restore(
        checkpoint: Option<&ironclaw_extension_contracts::reply::ReplySinkCheckpoint>,
    ) -> Self {
        let Some(checkpoint) = checkpoint else {
            return Self::default();
        };
        if checkpoint.version() != PROJECTION_REPLY_CHECKPOINT_VERSION {
            return Self::default();
        }
        serde_json::from_str(checkpoint.payload()).unwrap_or_default() // silent-ok: an unreadable checkpoint republishes every facet, which stable ids make idempotent
    }

    fn seal(
        &self,
    ) -> Result<
        ironclaw_extension_contracts::reply::ReplySinkCheckpoint,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        let payload = serde_json::to_string(self).map_err(|error| {
            ironclaw_extension_contracts::channel_adapter::ChannelError::Render {
                reason: format!("projection reply checkpoint could not be encoded: {error}"),
            }
        })?;
        ironclaw_extension_contracts::reply::ReplySinkCheckpoint::new(
            PROJECTION_REPLY_CHECKPOINT_VERSION,
            payload,
        )
        .map_err(|error| {
            ironclaw_extension_contracts::channel_adapter::ChannelError::Render {
                reason: format!("projection reply checkpoint exceeds its bound: {error}"),
            }
        })
    }
}

impl LiveProjectionPublisher {
    /// Publish the facets of `request.revision` that changed since the
    /// checkpoint as live projection items, and return the next checkpoint.
    pub(crate) fn publish_reply_revision(
        &self,
        request: &ironclaw_extension_contracts::reply::ReplyReconcileRequest,
    ) -> Result<
        ironclaw_extension_contracts::reply::ReplySinkCheckpoint,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        use ironclaw_extension_contracts::reply::ReplyActivityState;

        let mut checkpoint = ProjectionReplyCheckpoint::restore(request.checkpoint.as_ref());
        let document = &request.revision.document;
        let target = &request.target;
        let owner = Some(&target.actor.user_id);
        let scope = &target.scope;
        let run_id = target.run_id;

        // Closed segments publish once with their final text; the open tail
        // republishes (under its stable per-segment id, so the browser
        // upserts) whenever its in-place-grown text changes, and once more
        // when `close_reasoning` replaces it.
        let segments = &document.reasoning;
        let closed_count = if document.reasoning_open {
            segments.len().saturating_sub(1)
        } else {
            segments.len()
        };
        // Clamp: a checkpoint written before the open-tail tracking existed
        // counted the open tail too; treating it as closed would suppress
        // its final text.
        let published_closed = checkpoint.reasoning_segments.min(closed_count);
        for (index, segment) in segments
            .iter()
            .enumerate()
            .take(closed_count)
            .skip(published_closed)
        {
            let sequence = self.next_live_sequence();
            self.publish_live_item(
                owner,
                scope,
                sequence,
                ThreadLiveProjectionItem::Thinking {
                    id: thinking_id(run_id, index as u64),
                    run_id,
                    body: segment.as_str().to_string(),
                },
            );
        }
        checkpoint.reasoning_segments = closed_count;
        if document.reasoning_open
            && let Some(open) = segments.last()
        {
            let open_fingerprint = text_fingerprint(open.as_str());
            if open_fingerprint != checkpoint.open_reasoning_fingerprint {
                let sequence = self.next_live_sequence();
                self.publish_live_item(
                    owner,
                    scope,
                    sequence,
                    ThreadLiveProjectionItem::Thinking {
                        id: thinking_id(run_id, closed_count as u64),
                        run_id,
                        body: open.as_str().to_string(),
                    },
                );
                checkpoint.open_reasoning_fingerprint = open_fingerprint;
            }
        } else {
            checkpoint.open_reasoning_fingerprint = 0;
        }

        // A call the loop went on past is narration: it republishes once,
        // under the id it streamed as, flagged so the browser re-homes it
        // into the run's activity — ahead of the activity that proved it
        // narration and of the next phase's text. The republish carries the
        // phase's final text even when coalesced revisions skipped its tail.
        for entry in document
            .narration
            .iter()
            .skip(checkpoint.narration_published)
        {
            let sequence = self.next_live_sequence();
            self.publish_live_item(
                owner,
                scope,
                sequence,
                ThreadLiveProjectionItem::Text {
                    id: text_phase_id(run_id, entry.phase),
                    run_id,
                    body: entry.text.as_str().to_string(),
                    narration: true,
                },
            );
        }
        checkpoint.narration_published = document.narration.len();

        // The current phase's text under its own id. A new phase starts
        // from nothing; the earlier phase's item keeps the text it showed.
        if document.answer.phase != checkpoint.answer_phase {
            checkpoint.answer_len = 0;
            checkpoint.answer_fingerprint = 0;
            checkpoint.answer_finalized = false;
        }
        let answer = document.answer.text.as_str();
        let answer_fingerprint = text_fingerprint(answer);
        if !answer.is_empty()
            && (answer.len() != checkpoint.answer_len
                || answer_fingerprint != checkpoint.answer_fingerprint
                || document.answer.finalized != checkpoint.answer_finalized)
        {
            let sequence = self.next_live_sequence();
            self.publish_live_item(
                owner,
                scope,
                sequence,
                ThreadLiveProjectionItem::Text {
                    id: text_phase_id(run_id, document.answer.phase),
                    run_id,
                    body: answer.to_string(),
                    narration: false,
                },
            );
        }
        checkpoint.answer_phase = document.answer.phase;
        checkpoint.answer_len = answer.len();
        checkpoint.answer_fingerprint = answer_fingerprint;
        checkpoint.answer_finalized = document.answer.finalized;

        let mut highest_ordinal = checkpoint.activity_ordinal;
        for activity in document
            .activities
            .iter()
            .filter(|activity| activity.updated_ordinal > checkpoint.activity_ordinal)
        {
            highest_ordinal = highest_ordinal.max(activity.updated_ordinal);
            let Ok(invocation) = uuid::Uuid::parse_str(activity.id.as_str()) else {
                tracing::debug!(
                    activity_id = %activity.id,
                    "reply activity id is not an invocation identity; not projected as a capability card"
                );
                continue;
            };
            let Ok(capability_id) = CapabilityId::new(activity.title.as_str()) else {
                tracing::debug!(
                    activity_id = %activity.id,
                    "reply activity title is not a capability id; not projected as a capability card"
                );
                continue;
            };
            let (status, error_kind, error_detail) = match &activity.state {
                ReplyActivityState::Started => (CapabilityActivityStatus::Started, None, None),
                ReplyActivityState::Completed => (CapabilityActivityStatus::Completed, None, None),
                ReplyActivityState::Failed { kind } => (
                    CapabilityActivityStatus::Failed,
                    Some(kind.as_str().to_string()),
                    activity
                        .output_preview
                        .as_ref()
                        .map(|preview| preview.as_str().to_string()),
                ),
            };
            let sequence = self.next_live_sequence();
            self.publish_live_item(
                owner,
                scope,
                sequence,
                ThreadLiveProjectionItem::CapabilityActivity {
                    run_id,
                    invocation_id: InvocationId::from_uuid(invocation),
                    capability_id,
                    status,
                    provider: activity
                        .provenance
                        .as_ref()
                        .and_then(|p| p.provider.as_ref())
                        .and_then(|provider| ExtensionId::new(provider.as_str()).ok()),
                    runtime: activity
                        .provenance
                        .as_ref()
                        .and_then(|p| p.runtime.as_ref())
                        .and_then(|runtime| runtime_kind_from_display(runtime.as_str())),
                    output_bytes: activity.provenance.as_ref().and_then(|p| p.output_bytes),
                    error_kind,
                    error_detail,
                    input_summary: activity
                        .detail
                        .as_ref()
                        .map(|detail| detail.as_str().to_string()),
                },
            );
        }
        checkpoint.activity_ordinal = highest_ordinal;

        let status = document
            .status
            .as_ref()
            .map(|status| status.as_str().to_string());
        if status != checkpoint.status
            && let Some(body) = &status
        {
            checkpoint.status_publications = checkpoint.status_publications.saturating_add(1);
            let sequence = self.next_live_sequence();
            self.publish_live_item(
                owner,
                scope,
                sequence,
                ThreadLiveProjectionItem::WorkSummary {
                    // Keyed on the CHECKPOINT counter, not the process-global
                    // sequence: a republish from a restored (or lost)
                    // checkpoint re-mints the same id, so the browser
                    // upserts the status line instead of appending a twin.
                    id: work_summary_id(run_id, checkpoint.status_publications),
                    run_id,
                    phase: document
                        .status_kind
                        .map(reply_status_kind_to_live_work_summary_phase)
                        .unwrap_or_else(|| reply_phase_to_live_work_summary_phase(document.phase)),
                    body: body.clone(),
                },
            );
        }
        checkpoint.status = status;
        checkpoint.revision = request.revision.revision;
        checkpoint.seal()
    }
}

/// The runtime lane a provenance display text names; the display text is the
/// lane's own `as_str()`, so this is the inverse of that mapping.
fn runtime_kind_from_display(text: &str) -> Option<RuntimeKind> {
    // The exhaustive match keeps the candidate list honest: a new
    // `RuntimeKind` variant fails to compile here instead of silently
    // losing the capability card's runtime lane.
    const fn every_kind_is_listed(kind: RuntimeKind) {
        match kind {
            RuntimeKind::Wasm
            | RuntimeKind::Mcp
            | RuntimeKind::Script
            | RuntimeKind::Sandbox
            | RuntimeKind::FirstParty
            | RuntimeKind::System => {}
        }
    }
    let _ = every_kind_is_listed;
    [
        RuntimeKind::Wasm,
        RuntimeKind::Mcp,
        RuntimeKind::Script,
        RuntimeKind::Sandbox,
        RuntimeKind::FirstParty,
        RuntimeKind::System,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == text)
}

fn text_fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn reply_status_kind_to_live_work_summary_phase(
    kind: ironclaw_extension_contracts::reply::ReplyStatusKind,
) -> ThreadLiveWorkSummaryPhase {
    use ironclaw_extension_contracts::reply::ReplyStatusKind;
    match kind {
        ReplyStatusKind::Planning => ThreadLiveWorkSummaryPhase::Planning,
        ReplyStatusKind::Waiting => ThreadLiveWorkSummaryPhase::Waiting,
        ReplyStatusKind::Retrying => ThreadLiveWorkSummaryPhase::Retrying,
        ReplyStatusKind::Context => ThreadLiveWorkSummaryPhase::Context,
    }
}

fn reply_phase_to_live_work_summary_phase(
    phase: ironclaw_extension_contracts::reply::ReplyPhase,
) -> ThreadLiveWorkSummaryPhase {
    use ironclaw_extension_contracts::reply::ReplyPhase;
    match phase {
        ReplyPhase::Preparing | ReplyPhase::Thinking => ThreadLiveWorkSummaryPhase::Planning,
        ReplyPhase::WaitingForInput => ThreadLiveWorkSummaryPhase::Waiting,
        ReplyPhase::Working
        | ReplyPhase::Completed
        | ReplyPhase::Failed
        | ReplyPhase::Cancelled => ThreadLiveWorkSummaryPhase::Context,
    }
}
