use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_event_projections::{
    CapabilityActivityStatus, ProjectionCursor as EventProjectionCursor,
    ProjectionScope as EventProjectionScope,
};
use ironclaw_event_streams::{
    InMemoryProjectionUpdateSource, ProductProjectionEnvelope, ProjectionStreamError,
    ThreadLiveProjectionItem, ThreadLiveProjectionUpdate, ThreadLiveWorkSummaryPhase,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope, sanitize_error_summary};
use ironclaw_first_party_extension_ports::{SkillActivationObservedEvent, SkillActivationObserver};
use ironclaw_host_api::{CapabilityId, ExtensionId, InvocationId, RuntimeKind, UserId};
use ironclaw_product_adapters::{
    CapabilityActivityStatusView, CapabilityActivityView, CapabilityActivityViewInput,
    PROJECTION_SKILL_ACTIVATION_MAX_ITEMS, PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
    PROJECTION_SKILL_NAME_MAX_BYTES, PROJECTION_TEXT_MAX_BYTES, ProductProjectionItem,
    ProductWorkSummaryPhase,
};
use ironclaw_turns::{
    TurnRunId, TurnScope,
    run_profile::{
        AgentLoopHostError, LoopDriverNoteKind, LoopHostMilestone, LoopHostMilestoneKind,
        LoopHostMilestoneSink, LoopSafeSummary, sanitize_model_visible_text,
    },
};

// Live progress uses a synthetic cursor because it is an ephemeral UI hint,
// not a durable runtime event. This sink must remain the only producer on this
// `InMemoryProjectionUpdateSource`: mixing durable `ThreadUpdates` into the
// same live broadcast would put low append-log cursors and high synthetic
// cursors behind the same `last_delivered_cursor` ordering gate.
const LIVE_PROGRESS_CURSOR_BASE: u64 = 1 << 62;
const LIVE_TEXT_COALESCE_WINDOW: Duration = Duration::from_millis(75);

pub(super) struct LiveProgressMilestoneSink {
    inner: Arc<dyn LoopHostMilestoneSink>,
    publisher: Arc<LiveProjectionPublisher>,
    text_coalescer: Arc<LiveTextProjectionCoalescer>,
}

#[derive(Debug)]
pub(super) struct LiveSkillActivationObserver {
    publisher: Arc<LiveProjectionPublisher>,
}

pub(crate) struct LiveProjectionPublisher {
    update_source: Arc<InMemoryProjectionUpdateSource>,
    actor_user_id: UserId,
    // Shared by publishers from the same projection services so live cursors
    // stay monotonic across progress, skill, and other projection updates.
    next_sequence: Arc<AtomicU64>,
    no_active_subscriber_logged: AtomicBool,
}

struct LiveTextProjectionCoalescer {
    publisher: Arc<LiveProjectionPublisher>,
    next_generation: AtomicU64,
    // State mutation and channel publication are separate critical sections.
    // This guard preserves their relative order without holding `states` while
    // the broadcast source wakes subscribers.
    publication_order: Mutex<()>,
    states: Mutex<HashMap<TurnRunId, LiveTextProjectionState>>,
}

struct LiveTextProjectionState {
    generation: u64,
    last_published_at: tokio::time::Instant,
    pending: Option<PendingTextProjection>,
    timer_scheduled: bool,
}

struct PendingTextProjection {
    owner: Option<UserId>,
    scope: TurnScope,
    run_id: TurnRunId,
    body: String,
}

impl std::fmt::Debug for LiveProjectionPublisher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveProjectionPublisher")
            .field("actor_user_id", &self.actor_user_id)
            .finish_non_exhaustive()
    }
}

