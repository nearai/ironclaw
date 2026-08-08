use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use ironclaw_host_api::{
    Timestamp,
    ids::{AgentId, ProjectId, TenantId, ThreadId},
};
use serde::{Deserialize, Serialize};

use crate::delivery_resolution::{
    CommunicationDeliveryKind, CommunicationDeliveryResolutionRequest, CommunicationModality,
};
use crate::{OutboundDeliveryId, OutboundError, ProjectionSubscriptionId, ProjectionUpdateRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPushKind {
    FinalReply,
    Progress,
    GateRequired,
    AuthPrompt,
    DeliveryStatus,
    ModelDelivery,
}

impl From<CommunicationDeliveryKind> for OutboundPushKind {
    fn from(kind: CommunicationDeliveryKind) -> Self {
        match kind {
            CommunicationDeliveryKind::FinalReply => Self::FinalReply,
            CommunicationDeliveryKind::ProgressUpdate => Self::Progress,
            CommunicationDeliveryKind::ApprovalPrompt => Self::GateRequired,
            CommunicationDeliveryKind::AuthPrompt => Self::AuthPrompt,
            CommunicationDeliveryKind::DeliveryStatus => Self::DeliveryStatus,
            CommunicationDeliveryKind::ModelDelivery => Self::ModelDelivery,
        }
    }
}

#[allow(dead_code)] // retained for future debug/log surfaces — not yet wired
impl OutboundPushKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FinalReply => "final_reply",
            Self::Progress => "progress",
            Self::GateRequired => "gate_required",
            Self::AuthPrompt => "auth_prompt",
            Self::DeliveryStatus => "delivery_status",
            Self::ModelDelivery => "model_delivery",
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
    pub tenant_id: TenantId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
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
    /// Coordinator lifecycle: the attempt is persisted, no vendor egress has
    /// happened yet (crash here → safe to retry).
    Prepared,
    /// Coordinator lifecycle: vendor egress is in flight. An attempt found in
    /// this state after a crash becomes [`Self::Unknown`] — never blindly
    /// resent (the vendor may have accepted the message).
    Sending,
    /// Legacy pre-coordinator state (kept for persisted rows).
    Pending,
    Delivered,
    Failed,
    /// Terminal-ambiguous: the process died after possible vendor success.
    /// Resend only when a vendor idempotency key makes it provably safe.
    Unknown,
    DeadLettered,
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
    /// Unclassified failure — this codebase never wrote a more specific
    /// [`DeliveryFailureKind`] for this row. Contrast with
    /// [`Self::VendorContactAmbiguous`], which is a specific, deliberately
    /// classified kind: it does not mean "we don't know what happened", it
    /// means "we know retries were exhausted after the vendor-egress claim,
    /// and that state provides no proof the vendor was never contacted."
    Unknown,
    /// Retry exhaustion after the vendor-egress claim, settled without proof
    /// that no part of this delivery reached the vendor: either the adapter's
    /// `deliver` call returned a bare `Err` (no typed report at all), or it
    /// returned a typed report whose parts were all `Retryable` but retries
    /// were still exhausted. Both in-tree channel adapters (Slack, Telegram)
    /// use `Retryable` for post-send ambiguity (e.g. a timeout after the
    /// request was already sent), not just pre-send failure, so neither path
    /// proves the vendor was never contacted the way a preflight
    /// `TransportUnavailable`/`RateLimited`/`TransientValidatorError` does.
    ///
    /// This is distinct from [`Self::Unknown`]: `Unknown` means this codebase
    /// never assigned a specific kind to the row (an unclassified gap).
    /// `VendorContactAmbiguous` means the row *was* classified, precisely as
    /// "reached the point of no return and cannot prove the vendor wasn't
    /// contacted." A reopen must treat both as permanently terminal, but for
    /// different reasons — this variant exists so a reader auditing "why did
    /// this delivery id stay Failed forever" finds an explicit answer instead
    /// of an `Unknown` catch-all.
    VendorContactAmbiguous,
}

