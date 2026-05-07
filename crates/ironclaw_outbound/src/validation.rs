use std::collections::HashSet;

use ironclaw_turns::ReplyTargetBindingRef;

use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundError, ProjectionSubscriptionRecord, ThreadNotificationPolicy,
};

pub(crate) fn validate_policy(policy: &ThreadNotificationPolicy) -> Result<(), OutboundError> {
    let mut seen = HashSet::<ReplyTargetBindingRef>::new();
    for target in &policy.targets {
        if !target.final_replies && !target.progress {
            return Err(OutboundError::InvalidRequest {
                reason: "notification target must enable at least one push kind",
            });
        }
        if !seen.insert(target.target.clone()) {
            return Err(OutboundError::InvalidRequest {
                reason: "duplicate notification target",
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_subscription_record(
    record: &ProjectionSubscriptionRecord,
) -> Result<(), OutboundError> {
    let Some(thread_id) = record.scope.read_scope.thread_id.as_ref() else {
        return Err(OutboundError::InvalidRequest {
            reason: "subscription scope must be thread-scoped",
        });
    };
    if thread_id != &record.thread_id || record.actor.user_id != record.scope.stream.user_id {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    if let Some(cursor) = record.cursor.as_ref()
        && cursor.scope != record.scope
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

pub(crate) fn validate_subscription_request(
    record: &ProjectionSubscriptionRecord,
    request: &LoadSubscriptionCursorRequest,
) -> Result<(), OutboundError> {
    if record.subscription_id != request.subscription_id
        || record.actor != request.actor
        || record.scope != request.scope
        || record.thread_id != request.thread_id
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}
pub(crate) fn validate_subscription_identity(
    existing: &ProjectionSubscriptionRecord,
    incoming: &ProjectionSubscriptionRecord,
) -> Result<(), OutboundError> {
    if existing.subscription_id != incoming.subscription_id
        || existing.actor != incoming.actor
        || existing.scope != incoming.scope
        || existing.thread_id != incoming.thread_id
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

pub(crate) fn validate_advance_request(
    record: &ProjectionSubscriptionRecord,
    request: &AdvanceSubscriptionCursorRequest,
) -> Result<(), OutboundError> {
    if record.subscription_id != request.subscription_id
        || record.actor != request.actor
        || record.thread_id != request.thread_id
        || record.scope != request.cursor.scope
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

pub(crate) fn validate_delivery_attempt(
    attempt: &OutboundDeliveryAttempt,
) -> Result<(), OutboundError> {
    if attempt.scope.thread_id != attempt.candidate.thread_id {
        return Err(OutboundError::InvalidRequest {
            reason: "delivery candidate thread does not match scope",
        });
    }
    Ok(())
}

pub(crate) fn validate_delivery_identity(
    existing: &OutboundDeliveryAttempt,
    incoming: &OutboundDeliveryAttempt,
) -> Result<(), OutboundError> {
    if existing.delivery_id != incoming.delivery_id
        || existing.scope != incoming.scope
        || existing.candidate != incoming.candidate
        || existing.attempted_at != incoming.attempted_at
    {
        return Err(OutboundError::Backend);
    }
    Ok(())
}
