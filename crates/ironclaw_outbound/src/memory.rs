use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionCursor;
use ironclaw_turns::TurnScope;

use crate::validation::{
    validate_advance_request, validate_delivery_attempt, validate_delivery_identity,
    validate_policy, validate_subscription_identity, validate_subscription_record,
    validate_subscription_request,
};
use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundDeliveryId, OutboundError, OutboundStateStore, ProjectionSubscriptionId,
    ProjectionSubscriptionRecord, ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

#[derive(Default)]
pub struct InMemoryOutboundStateStore {
    state: Mutex<InMemoryOutboundState>,
}

#[derive(Default)]
struct InMemoryOutboundState {
    policies: HashMap<ThreadScopeKey, ThreadNotificationPolicy>,
    subscriptions: HashMap<ProjectionSubscriptionId, ProjectionSubscriptionRecord>,
    deliveries: HashMap<OutboundDeliveryId, OutboundDeliveryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ThreadScopeKey {
    tenant_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    thread_id: String,
}

impl ThreadScopeKey {
    fn new(scope: &TurnScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            agent_id: scope.agent_id.as_ref().map(ToString::to_string),
            project_id: scope.project_id.as_ref().map(ToString::to_string),
            thread_id: scope.thread_id.to_string(),
        }
    }
}

#[async_trait]
impl OutboundStateStore for InMemoryOutboundStateStore {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        let mut state = self.lock_state()?;
        state
            .policies
            .insert(ThreadScopeKey::new(&policy.scope), policy);
        Ok(())
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        let state = self.lock_state()?;
        Ok(state
            .policies
            .get(&ThreadScopeKey::new(&scope))
            .cloned()
            .unwrap_or_else(|| ThreadNotificationPolicy::default_for_scope(scope)))
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        let mut state = self.lock_state()?;
        if let Some(existing) = state.subscriptions.get(&record.subscription_id) {
            validate_subscription_identity(existing, &record)?;
        }
        state
            .subscriptions
            .insert(record.subscription_id.clone(), record);
        Ok(())
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        let state = self.lock_state()?;
        let Some(record) = state.subscriptions.get(&request.subscription_id) else {
            return Ok(None);
        };
        validate_subscription_request(record, &request)?;
        Ok(record.cursor.clone())
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        let mut state = self.lock_state()?;
        let Some(record) = state.subscriptions.get_mut(&request.subscription_id) else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(record, &request)?;
        record.cursor = Some(request.cursor);
        Ok(())
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        let mut state = self.lock_state()?;
        if let Some(existing) = state.deliveries.get(&attempt.delivery_id) {
            validate_delivery_identity(existing, &attempt)?;
            return Ok(());
        }
        state.deliveries.insert(attempt.delivery_id, attempt);
        Ok(())
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        let _updated_at = request.updated_at;
        let mut state = self.lock_state()?;
        let Some(attempt) = state.deliveries.get_mut(&request.delivery_id) else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        Ok(())
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        let state = self.lock_state()?;
        let key = ThreadScopeKey::new(&scope);
        let mut deliveries = state
            .deliveries
            .values()
            .filter(|attempt| ThreadScopeKey::new(&attempt.scope) == key)
            .cloned()
            .collect::<Vec<_>>();
        deliveries.sort_by_key(|attempt| (attempt.attempted_at, attempt.delivery_id.to_string()));
        Ok(deliveries)
    }
}

impl InMemoryOutboundStateStore {
    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, InMemoryOutboundState>, OutboundError> {
        self.state.lock().map_err(|_| OutboundError::Backend)
    }
}
