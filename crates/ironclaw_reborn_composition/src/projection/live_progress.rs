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
use ironclaw_host_api::UserId;
use ironclaw_turns::{
    TurnRunId,
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
    update_source: Arc<InMemoryProjectionUpdateSource>,
    actor_user_id: UserId,
    next_sequence: AtomicU64,
}

impl LiveProgressMilestoneSink {
    pub(super) fn new(
        inner: Arc<dyn LoopHostMilestoneSink>,
        update_source: Arc<InMemoryProjectionUpdateSource>,
        actor_user_id: UserId,
    ) -> Self {
        Self {
            inner,
            update_source,
            actor_user_id,
            next_sequence: AtomicU64::new(0),
        }
    }

    fn next_live_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn publish_live_item(
        &self,
        milestone: &LoopHostMilestone,
        sequence: u64,
        item: ThreadLiveProjectionItem,
    ) {
        let cursor = EventProjectionCursor::for_scope(
            self.projection_scope(milestone),
            EventCursor::new(LIVE_PROGRESS_CURSOR_BASE.saturating_add(sequence)),
        );
        let update = ThreadLiveProjectionUpdate {
            cursor,
            thread_id: milestone.scope.thread_id.clone(),
            items: vec![item],
        };
        if let Err(error) = self
            .update_source
            .publish(ProductProjectionEnvelope::ThreadLiveUpdate(update))
        {
            tracing::debug!(
                error = %error,
                run_id = %milestone.run_id,
                "failed to publish live progress projection"
            );
        }
    }

    fn publish_reasoning_delta(&self, milestone: &LoopHostMilestone, safe_delta: &str) {
        // The delta is already model-visible sanitized upstream. Re-sanitize at
        // the product projection boundary so this publish path has its own
        // last-mile redaction gate before sending a browser-facing payload.
        let safe_delta = sanitize_model_visible_text(safe_delta);
        if safe_delta.is_empty() {
            return;
        }
        let sequence = self.next_live_sequence();
        self.publish_live_item(
            milestone,
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
        let sequence = self.next_live_sequence();
        self.publish_live_item(
            milestone,
            sequence,
            ThreadLiveProjectionItem::WorkSummary {
                id: work_summary_id(milestone.run_id, sequence),
                run_id: milestone.run_id,
                phase: driver_note_kind_to_live_work_summary_phase(kind),
                body,
            },
        );
    }

    fn projection_scope(&self, milestone: &LoopHostMilestone) -> EventProjectionScope {
        EventProjectionScope {
            stream: EventStreamKey::new(
                milestone.scope.tenant_id.clone(),
                self.actor_user_id.clone(),
                milestone.scope.agent_id.clone(),
            ),
            read_scope: ReadScope {
                project_id: milestone.scope.project_id.clone(),
                mission_id: None,
                thread_id: Some(milestone.scope.thread_id.clone()),
                process_id: None,
            },
        }
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
