//! [`OutboundDeliverySink`] implementation that records delivery outcomes to
//! [`OutboundStateStore`].
//!
//! The sink is constructed per-render-call with a fixed `TurnScope` and
//! `ThreadId`, then handed to `ProductAdapter::render_outbound`. When the
//! adapter calls `record(DeliveryStatus)`, the sink persists an
//! `OutboundDeliveryAttempt` row reflecting the terminal outcome.
//!
//! No new schema — reuses `crates/ironclaw_outbound/`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use ironclaw_outbound::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryStatus, OutboundPushCandidate,
    OutboundPushKind, OutboundStateStore, ProjectionUpdateRef,
};
use ironclaw_product_adapters::{DeliveryStatus, OutboundDeliverySink};
use ironclaw_turns::TurnScope;

/// Sink that translates adapter `DeliveryStatus` callbacks into
/// `OutboundStateStore` writes. Logs (does not propagate) store errors so a
/// successful HTTP delivery is never undone by a metadata write failure.
pub struct OutboundStateStoreDeliverySink {
    store: Arc<dyn OutboundStateStore>,
    scope: TurnScope,
    thread_id: ThreadId,
    projection_ref: ProjectionUpdateRef,
}

impl OutboundStateStoreDeliverySink {
    pub fn new(
        store: Arc<dyn OutboundStateStore>,
        scope: TurnScope,
        thread_id: ThreadId,
        projection_ref: ProjectionUpdateRef,
    ) -> Self {
        Self {
            store,
            scope,
            thread_id,
            projection_ref,
        }
    }
}

fn translate_status(
    status: &DeliveryStatus,
) -> (OutboundDeliveryStatus, Option<DeliveryFailureKind>) {
    match status {
        DeliveryStatus::Delivered { .. } => (OutboundDeliveryStatus::Delivered, None),
        DeliveryStatus::FailedRetryable { .. } => (
            OutboundDeliveryStatus::Failed,
            Some(DeliveryFailureKind::TransportUnavailable),
        ),
        DeliveryStatus::FailedPermanent { .. } => (
            OutboundDeliveryStatus::DeadLettered,
            Some(DeliveryFailureKind::Rejected),
        ),
        DeliveryStatus::FailedUnauthorized { .. } => (
            OutboundDeliveryStatus::DeadLettered,
            Some(DeliveryFailureKind::AuthorizationRevoked),
        ),
        DeliveryStatus::Deferred { .. } => (
            OutboundDeliveryStatus::Pending,
            Some(DeliveryFailureKind::RateLimited),
        ),
    }
}

#[async_trait]
impl OutboundDeliverySink for OutboundStateStoreDeliverySink {
    async fn record(&self, status: DeliveryStatus) {
        let (mapped_status, failure_kind) = translate_status(&status);
        let (target, run_id) = match &status {
            DeliveryStatus::Delivered { target, run_id, .. }
            | DeliveryStatus::FailedRetryable { target, run_id, .. }
            | DeliveryStatus::FailedPermanent { target, run_id, .. }
            | DeliveryStatus::FailedUnauthorized { target, run_id, .. }
            | DeliveryStatus::Deferred { target, run_id, .. } => (target.clone(), *run_id),
        };

        let attempt = OutboundDeliveryAttempt {
            delivery_id: ironclaw_outbound::OutboundDeliveryId::from_uuid(status.attempt_id()),
            scope: self.scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: self.scope.tenant_id.clone(),
                agent_id: self.scope.agent_id.clone(),
                project_id: self.scope.project_id.clone(),
                thread_id: self.thread_id.clone(),
                turn_run_id: run_id,
                target,
                kind: OutboundPushKind::FinalReply,
                projection_ref: self.projection_ref.clone(),
                requires_reply_target_revalidation: false,
            },
            status: mapped_status,
            attempted_at: chrono::Utc::now(),
            failure_kind,
        };

        if let Err(err) = self.store.record_delivery_attempt(attempt).await {
            // Persist failure must not unwind a successful HTTP send. Log and
            // continue; reconciliation jobs can repair audit state later.
            tracing::warn!(
                error = %err,
                "OutboundStateStoreDeliverySink: failed to record delivery attempt"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, TenantId};
    use ironclaw_outbound::InMemoryOutboundStateStore;
    use ironclaw_product_adapters::DeliveryStatus;
    use ironclaw_turns::ReplyTargetBindingRef;
    use uuid::Uuid;

    fn thread() -> ThreadId {
        ThreadId::new("thread_t1").expect("thread")
    }

    fn scope() -> TurnScope {
        TurnScope {
            tenant_id: TenantId::new("tenant_default").expect("tenant"),
            agent_id: Some(AgentId::new("agent_default").expect("agent")),
            project_id: None,
            thread_id: thread(),
        }
    }

    fn projection() -> ProjectionUpdateRef {
        ProjectionUpdateRef::new("tracer:no-projection").expect("ref")
    }

    #[tokio::test]
    async fn delivered_status_records_attempt() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let sink =
            OutboundStateStoreDeliverySink::new(store.clone(), scope(), thread(), projection());
        sink.record(DeliveryStatus::Delivered {
            attempt_id: Uuid::new_v4(),
            target: ReplyTargetBindingRef::new("tg:123:_:_").expect("target"),
            run_id: None,
        })
        .await;

        let attempts = store.list_delivery_attempts(scope()).await.expect("list");
        assert_eq!(attempts.len(), 1);
        assert!(matches!(
            attempts[0].status,
            OutboundDeliveryStatus::Delivered
        ));
    }

    #[tokio::test]
    async fn failed_unauthorized_records_dead_letter() {
        let store = Arc::new(InMemoryOutboundStateStore::default());
        let sink =
            OutboundStateStoreDeliverySink::new(store.clone(), scope(), thread(), projection());
        sink.record(DeliveryStatus::FailedUnauthorized {
            attempt_id: Uuid::new_v4(),
            target: ReplyTargetBindingRef::new("tg:456:_:_").expect("target"),
            run_id: None,
            reason: ironclaw_product_adapters::RedactedString::new("403 forbidden"),
        })
        .await;

        let attempts = store.list_delivery_attempts(scope()).await.expect("list");
        assert_eq!(attempts.len(), 1);
        assert!(matches!(
            attempts[0].status,
            OutboundDeliveryStatus::DeadLettered
        ));
        assert!(matches!(
            attempts[0].failure_kind,
            Some(DeliveryFailureKind::AuthorizationRevoked)
        ));
    }
}
