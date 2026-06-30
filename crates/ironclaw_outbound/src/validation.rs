use std::collections::HashSet;

use ironclaw_turns::ReplyTargetBindingRef;

use crate::{
    AdvanceSubscriptionCursorRequest, CommunicationPreferenceRecord, DeliveryDefaultScope,
    DeliveryFailureKind, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundDeliveryStatus, OutboundError, ProjectionSubscriptionRecord, ThreadNotificationPolicy,
    UpdateDeliveryStatusRequest,
};

const MAX_NOTIFICATION_TARGETS: usize = 32;

pub(crate) fn validate_policy(policy: &ThreadNotificationPolicy) -> Result<(), OutboundError> {
    if policy.targets.len() > MAX_NOTIFICATION_TARGETS {
        return Err(OutboundError::InvalidRequest {
            reason: "notification policy has too many targets",
        });
    }
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
    validate_subscription_cursor_progression(existing.cursor.as_ref(), incoming.cursor.as_ref())
}

pub(crate) fn validate_subscription_cursor_progression(
    current: Option<&ironclaw_event_projections::ProjectionCursor>,
    incoming: Option<&ironclaw_event_projections::ProjectionCursor>,
) -> Result<(), OutboundError> {
    match (current, incoming) {
        (Some(_), None) => Err(OutboundError::InvalidRequest {
            reason: "subscription cursor must not be cleared",
        }),
        (Some(current), Some(incoming)) if incoming.runtime < current.runtime => {
            Err(OutboundError::InvalidRequest {
                reason: "subscription cursor must not move backwards",
            })
        }
        _ => Ok(()),
    }
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
    validate_subscription_cursor_progression(record.cursor.as_ref(), Some(&request.cursor))
}

pub(crate) fn validate_delivery_attempt(
    attempt: &OutboundDeliveryAttempt,
) -> Result<(), OutboundError> {
    validate_delivery_scope_candidate(&attempt.scope, &attempt.candidate)?;
    validate_delivery_status(attempt.status, attempt.failure_kind)
}

pub(crate) fn validate_delivery_scope_candidate(
    scope: &ironclaw_turns::TurnScope,
    candidate: &crate::OutboundPushCandidate,
) -> Result<(), OutboundError> {
    if scope.tenant_id != candidate.tenant_id
        || scope.agent_id != candidate.agent_id
        || scope.project_id != candidate.project_id
        || scope.thread_id != candidate.thread_id
    {
        return Err(OutboundError::InvalidRequest {
            reason: "delivery candidate scope does not match request scope",
        });
    }
    Ok(())
}

pub(crate) fn validate_delivery_status_request(
    request: &UpdateDeliveryStatusRequest,
) -> Result<(), OutboundError> {
    validate_delivery_status(request.status, request.failure_kind)
}

fn validate_delivery_status(
    status: OutboundDeliveryStatus,
    failure_kind: Option<DeliveryFailureKind>,
) -> Result<(), OutboundError> {
    match (status, failure_kind) {
        (OutboundDeliveryStatus::Pending | OutboundDeliveryStatus::Delivered, None) => Ok(()),
        (OutboundDeliveryStatus::Failed | OutboundDeliveryStatus::DeadLettered, Some(_)) => Ok(()),
        (OutboundDeliveryStatus::Pending | OutboundDeliveryStatus::Delivered, Some(_)) => {
            Err(OutboundError::InvalidRequest {
                reason: "successful delivery statuses must not include failure kind",
            })
        }
        (OutboundDeliveryStatus::Failed | OutboundDeliveryStatus::DeadLettered, None) => {
            Err(OutboundError::InvalidRequest {
                reason: "failed delivery statuses require failure kind",
            })
        }
    }
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

pub(crate) fn validate_communication_preference(
    record: &CommunicationPreferenceRecord,
) -> Result<(), OutboundError> {
    match &record.scope {
        DeliveryDefaultScope::Personal { tenant_id, user_id } => {
            if tenant_id.as_str().is_empty() {
                return Err(OutboundError::InvalidRequest {
                    reason: "communication preference tenant is required",
                });
            }
            if user_id.as_str().is_empty() {
                return Err(OutboundError::InvalidRequest {
                    reason: "communication preference user is required",
                });
            }
        }
        DeliveryDefaultScope::SharedAgent {
            tenant_id,
            agent_id,
            project_id,
        } => {
            if tenant_id.as_str().is_empty() {
                return Err(OutboundError::InvalidRequest {
                    reason: "communication preference tenant is required",
                });
            }
            if agent_id.as_str().is_empty() {
                return Err(OutboundError::InvalidRequest {
                    reason: "communication preference shared agent is required",
                });
            }
            if project_id
                .as_ref()
                .is_some_and(|project_id| project_id.as_str().is_empty())
            {
                return Err(OutboundError::InvalidRequest {
                    reason: "communication preference project is required when present",
                });
            }
        }
    }
    if record.updated_by.as_str().is_empty() {
        return Err(OutboundError::InvalidRequest {
            reason: "communication preference updater is required",
        });
    }
    Ok(())
}