impl LiveProgressMilestoneSink {
    pub(super) fn new(
        inner: Arc<dyn LoopHostMilestoneSink>,
        publisher: Arc<LiveProjectionPublisher>,
    ) -> Self {
        Self {
            inner,
            text_coalescer: Arc::new(LiveTextProjectionCoalescer::new(Arc::clone(&publisher))),
            publisher,
        }
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
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn publish_skill_learned(
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

impl LiveTextProjectionCoalescer {
    fn new(publisher: Arc<LiveProjectionPublisher>) -> Self {
        Self {
            publisher,
            next_generation: AtomicU64::new(0),
            publication_order: Mutex::new(()),
            states: Mutex::new(HashMap::new()),
        }
    }

    fn submit(self: &Arc<Self>, projection: PendingTextProjection) {
        // ModelTextDelta carries the full cumulative assistant text, so an
        // intermediate replacement is redundant. Keep this policy here rather
        // than in the generic stream manager, whose other items are lossless.
        let run_id = projection.run_id;
        let mut timer = None;
        let mut publish_projection = None;
        {
            let _publication_order = self.lock_publication_order();
            {
                let mut states = self.lock_states();
                match states.get_mut(&run_id) {
                    Some(state) => {
                        state.pending = Some(projection);
                        if !state.timer_scheduled {
                            state.timer_scheduled = true;
                            timer = Some((
                                state.generation,
                                state.last_published_at + LIVE_TEXT_COALESCE_WINDOW,
                            ));
                        }
                    }
                    None => {
                        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
                        states.insert(
                            run_id,
                            LiveTextProjectionState {
                                generation,
                                last_published_at: tokio::time::Instant::now(),
                                pending: None,
                                timer_scheduled: false,
                            },
                        );
                        publish_projection = Some(projection);
                    }
                }
            }
            if let Some(projection) = publish_projection.take() {
                self.publish(projection);
            }
        }

        if let Some((generation, deadline)) = timer {
            let coalescer = Arc::clone(self);
            tokio::spawn(async move {
                tokio::time::sleep_until(deadline).await;
                coalescer.flush_timer(run_id, generation);
            });
        }
    }

    fn flush_boundary(&self, run_id: TurnRunId) {
        let _publication_order = self.lock_publication_order();
        let projection = {
            let mut states = self.lock_states();
            states.remove(&run_id).and_then(|state| state.pending)
        };
        if let Some(projection) = projection {
            self.publish(projection);
        }
    }

    fn flush_timer(&self, run_id: TurnRunId, generation: u64) {
        let _publication_order = self.lock_publication_order();
        let projection = {
            let mut states = self.lock_states();
            let Some(state) = states.get_mut(&run_id) else {
                return;
            };
            if state.generation != generation {
                return;
            }

            state.timer_scheduled = false;
            let projection = state.pending.take();
            if projection.is_some() {
                state.last_published_at = tokio::time::Instant::now();
            }
            projection
        };
        if let Some(projection) = projection {
            self.publish(projection);
        }
    }

    fn publish(&self, projection: PendingTextProjection) {
        let sequence = self.publisher.next_live_sequence();
        self.publisher.publish_live_item(
            projection.owner.as_ref(),
            &projection.scope,
            sequence,
            ThreadLiveProjectionItem::Text {
                id: text_id(projection.run_id),
                run_id: projection.run_id,
                body: projection.body,
            },
        );
    }

    fn lock_states(&self) -> MutexGuard<'_, HashMap<TurnRunId, LiveTextProjectionState>> {
        match self.states.lock() {
            Ok(states) => states,
            Err(poisoned) => {
                tracing::debug!("live text projection coalescer lock recovered after panic");
                self.states.clear_poison();
                poisoned.into_inner()
            }
        }
    }

    fn lock_publication_order(&self) -> MutexGuard<'_, ()> {
        match self.publication_order.lock() {
            Ok(order) => order,
            Err(poisoned) => {
                tracing::debug!("live text projection publication lock recovered after panic");
                self.publication_order.clear_poison();
                poisoned.into_inner()
            }
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
            ThreadLiveProjectionItem::Text { id, run_id, body } => {
                Some(ProductProjectionItem::Text {
                    id: id.clone(),
                    run_id: Some(*run_id),
                    body: body.clone(),
                })
            }
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
                    input_summary: running.and_then(|input| input.input_summary),
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

impl LiveProgressMilestoneSink {
    fn publish_text_delta(&self, milestone: &LoopHostMilestone, safe_text: &str) {
        // The model port already sanitizes chunks before milestone emission.
        // Re-sanitize and bound here because this path is browser-facing.
        let body = sanitize_bounded_projection_text(safe_text, PROJECTION_TEXT_MAX_BYTES);
        if body.is_empty() {
            return;
        }
        self.text_coalescer.submit(PendingTextProjection {
            owner: milestone.actor.as_ref().map(|actor| actor.user_id.clone()),
            scope: milestone.scope.clone(),
            run_id: milestone.run_id,
            body,
        });
    }

    fn publish_reasoning_delta(&self, milestone: &LoopHostMilestone, safe_delta: &str) {
        // The delta is already model-visible sanitized upstream. Re-sanitize at
        // the product projection boundary so this publish path has its own
        // last-mile redaction gate before sending a browser-facing payload.
        let safe_delta = sanitize_model_visible_text(safe_delta);
        if safe_delta.is_empty() {
            return;
        }
        let sequence = self.publisher.next_live_sequence();
        self.publisher.publish_live_item(
            milestone.actor.as_ref().map(|actor| &actor.user_id),
            &milestone.scope,
            sequence,
            ThreadLiveProjectionItem::Thinking {
                id: thinking_id(milestone.run_id, sequence),
                run_id: milestone.run_id,
                body: safe_delta,
            },
        );
    }

    fn publish_capability_activity(
        &self,
        milestone: &LoopHostMilestone,
        invocation_id: InvocationId,
        capability_id: &CapabilityId,
        status: CapabilityActivityStatus,
        terminal: TerminalCapabilityActivity,
    ) {
        let sequence = self.publisher.next_live_sequence();
        self.publisher.publish_live_item(
            milestone.actor.as_ref().map(|actor| &actor.user_id),
            &milestone.scope,
            sequence,
            ThreadLiveProjectionItem::CapabilityActivity {
                run_id: milestone.run_id,
                invocation_id,
                capability_id: capability_id.clone(),
                status,
                provider: terminal.provider,
                runtime: terminal.runtime,
                output_bytes: terminal.output_bytes,
                error_kind: terminal.error_kind,
                error_detail: terminal.error_detail,
            },
        );
    }

    fn publish_work_summary(
        &self,
        milestone: &LoopHostMilestone,
        kind: LoopDriverNoteKind,
        safe_summary: &str,
    ) {
        let body = sanitize_model_visible_text(safe_summary).trim().to_string();
        if body.is_empty() {
            return;
        }
        let body = match LoopSafeSummary::new(body) {
            Ok(summary) => summary.to_string(),
            Err(reason) => {
                tracing::debug!(
                    reason = %reason,
                    run_id = %milestone.run_id,
                    "live progress work summary rejected by boundary validation"
                );
                return;
            }
        };
        let sequence = self.publisher.next_live_sequence();
        self.publisher.publish_live_item(
            milestone.actor.as_ref().map(|actor| &actor.user_id),
            &milestone.scope,
            sequence,
            ThreadLiveProjectionItem::WorkSummary {
                id: work_summary_id(milestone.run_id, sequence),
                run_id: milestone.run_id,
                phase: driver_note_kind_to_live_work_summary_phase(kind),
                body,
            },
        );
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

#[async_trait]
impl LoopHostMilestoneSink for LiveProgressMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.publish_loop_milestone(milestone.clone()).await?;
        if !matches!(
            &milestone.kind,
            LoopHostMilestoneKind::ModelTextDelta { .. }
        ) {
            self.text_coalescer.flush_boundary(milestone.run_id);
        }
        match &milestone.kind {
            LoopHostMilestoneKind::ModelTextDelta { safe_text } => {
                self.publish_text_delta(&milestone, safe_text);
            }
            LoopHostMilestoneKind::ModelReasoningDelta { safe_delta } => {
                self.publish_reasoning_delta(&milestone, safe_delta);
            }
            LoopHostMilestoneKind::CapabilityInvoked {
                activity_id,
                capability_id,
            } => {
                self.publish_capability_activity(
                    &milestone,
                    InvocationId::from_uuid(activity_id.as_uuid()),
                    capability_id,
                    CapabilityActivityStatus::Started,
                    TerminalCapabilityActivity::default(),
                );
            }
            LoopHostMilestoneKind::CapabilityCompleted {
                activity_id,
                capability_id,
                provider,
                runtime,
                output_bytes,
            } => {
                self.publish_capability_activity(
                    &milestone,
                    InvocationId::from_uuid(activity_id.as_uuid()),
                    capability_id,
                    CapabilityActivityStatus::Completed,
                    TerminalCapabilityActivity {
                        provider: Some(provider.clone()),
                        runtime: Some(*runtime),
                        output_bytes: Some(*output_bytes),
                        error_kind: None,
                        error_detail: None,
                    },
                );
            }
            LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id,
                provider,
                runtime,
                reason_kind,
                safe_summary,
            } => {
                self.publish_capability_activity(
                    &milestone,
                    InvocationId::from_uuid(activity_id.as_uuid()),
                    capability_id,
                    CapabilityActivityStatus::Failed,
                    TerminalCapabilityActivity {
                        provider: provider.clone(),
                        runtime: *runtime,
                        output_bytes: None,
                        error_kind: Some(reason_kind.as_str().to_string()),
                        error_detail: sanitized_capability_error_detail(
                            safe_summary.as_ref().map(LoopSafeSummary::as_str),
                        ),
                    },
                );
            }
            LoopHostMilestoneKind::DriverNote { kind, safe_summary } => {
                self.publish_work_summary(&milestone, *kind, safe_summary.as_str());
            }
            _ => {}
        }
        Ok(())
    }
}

/// Sanitize and bound a host-authored capability failure summary for the live
/// activity card. Returns `None` for absent/empty input so the card falls back
/// to the bare error kind. The product-adapter boundary re-validates length and
/// control chars.
fn sanitized_capability_error_detail(safe_summary: Option<&str>) -> Option<String> {
    let summary = safe_summary?;
    sanitize_error_summary(summary)
}

#[derive(Default)]
struct TerminalCapabilityActivity {
    provider: Option<ExtensionId>,
    runtime: Option<RuntimeKind>,
    output_bytes: Option<u64>,
    error_kind: Option<String>,
    error_detail: Option<String>,
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

fn thinking_id(run_id: TurnRunId, sequence: u64) -> String {
    format!("thinking:{run_id}:{sequence}")
}

fn text_id(run_id: TurnRunId) -> String {
    format!("text:{run_id}")
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

fn sanitize_bounded_projection_text(value: &str, max_bytes: usize) -> String {
    let sanitized = sanitize_model_visible_text(value);
    if sanitized.len() <= max_bytes {
        return sanitized;
    }
    let mut end = max_bytes;
    while end > 0 && !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].to_string()
}

fn driver_note_kind_to_live_work_summary_phase(
    kind: LoopDriverNoteKind,
) -> ThreadLiveWorkSummaryPhase {
    match kind {
        LoopDriverNoteKind::Planning => ThreadLiveWorkSummaryPhase::Planning,
        LoopDriverNoteKind::Waiting => ThreadLiveWorkSummaryPhase::Waiting,
        LoopDriverNoteKind::Retrying => ThreadLiveWorkSummaryPhase::Retrying,
        LoopDriverNoteKind::Context | LoopDriverNoteKind::EventSubscriptionTerminated => {
            ThreadLiveWorkSummaryPhase::Context
        }
    }
}
