use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_event_projections::ProjectionCursor;
use ironclaw_turns::{ReplyTargetBindingRef, TurnScope};

use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundError, OutboundPushCandidate, OutboundPushKind, OutboundPushPlan,
    OutboundPushTargetRequest, ProjectionSubscriptionRecord, ThreadNotificationPolicy,
    UpdateDeliveryStatusRequest,
};

#[async_trait]
pub trait OutboundStateStore: Send + Sync {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError>;

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError>;

    async fn plan_push_targets(
        &self,
        request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        let policy = self
            .load_thread_notification_policy(request.scope.clone())
            .await?;
        plan_push_targets_from_policy(request, &policy)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError>;

    /// Load a cursor only for the exact authorized actor/scope/thread tuple.
    ///
    /// Returns `Ok(None)` for missing rows and for rows with a mismatched
    /// actor/scope/thread. The indistinguishable `None` preserves
    /// anti-enumeration semantics: callers cannot learn whether a
    /// subscription id exists outside their authorized tuple.
    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError>;

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError>;

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError>;

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError>;

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError>;
}

fn plan_push_targets_from_policy(
    request: OutboundPushTargetRequest,
    policy: &ThreadNotificationPolicy,
) -> Result<OutboundPushPlan, OutboundError> {
    if policy.scope != request.scope {
        return Err(OutboundError::InvalidRequest {
            reason: "notification policy scope does not match request",
        });
    }

    let mut seen = HashSet::<ReplyTargetBindingRef>::new();
    let mut candidates = Vec::new();
    if request.kind == OutboundPushKind::FinalReply {
        push_candidate(
            &request,
            request.reply_target.clone(),
            &mut seen,
            &mut candidates,
        );
    }

    for target in &policy.targets {
        let allowed = match request.kind {
            OutboundPushKind::FinalReply => target.final_replies,
            OutboundPushKind::Progress
            | OutboundPushKind::GateRequired
            | OutboundPushKind::AuthPrompt
            | OutboundPushKind::DeliveryStatus => target.progress,
        };
        if allowed {
            push_candidate(&request, target.target.clone(), &mut seen, &mut candidates);
        }
    }
    Ok(OutboundPushPlan { candidates })
}

fn push_candidate(
    request: &OutboundPushTargetRequest,
    target: ReplyTargetBindingRef,
    seen: &mut HashSet<ReplyTargetBindingRef>,
    candidates: &mut Vec<OutboundPushCandidate>,
) {
    if !seen.insert(target.clone()) {
        return;
    }
    candidates.push(OutboundPushCandidate {
        tenant_id: request.scope.tenant_id.clone(),
        agent_id: request.scope.agent_id.clone(),
        project_id: request.scope.project_id.clone(),
        thread_id: request.scope.thread_id.clone(),
        turn_run_id: request.turn_run_id,
        target,
        kind: request.kind,
        projection_ref: request.projection_ref.clone(),
        requires_reply_target_revalidation: true,
    });
}
