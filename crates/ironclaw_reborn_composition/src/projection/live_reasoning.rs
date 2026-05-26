use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use ironclaw_loop_support::{HostManagedModelResponse, HostManagedModelResponseObserver};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductOutboundPayload, ProductProjectionItem, ProductProjectionState,
    ProductWorkflowRejectionKind, RedactedString,
};
use ironclaw_turns::{
    TurnRunId, TurnScope,
    run_profile::{LoopRunContext, sanitize_model_visible_text},
};

use super::internal_projection_error;

#[derive(Default)]
pub(super) struct WebuiLiveProjectionStore {
    inner: Mutex<WebuiLiveProjectionInner>,
}

#[derive(Default)]
struct WebuiLiveProjectionInner {
    next_sequence: u64,
    entries: VecDeque<WebuiLiveProjectionEntry>,
    last_evicted_by_scope: HashMap<TurnScope, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct WebuiLiveProjectionCursor {
    pub(super) scope: TurnScope,
    pub(super) sequence: u64,
}

#[derive(Clone)]
pub(super) struct WebuiLiveProjectionEntry {
    pub(super) cursor: WebuiLiveProjectionCursor,
    pub(super) payload: Arc<ProductOutboundPayload>,
}

impl WebuiLiveProjectionStore {
    pub(super) const MAX_ENTRIES: usize = 1024;

    pub(super) fn publish_reasoning_delta(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        reasoning_delta: &str,
    ) -> Result<(), ProductAdapterError> {
        let reasoning_delta = sanitize_model_visible_text(reasoning_delta);
        if reasoning_delta.is_empty() {
            return Ok(());
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| internal_projection_error("live lock"))?;
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        let sequence = inner.next_sequence;
        let state = ProductProjectionState::new(
            scope.thread_id.to_string(),
            vec![ProductProjectionItem::Thinking {
                id: format!("thinking:{run_id}:{sequence}"),
                body: reasoning_delta,
            }],
        )?;
        inner.entries.push_back(WebuiLiveProjectionEntry {
            cursor: WebuiLiveProjectionCursor { scope, sequence },
            payload: Arc::new(ProductOutboundPayload::ProjectionUpdate { state }),
        });
        while inner.entries.len() > Self::MAX_ENTRIES {
            if let Some(evicted) = inner.entries.pop_front() {
                inner
                    .last_evicted_by_scope
                    .insert(evicted.cursor.scope, evicted.cursor.sequence);
            }
        }
        Ok(())
    }

    pub(super) fn drain_after(
        &self,
        scope: &TurnScope,
        after: u64,
        limit: usize,
    ) -> Result<Vec<WebuiLiveProjectionEntry>, ProductAdapterError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| internal_projection_error("live lock"))?;
        if after > 0
            && inner
                .last_evicted_by_scope
                .get(scope)
                .is_some_and(|evicted| after <= *evicted)
        {
            return Err(ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Unavailable,
                status_code: 503,
                retryable: true,
                reason: RedactedString::new("live projection stream lagged; reconnect from origin"),
            });
        }
        Ok(inner
            .entries
            .iter()
            .filter(|entry| entry.cursor.sequence > after && &entry.cursor.scope == scope)
            .take(limit)
            .cloned()
            .collect())
    }
}

pub(super) struct ReasoningProjectionObserver {
    live_projections: Arc<WebuiLiveProjectionStore>,
}

impl ReasoningProjectionObserver {
    pub(super) fn new(live_projections: Arc<WebuiLiveProjectionStore>) -> Self {
        Self { live_projections }
    }
}

impl HostManagedModelResponseObserver for ReasoningProjectionObserver {
    fn observe_host_model_response(
        &self,
        run_context: &LoopRunContext,
        response: &HostManagedModelResponse,
    ) {
        for reasoning_delta in &response.safe_reasoning_deltas {
            if let Err(error) = self.live_projections.publish_reasoning_delta(
                run_context.scope.clone(),
                run_context.run_id,
                reasoning_delta,
            ) {
                tracing::debug!(
                    error = %error,
                    run_id = %run_context.run_id,
                    "failed to publish model reasoning projection"
                );
            }
        }
    }
}
