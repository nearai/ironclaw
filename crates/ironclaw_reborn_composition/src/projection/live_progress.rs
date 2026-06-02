use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use ironclaw_event_projections::{
    ProjectionCursor as EventProjectionCursor, ProjectionScope as EventProjectionScope,
};
use ironclaw_event_streams::{
    InMemoryProjectionUpdateSource, ProductProjectionEnvelope, ThreadLiveProjectionItem,
    ThreadLiveProjectionUpdate, ThreadLiveWorkSummaryPhase,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_first_party_extension_ports::{SkillActivationObservedEvent, SkillActivationObserver};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{
    PROJECTION_SKILL_ACTIVATION_MAX_ITEMS, PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
    PROJECTION_SKILL_NAME_MAX_BYTES, ProductProjectionItem, ProductWorkSummaryPhase,
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

pub(super) struct LiveProgressMilestoneSink {
    inner: Arc<dyn LoopHostMilestoneSink>,
    publisher: Arc<LiveProjectionPublisher>,
}

#[derive(Debug)]
pub(super) struct LiveSkillActivationObserver {
    publisher: Arc<LiveProjectionPublisher>,
}

pub(crate) struct LiveProjectionPublisher {
    update_source: Arc<InMemoryProjectionUpdateSource>,
    actor_user_id: UserId,
    next_sequence: AtomicU64,
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
        Self { inner, publisher }
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
    ) -> Self {
        Self {
            update_source,
            actor_user_id,
            next_sequence: AtomicU64::new(0),
        }
    }

    fn next_live_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn publish_live_item(&self, scope: &TurnScope, sequence: u64, item: ThreadLiveProjectionItem) {
        let cursor = EventProjectionCursor::for_scope(
            self.projection_scope(scope),
            EventCursor::new(LIVE_PROGRESS_CURSOR_BASE.saturating_add(sequence)),
        );
        let update = ThreadLiveProjectionUpdate {
            cursor,
            thread_id: scope.thread_id.clone(),
            items: vec![item],
        };
        if let Err(error) = self
            .update_source
            .publish(ProductProjectionEnvelope::ThreadLiveUpdate(update))
        {
            tracing::debug!(
                error = %error,
                "failed to publish live progress projection"
            );
        }
    }

    fn projection_scope(&self, scope: &TurnScope) -> EventProjectionScope {
        EventProjectionScope {
            stream: EventStreamKey::new(
                scope.tenant_id.clone(),
                self.actor_user_id.clone(),
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
    update: &ThreadLiveProjectionUpdate,
) -> Vec<ProductProjectionItem> {
    update
        .items
        .iter()
        .map(|item| match item {
            ThreadLiveProjectionItem::Thinking { id, body } => ProductProjectionItem::Thinking {
                id: id.clone(),
                body: body.clone(),
            },
            ThreadLiveProjectionItem::WorkSummary {
                id,
                run_id,
                phase,
                body,
            } => ProductProjectionItem::WorkSummary {
                id: id.clone(),
                run_id: *run_id,
                phase: live_work_summary_phase_to_product_phase(*phase),
                body: body.clone(),
            },
            ThreadLiveProjectionItem::SkillActivation {
                id,
                run_id,
                skill_names,
                feedback,
            } => ProductProjectionItem::SkillActivation {
                id: id.clone(),
                run_id: *run_id,
                skill_names: skill_names.clone(),
                feedback: feedback.clone(),
            },
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
            &milestone.scope,
            sequence,
            ThreadLiveProjectionItem::Thinking {
                id: thinking_id(milestone.run_id, sequence),
                body: safe_delta,
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
        match &milestone.kind {
            LoopHostMilestoneKind::ModelReasoningDelta { safe_delta } => {
                self.publish_reasoning_delta(&milestone, safe_delta);
            }
            LoopHostMilestoneKind::DriverNote { kind, safe_summary } => {
                self.publish_work_summary(&milestone, *kind, safe_summary.as_str());
            }
            _ => {}
        }
        Ok(())
    }
}

fn thinking_id(run_id: TurnRunId, sequence: u64) -> String {
    format!("thinking:{run_id}:{sequence}")
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
