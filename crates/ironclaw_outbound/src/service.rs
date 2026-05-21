use async_trait::async_trait;

use crate::validation::validate_delivery_scope_candidate;
use crate::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryId,
    OutboundDeliveryStatus, OutboundError, OutboundStateStore, PrepareOutboundDeliveryRequest,
    ProjectionSubscriptionRecord, ProjectionSubscriptionRequest, ReplyTargetBindingClaim,
    ReplyTargetValidationRequest, ThreadProjectionAccessClaim, ThreadProjectionAccessGrant,
    ThreadProjectionAccessRequest, ValidatedReplyTargetBinding,
};

#[async_trait]
pub trait ThreadProjectionAccessPolicy: Send + Sync {
    /// Decide whether the request actor may subscribe to projections for the
    /// requested thread/scope. The returned [`ThreadProjectionAccessClaim`] is
    /// **untrusted** — the [`OutboundPolicyService`] mints the sealed
    /// [`ThreadProjectionAccessGrant`] only after verifying the claim's fields
    /// match the original request.
    async fn authorize_projection_access(
        &self,
        request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError>;
}

#[async_trait]
pub trait ReplyTargetBindingValidator: Send + Sync {
    /// Validate that the candidate's reply target binding is still authorized
    /// for the current scope. The returned [`ReplyTargetBindingClaim`] is
    /// **untrusted** — the [`OutboundPolicyService`] mints the sealed
    /// [`ValidatedReplyTargetBinding`] only after confirming the claim's
    /// target matches the original push candidate (no target substitution).
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError>;
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
        let claim = self
            .projection_access_policy
            .authorize_projection_access(ThreadProjectionAccessRequest {
                actor: request.actor.clone(),
                scope: request.scope.clone(),
                thread_id: request.thread_id.clone(),
            })
            .await?;
        validate_access_claim(&request, &claim)?;
        let grant = ThreadProjectionAccessGrant::from_claim(claim);

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
        validate_delivery_scope_candidate(&request.scope, &request.candidate)?;

        let validation = self
            .reply_target_validator
            .validate_reply_target(ReplyTargetValidationRequest {
                scope: request.scope.clone(),
                candidate: request.candidate.clone(),
            })
            .await;

        match validation {
            Ok(claim) => {
                claim.validate_against(&request.candidate)?;
                let target = ValidatedReplyTargetBinding::from_claim(claim);
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
            Err(error) if is_transient_validator_error(&error) => {
                let attempt = OutboundDeliveryAttempt {
                    delivery_id: OutboundDeliveryId::new(),
                    scope: request.scope,
                    candidate: request.candidate,
                    status: OutboundDeliveryStatus::Failed,
                    attempted_at: request.attempted_at,
                    failure_kind: Some(DeliveryFailureKind::TransientValidatorError),
                };
                self.store.record_delivery_attempt(attempt.clone()).await?;
                Ok(OutboundDeliveryDecision::Rejected { attempt })
            }
            Err(error) => Err(error),
        }
    }
}

fn validate_access_claim(
    request: &ProjectionSubscriptionRequest,
    claim: &ThreadProjectionAccessClaim,
) -> Result<(), OutboundError> {
    if request.actor != claim.actor
        || request.scope != claim.scope
        || request.thread_id != claim.thread_id
    {
        return Err(OutboundError::InvalidRequest {
            reason: "projection access claim does not match subscription request",
        });
    }
    Ok(())
}

/// Returns true when a non-`AccessDenied` validator error reflects a
/// transient infrastructure failure rather than a caller bug. Caller-bug
/// errors (e.g. `InvalidRequest`, `SubscriptionScopeMismatch`,
/// `DeliveryNotFound`) propagate to the caller so they are not silently
/// retried; backend/serialization failures become recorded delivery
/// attempts so the saga can retry without losing the audit trail.
fn is_transient_validator_error(error: &OutboundError) -> bool {
    match error {
        // `CasConflict` is the typed compare-and-swap failure from the
        // filesystem-backed store. It is internal to the store layer and the
        // store's bounded retry loop converts it to `Backend` before returning
        // to the service, so it should never reach this classification site —
        // but classify it as transient (retryable) for defence in depth.
        OutboundError::Backend | OutboundError::Serialization | OutboundError::CasConflict => true,
        OutboundError::InvalidRequest { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::AccessDenied
        | OutboundError::DeliveryNotFound => false,
    }
}
