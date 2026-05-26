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
    ThreadLiveProjectionUpdate,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::UserId;
use ironclaw_turns::{
    TurnRunId,
    run_profile::{
        AgentLoopHostError, LoopHostMilestone, LoopHostMilestoneKind, LoopHostMilestoneSink,
        sanitize_model_visible_text,
    },
};

// Live reasoning uses a synthetic cursor because it is an ephemeral UI hint,
// not a durable runtime event. This sink must remain the only producer on this
// `InMemoryProjectionUpdateSource`: mixing durable `ThreadUpdates` into the
// same live broadcast would put low append-log cursors and high synthetic
// cursors behind the same `last_delivered_cursor` ordering gate.
const LIVE_REASONING_CURSOR_BASE: u64 = 1 << 62;

pub(super) struct LiveReasoningMilestoneSink {
    inner: Arc<dyn LoopHostMilestoneSink>,
    update_source: Arc<InMemoryProjectionUpdateSource>,
    actor_user_id: UserId,
    next_sequence: AtomicU64,
}

impl LiveReasoningMilestoneSink {
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

    fn publish_reasoning_delta(&self, milestone: &LoopHostMilestone, safe_delta: &str) {
        // The delta is already model-visible sanitized upstream. Re-sanitize at
        // the product projection boundary so this publish path has its own
        // last-mile redaction gate before sending a browser-facing payload.
        let safe_delta = sanitize_model_visible_text(safe_delta);
        if safe_delta.is_empty() {
            return;
        }
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let cursor = EventProjectionCursor::for_scope(
            self.projection_scope(milestone),
            EventCursor::new(LIVE_REASONING_CURSOR_BASE.saturating_add(sequence)),
        );
        let update = ThreadLiveProjectionUpdate {
            cursor,
            thread_id: milestone.scope.thread_id.clone(),
            items: vec![ThreadLiveProjectionItem::Thinking {
                id: thinking_id(milestone.run_id, sequence),
                body: safe_delta,
            }],
        };
        if let Err(error) = self
            .update_source
            .publish(ProductProjectionEnvelope::ThreadLiveUpdate(update))
        {
            tracing::debug!(
                error = %error,
                run_id = %milestone.run_id,
                "failed to publish model reasoning projection"
            );
        }
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
impl LoopHostMilestoneSink for LiveReasoningMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.publish_loop_milestone(milestone.clone()).await?;
        if let LoopHostMilestoneKind::ModelReasoningDelta { safe_delta } = &milestone.kind {
            self.publish_reasoning_delta(&milestone, safe_delta);
        }
        Ok(())
    }
}

fn thinking_id(run_id: TurnRunId, sequence: u64) -> String {
    format!("thinking:{run_id}:{sequence}")
}
