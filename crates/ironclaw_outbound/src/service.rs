use async_trait::async_trait;

use crate::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryId,
    OutboundDeliveryStatus, OutboundError, OutboundStateStore, PrepareOutboundDeliveryRequest,
    ProjectionSubscriptionRecord, ProjectionSubscriptionRequest, ReplyTargetValidationRequest,
    ThreadProjectionAccessGrant, ThreadProjectionAccessRequest, ValidatedReplyTargetBinding,
};

#[async_trait]
pub trait ThreadProjectionAccessPolicy: Send + Sync {
    async fn authorize_projection_access(
        &self,
        request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessGrant, OutboundError>;
}

#[async_trait]
pub trait ReplyTargetBindingValidator: Send + Sync {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ValidatedReplyTargetBinding, OutboundError>;
}

pub struct OutboundPolicyService<'a> {
    store: &'a dyn OutboundStateStore,
    projection_access_policy: &'a dyn ThreadProjectionAccessPolicy,
    reply_target_validator: &'a dyn ReplyTargetBindingValidator,
}

impl<'a> OutboundPolicyService<'a> {
    pub fn new(
        store: &'a dyn OutboundStateStore,
        projection_access_policy: &'a dyn ThreadProjectionAccessPolicy,
        reply_target_validator: &'a dyn ReplyTargetBindingValidator,
    ) -> Self {
        Self {
            store,
            projection_access_policy,
            reply_target_validator,
        }
    }

    pub async fn authorize_subscription(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<ProjectionSubscriptionRecord, OutboundError> {
        let grant = self
            .projection_access_policy
            .authorize_projection_access(ThreadProjectionAccessRequest {
                actor: request.actor.clone(),
                scope: request.scope.clone(),
                thread_id: request.thread_id.clone(),
            })
            .await?;
        validate_access_grant(&request, &grant)?;

        let record = ProjectionSubscriptionRecord {
            subscription_id: request.subscription_id,
            actor: grant.actor,
            scope: grant.scope,
            thread_id: grant.thread_id,
            cursor: request.after_cursor,
        };
        self.store.upsert_subscription(record.clone()).await?;
        Ok(record)
    }

    pub async fn prepare_delivery_attempt(
        &self,
        request: PrepareOutboundDeliveryRequest,
    ) -> Result<OutboundDeliveryDecision, OutboundError> {
        if !request.candidate.requires_reply_target_revalidation {
            return Err(OutboundError::InvalidRequest {
                reason: "outbound push candidate must require reply target revalidation",
            });
        }
        if request.scope.thread_id != request.candidate.thread_id {
            return Err(OutboundError::InvalidRequest {
                reason: "delivery candidate thread does not match scope",
            });
        }

        let validation = self
            .reply_target_validator
            .validate_reply_target(ReplyTargetValidationRequest {
                scope: request.scope.clone(),
                candidate: request.candidate.clone(),
            })
            .await;

        match validation {
            Ok(target) => {
                if target.target != request.candidate.target {
                    return Err(OutboundError::InvalidRequest {
                        reason: "validated reply target does not match push candidate",
                    });
                }
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Pending,
                    attempted_at: request.attempted_at,
                    failure_kind: None,
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Authorized { attempt, target })
            }
            Err(OutboundError::AccessDenied) => {
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Failed,
                    attempted_at: request.attempted_at,
                    failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Rejected { attempt })
            }
            Err(error) => Err(error),
        }
    }
}

fn validate_access_grant(
    request: &ProjectionSubscriptionRequest,
    grant: &ThreadProjectionAccessGrant,
) -> Result<(), OutboundError> {
    if request.actor != grant.actor
        || request.scope != grant.scope
        || request.thread_id != grant.thread_id
    {
        return Err(OutboundError::InvalidRequest {
            reason: "projection access grant does not match subscription request",
        });
    }
    Ok(())
}
