use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_host_api::{ThreadId, Timestamp};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::{OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPushKind {
    FinalReply,
    Progress,
    GateRequired,
    DeliveryStatus,
}

impl OutboundPushKind {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FinalReply => "final_reply",
            Self::Progress => "progress",
            Self::GateRequired => "gate_required",
            Self::DeliveryStatus => "delivery_status",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationTarget {
    pub target: ReplyTargetBindingRef,
    pub final_replies: bool,
    pub progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationPolicy {
    pub scope: TurnScope,
    pub targets: Vec<ThreadNotificationTarget>,
}

impl ThreadNotificationPolicy {
    pub fn default_for_scope(scope: TurnScope) -> Self {
        Self {
            scope,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushTargetRequest {
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    pub reply_target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushCandidate {
    pub thread_id: ThreadId,
    pub turn_run_id: Option<TurnRunId>,
    pub target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
    pub requires_reply_target_revalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushPlan {
    pub candidates: Vec<OutboundPushCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadProjectionAccessRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
}

/// Untrusted access decision returned by a [`ThreadProjectionAccessPolicy`]
/// implementation. Only the [`OutboundPolicyService`] mints the sealed
/// [`ThreadProjectionAccessGrant`] from this claim after cross-checking the
/// request, so policy implementors cannot forge a grant by constructing one
/// directly.
///
/// [`ThreadProjectionAccessPolicy`]: crate::ThreadProjectionAccessPolicy
/// [`OutboundPolicyService`]: crate::OutboundPolicyService
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadProjectionAccessClaim {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
}

/// Trust-bearing record that the [`OutboundPolicyService`] has authorized a
/// projection subscription for a specific actor/scope/thread triple. Sealed
/// against external construction; obtain instances only by calling
/// [`OutboundPolicyService::authorize_subscription`].
///
/// [`OutboundPolicyService`]: crate::OutboundPolicyService
/// [`OutboundPolicyService::authorize_subscription`]: crate::OutboundPolicyService::authorize_subscription
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadProjectionAccessGrant {
    pub(crate) actor: TurnActor,
    pub(crate) scope: ProjectionScope,
    pub(crate) thread_id: ThreadId,
}

impl ThreadProjectionAccessGrant {
    pub(crate) fn from_claim(claim: ThreadProjectionAccessClaim) -> Self {
        Self {
            actor: claim.actor,
            scope: claim.scope,
            thread_id: claim.thread_id,
        }
    }

    pub fn actor(&self) -> &TurnActor {
        &self.actor
    }

    pub fn scope(&self) -> &ProjectionScope {
        &self.scope
    }

    pub fn thread_id(&self) -> &ThreadId {
        &self.thread_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRecord {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
    pub cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub thread_id: ThreadId,
    pub cursor: ProjectionCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundDeliveryStatus {
    Pending,
    Delivered,
    Failed,
    DeadLettered,
}

impl OutboundDeliveryStatus {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::DeadLettered => "dead_lettered",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryFailureKind {
    /// Permanent denial from the reply-target validator. Do not retry — the
    /// authorization that originally established this binding has been
    /// revoked or never existed.
    AuthorizationRevoked,
    /// Transient validator-side failure (backend, serialization, or other
    /// non-`AccessDenied` error). Callers may retry; the underlying validator
    /// or its dependency was unavailable at attempt time.
    TransientValidatorError,
    TransportUnavailable,
    RateLimited,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetValidationRequest {
    pub scope: TurnScope,
    pub candidate: OutboundPushCandidate,
}

/// Untrusted validator decision returned by a [`ReplyTargetBindingValidator`]
/// implementation. Only the [`OutboundPolicyService`] mints the sealed
/// [`ValidatedReplyTargetBinding`] from this claim after confirming the
/// claimed target matches the original push candidate, so validators cannot
/// forge a "validated" binding by constructing one directly.
///
/// [`ReplyTargetBindingValidator`]: crate::ReplyTargetBindingValidator
/// [`OutboundPolicyService`]: crate::OutboundPolicyService
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetBindingClaim {
    pub target: ReplyTargetBindingRef,
}

/// Trust-bearing record that the [`OutboundPolicyService`] has authorized a
/// push to a specific [`ReplyTargetBindingRef`] for the current attempt.
/// Sealed against external construction; obtain instances only by calling
/// [`OutboundPolicyService::prepare_delivery_attempt`], which performs the
/// claim/candidate target-equality check that prevents validator-supplied
/// target substitution.
///
/// [`OutboundPolicyService`]: crate::OutboundPolicyService
/// [`OutboundPolicyService::prepare_delivery_attempt`]: crate::OutboundPolicyService::prepare_delivery_attempt
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidatedReplyTargetBinding {
    pub(crate) target: ReplyTargetBindingRef,
}

impl ValidatedReplyTargetBinding {
    pub(crate) fn from_claim(claim: ReplyTargetBindingClaim) -> Self {
        Self {
            target: claim.target,
        }
    }

    pub fn target(&self) -> &ReplyTargetBindingRef {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareOutboundDeliveryRequest {
    pub scope: TurnScope,
    pub candidate: OutboundPushCandidate,
    pub attempted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundDeliveryAttempt {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub candidate: OutboundPushCandidate,
    pub status: OutboundDeliveryStatus,
    pub attempted_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundDeliveryDecision {
    Authorized {
        attempt: OutboundDeliveryAttempt,
        target: ValidatedReplyTargetBinding,
    },
    Rejected {
        attempt: OutboundDeliveryAttempt,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateDeliveryStatusRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub status: OutboundDeliveryStatus,
    pub updated_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}