impl DeliveryFailureKind {
    /// Every [`DeliveryFailureKind`] variant, for exhaustiveness-style tests
    /// that need to iterate the type instead of hand-maintaining a literal
    /// array that silently drifts when a variant is added.
    pub const ALL: &'static [Self] = &[
        Self::AuthorizationRevoked,
        Self::TransientValidatorError,
        Self::TransportUnavailable,
        Self::RateLimited,
        Self::Rejected,
        Self::Unknown,
        Self::VendorContactAmbiguous,
    ];

    /// Whether a `Failed` row carrying this kind may be reopened to a fresh
    /// `Prepared` reservation under the same deterministic delivery id.
    /// True only for kinds this codebase provably never writes after the
    /// vendor-egress claim, so a reopen cannot duplicate an accepted send.
    pub const fn permits_reopen(self) -> bool {
        match self {
            Self::TransientValidatorError | Self::TransportUnavailable | Self::RateLimited => true,
            Self::AuthorizationRevoked
            | Self::Rejected
            | Self::Unknown
            | Self::VendorContactAmbiguous => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetValidationRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub modality: CommunicationModality,
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
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetBindingClaim {
    pub target: ReplyTargetBindingRef,
}

impl ReplyTargetBindingClaim {
    pub fn new(target: ReplyTargetBindingRef) -> Self {
        Self { target }
    }

    pub(crate) fn validate_against(
        &self,
        candidate: &OutboundPushCandidate,
    ) -> Result<(), OutboundError> {
        let Self { target } = self;
        if target != &candidate.target {
            return Err(OutboundError::InvalidRequest {
                reason: "validated reply target does not match push candidate",
            });
        }
        Ok(())
    }
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
        let ReplyTargetBindingClaim { target } = claim;
        Self { target }
    }

    pub fn target(&self) -> &ReplyTargetBindingRef {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareOutboundDeliveryRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub modality: CommunicationModality,
    pub candidate: OutboundPushCandidate,
    pub attempted_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareCommunicationDeliveryRequest {
    pub resolution_request: CommunicationDeliveryResolutionRequest,
    pub turn_run_id: Option<TurnRunId>,
    pub projection_ref: ProjectionUpdateRef,
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

/// Atomic ownership claim for the sole vendor-egress writer of a prepared
/// delivery. Stores transition `Prepared -> Sending` exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDeliveryAttemptForSendRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
}

/// Result of atomically claiming the sole vendor-egress drive for a prepared
/// delivery. `Existing` carries the exact row that blocked the claim, read
/// atomically inside the same CAS attempt that lost — never from a separate
/// subsequent read, which would reopen a TOCTOU window between the failed
/// CAS and the re-read (a concurrent caller could reopen the row in between,
/// and a claim-loser reading it afterward could misclassify a freshly
/// reopened `Prepared` row as still in flight).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDeliveryAttemptForSendOutcome {
    /// This caller persisted `Prepared -> Sending` and owns vendor egress.
    Claimed,
    /// The authoritative attempt no longer permits the transition.
    Existing(Box<OutboundDeliveryAttempt>),
}

/// Guarded crash-recovery transition for an interrupted send. The store
/// re-reads the attempt inside its own CAS and transitions `Sending -> Unknown`
/// only when it is still `Sending`. A stale recovery snapshot therefore cannot
/// overwrite a terminal result (`Delivered`/`Failed`) that a different worker
/// wrote after completing egress. Mirrors the `Prepared`-guard on
/// [`ClaimDeliveryAttemptForSendRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverInterruptedDeliveryRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
}

#[cfg(test)]
mod tests {
    use super::DeliveryFailureKind;

    /// G4: an exhaustive match in the test itself (not an array literal), so
    /// adding a future `DeliveryFailureKind` variant without updating this
    /// test is a compile error, not a silent gap.
    #[test]
    fn permits_reopen_is_exhaustively_classified() {
        for kind in DeliveryFailureKind::ALL {
            let expected = match kind {
                DeliveryFailureKind::TransientValidatorError
                | DeliveryFailureKind::TransportUnavailable
                | DeliveryFailureKind::RateLimited => true,
                DeliveryFailureKind::AuthorizationRevoked
                | DeliveryFailureKind::Rejected
                | DeliveryFailureKind::Unknown
                | DeliveryFailureKind::VendorContactAmbiguous => false,
            };
            assert_eq!(kind.permits_reopen(), expected, "{kind:?}");
        }
    }

    #[test]
    fn all_lists_every_variant_exactly_once() {
        // Defence-in-depth alongside the exhaustive match above: catches a
        // typo'd duplicate entry in `ALL` that the match wouldn't.
        let mut seen = std::collections::HashSet::new();
        for kind in DeliveryFailureKind::ALL {
            assert!(
                seen.insert(format!("{kind:?}")),
                "duplicate entry: {kind:?}"
            );
        }
        assert_eq!(seen.len(), 7);
    }
}
