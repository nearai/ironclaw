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
}

impl From<CommunicationDeliveryKind> for OutboundPushKind {
    fn from(kind: CommunicationDeliveryKind) -> Self {
        match kind {
            CommunicationDeliveryKind::FinalReply => Self::FinalReply,
            CommunicationDeliveryKind::ProgressUpdate => Self::Progress,
            CommunicationDeliveryKind::ApprovalPrompt => Self::GateRequired,
            CommunicationDeliveryKind::AuthPrompt => Self::AuthPrompt,
            CommunicationDeliveryKind::DeliveryStatus => Self::DeliveryStatus,
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
    /// Coordinator lifecycle: after target/channel/context resolution and
    /// attachment materialization, durable sole-egress ownership was claimed
    /// immediately before adapter egress. This state marks vendor-contact
    /// ambiguity rather than proving contact. After a crash it becomes
    /// [`Self::Unknown`]; never blindly resend it.
    Sending,
    /// Legacy pre-coordinator state (kept for persisted rows).
    Pending,
    Delivered,
    Failed,
    /// Terminal-ambiguous: the process died after claiming egress ownership,
    /// either before vendor contact or after possible vendor success. Recovery
    /// cannot distinguish those cases. Resend only when a vendor idempotency
    /// key makes it provably safe.
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
    Unknown,
    /// Retry exhaustion after the vendor-egress claim, settled without proof
    /// that no part of this delivery reached the vendor. Two adapter
    /// outcomes reach this kind: a bare adapter `Err` from `deliver` that
    /// survived retry exhaustion with no typed delivery report ever
    /// returned, and a typed report whose parts are all `Retryable` but
    /// still exhausted retries. Unlike `TransportUnavailable`, `RateLimited`,
    /// and `TransientValidatorError` — which this delivery id only ever
    /// settles for when nothing reached the vendor — neither of these
    /// carries that proof: the channel adapter contract does not guarantee
    /// `Err` means "no vendor egress was attempted" for every
    /// implementation, and in-tree adapters may report post-send ambiguity
    /// (e.g. a timeout after the request was sent) as `Retryable`, so either
    /// path may have reached the vendor before settling. This is the same
    /// ambiguity `OutboundDeliveryStatus::Unknown` captures for crash
    /// recovery — "never blindly resend... unless a vendor idempotency key
    /// makes a resend provably safe" — and this codebase has no such key
    /// mechanism, so this kind is permanently terminal, exactly like
    /// `Unknown`.
    VendorContactAmbiguous,
}

impl DeliveryFailureKind {
    /// Returns whether this failure kind may settle the guarded
    /// `Prepared -> Failed` preflight transition.
    ///
    /// This classification is specific to failures discovered before vendor
    /// egress ownership is claimed. It does not classify delivery failures for
    /// retry policy outside that settlement boundary — see
    /// [`Self::is_permanent`] for that broader question.
    ///
    /// `VendorContactAmbiguous` is never actually produced at this
    /// settlement boundary (it is only constructed after the egress claim,
    /// on adapter retry exhaustion), but it is classified `true` here for
    /// the same reason `is_permanent` treats it that way: an
    /// ambiguous-contact kind must never be treated as safely reopenable.
    pub const fn is_permanent_preflight(self) -> bool {
        match self {
            Self::AuthorizationRevoked
            | Self::Rejected
            | Self::Unknown
            | Self::VendorContactAmbiguous => true,
            Self::TransientValidatorError | Self::TransportUnavailable | Self::RateLimited => false,
        }
    }

    /// Returns whether a `Failed` row carrying this failure kind must stay
    /// terminal for its deterministic delivery id, across the whole delivery
    /// lifecycle — not just the preflight settlement boundary
    /// `is_permanent_preflight` covers. A caller replaying the same logical
    /// delivery (same scope/actor/modality/candidate, hence the same
    /// deterministic id) may reopen a non-permanent `Failed` row to a fresh
    /// `Prepared` attempt instead of being stuck behind it forever.
    ///
    /// `AuthorizationRevoked` and `Unknown` are permanently terminal for the
    /// same reasons `is_permanent_preflight` treats them that way.
    /// `Rejected` additionally covers the OUT-7 partial-multipart terminal
    /// case recorded after the egress claim: once any part of a multipart
    /// delivery reached the vendor, a whole-envelope retry would duplicate
    /// the accepted parts, so it must never reopen even though the
    /// underlying per-part outcome may itself have been retryable.
    /// `TransportUnavailable`, `RateLimited`, and `TransientValidatorError`
    /// are only ever settled for this delivery id when nothing reached the
    /// vendor, before the egress claim, so reopening cannot duplicate a
    /// vendor-accepted send. `VendorContactAmbiguous` is different: it
    /// settles after the egress claim, once retries are exhausted without
    /// proof that no part of this delivery reached the vendor — either
    /// because the adapter's `deliver` call returned a bare `Err` instead of
    /// a typed report, or because every returned report part was
    /// `Retryable` (which in-tree adapters also use for post-send
    /// ambiguity, not just pre-send failure). Reopening it could resend a
    /// message the vendor already received, so — like `Unknown` — it must
    /// never reopen.
    pub const fn is_permanent(self) -> bool {
        match self {
            Self::AuthorizationRevoked
            | Self::Rejected
            | Self::Unknown
            | Self::VendorContactAmbiguous => true,
            Self::TransientValidatorError | Self::TransportUnavailable | Self::RateLimited => false,
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

/// Result of atomically claiming the sole vendor-egress drive for a prepared
/// delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDeliveryAttemptForSendOutcome {
    /// This caller persisted `Prepared -> Sending` and owns vendor egress.
    Claimed,
    /// The authoritative attempt no longer permits the transition.
    Existing(Box<OutboundDeliveryAttempt>),
}

/// Result of atomically settling a permanent failure while an attempt is
/// still prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailPreparedDeliveryAttemptOutcome {
    /// This caller persisted `Prepared -> Failed`.
    Settled,
    /// The authoritative attempt no longer permits the transition.
    Existing(Box<OutboundDeliveryAttempt>),
}

/// Atomic ownership claim for the sole vendor-egress writer of a prepared
/// delivery. Stores transition `Prepared -> Sending` exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimDeliveryAttemptForSendRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
}

/// Guarded settlement for a permanent failure discovered before vendor
/// egress ownership is claimed. Stores transition `Prepared -> Failed` only;
/// attempts that are already claimed or terminal are left unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailPreparedDeliveryAttemptRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub updated_at: Timestamp,
    pub failure_kind: DeliveryFailureKind,
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

    #[test]
    fn permanent_preflight_classification_covers_every_failure_kind() {
        let cases = [
            (DeliveryFailureKind::AuthorizationRevoked, true),
            (DeliveryFailureKind::TransientValidatorError, false),
            (DeliveryFailureKind::TransportUnavailable, false),
            (DeliveryFailureKind::RateLimited, false),
            (DeliveryFailureKind::Rejected, true),
            (DeliveryFailureKind::Unknown, true),
            (DeliveryFailureKind::VendorContactAmbiguous, true),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.is_permanent_preflight(), expected, "{kind:?}");
        }
    }

    #[test]
    fn permanent_classification_covers_every_failure_kind() {
        let cases = [
            (DeliveryFailureKind::AuthorizationRevoked, true),
            (DeliveryFailureKind::TransientValidatorError, false),
            (DeliveryFailureKind::TransportUnavailable, false),
            (DeliveryFailureKind::RateLimited, false),
            (DeliveryFailureKind::Rejected, true),
            (DeliveryFailureKind::Unknown, true),
            (DeliveryFailureKind::VendorContactAmbiguous, true),
        ];

        for (kind, expected) in cases {
            assert_eq!(kind.is_permanent(), expected, "{kind:?}");
        }
    }
}
