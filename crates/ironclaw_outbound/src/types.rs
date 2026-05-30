use crate::ids::{TriggerFireSlot, TriggerId};
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, ExternalConversationRef,
};
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, Timestamp};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::{OutboundDeliveryId, OutboundError, ProjectionSubscriptionId, ProjectionUpdateRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPushKind {
    FinalReply,
    Progress,
    GateRequired,
    DeliveryStatus,
}

#[allow(dead_code)] // retained for future debug/log surfaces — not yet wired
impl OutboundPushKind {
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

/// Delivery resolution target categories used by the outbound resolver.
///
/// Translation note: these domain kinds lower into the existing
/// `OutboundPushKind`/`PrepareOutboundDeliveryRequest` path at the outbound
/// policy boundary. `ApprovalPrompt` and `AuthPrompt` stay on the
/// run-notification side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationDeliveryKind {
    FinalReply,
    ProgressUpdate,
    DeliveryStatus,
    ApprovalPrompt,
    AuthPrompt,
}

/// Narrow intent for explicitly requested outbound delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedOutboundKind {
    ProductMessage,
    DeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationDeliveryResolutionRequest {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub delivery_kind: CommunicationDeliveryKind,
    pub modality: CommunicationModality,
    pub intent: CommunicationDeliveryIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationDeliveryIntent {
    RequestedOutbound(RequestedOutboundContext),
    RunNotification(RunNotificationContext),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedOutboundContext {
    pub requested_target: ReplyTargetBindingRef,
    pub requested_kind: RequestedOutboundKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNotificationContext {
    pub event_kind: RunNotificationEventKind,
    pub origin: RunNotificationOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemEventReasonCode {
    Generic,
    Trigger,
    Tool,
    Operator,
}

impl SystemEventReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Trigger => "trigger",
            Self::Tool => "tool",
            Self::Operator => "operator",
        }
    }
}

impl std::fmt::Display for SystemEventReasonCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationEventKind {
    FinalReplyReady,
    ProgressUpdate,
    ApprovalNeeded,
    AuthRequired,
    RunBlocked,
    DeliveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunNotificationOrigin {
    LiveSourceRoute {
        source_route: SourceRouteContext,
    },
    Triggered {
        trigger: TriggerCommunicationContext,
    },
    TriggeredFromSourceRoute {
        trigger: TriggerCommunicationContext,
        source_route: SourceRouteContext,
    },
    SystemEvent {
        reason: SystemEventReasonCode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRouteContext {
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCommunicationContext {
    pub trigger_id: TriggerId,
    pub trigger_source_kind: TriggerSourceKind,
    pub fire_slot: TriggerFireSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSourceKind {
    Schedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationModality {
    Text,
    Voice,
    Image,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeliveryTargetCapabilities {
    pub final_replies: bool,
    pub progress: bool,
    pub gate_prompts: bool,
    pub auth_prompts: bool,
    pub modalities: Vec<CommunicationModality>,
}

/// Candidate produced by the outbound resolution step.
///
/// The candidate is still only a target choice. It lowers into the existing
/// `OutboundPushCandidate` / `PrepareOutboundDeliveryRequest` boundary, where
/// target validation and delivery-attempt recording still live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunicationDeliveryCandidate {
    pub target: ReplyTargetBindingRef,
    pub kind: CommunicationDeliveryKind,
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

#[allow(dead_code)] // retained for future debug/log surfaces — not yet wired
impl OutboundDeliveryStatus {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
    use serde::de::DeserializeOwned;
    use serde_json::{from_str, to_string};

    #[test]
    fn communication_delivery_resolution_request_round_trips_requested_outbound() {
        let request = CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor(),
            delivery_kind: CommunicationDeliveryKind::FinalReply,
            modality: CommunicationModality::Mixed,
            intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
                requested_target: reply_ref("reply:requested"),
                requested_kind: RequestedOutboundKind::ProductMessage,
            }),
        };

        let json = to_string(&request).expect("serialize requested outbound request");
        let decoded: CommunicationDeliveryResolutionRequest =
            from_str(&json).expect("deserialize requested outbound request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn communication_delivery_resolution_request_round_trips_run_notification() {
        let request = CommunicationDeliveryResolutionRequest {
            scope: scope(),
            actor: actor(),
            delivery_kind: CommunicationDeliveryKind::ApprovalPrompt,
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::RunBlocked,
                origin: RunNotificationOrigin::TriggeredFromSourceRoute {
                    trigger: trigger_context(),
                    source_route: source_route_context(),
                },
            }),
        };

        let json = to_string(&request).expect("serialize run notification request");
        let decoded: CommunicationDeliveryResolutionRequest =
            from_str(&json).expect("deserialize run notification request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn outbound_translation_enums_round_trip_all_variants() {
        for value in [
            CommunicationDeliveryKind::FinalReply,
            CommunicationDeliveryKind::ProgressUpdate,
            CommunicationDeliveryKind::DeliveryStatus,
            CommunicationDeliveryKind::ApprovalPrompt,
            CommunicationDeliveryKind::AuthPrompt,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            RequestedOutboundKind::ProductMessage,
            RequestedOutboundKind::DeliveryStatus,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            RunNotificationEventKind::FinalReplyReady,
            RunNotificationEventKind::ProgressUpdate,
            RunNotificationEventKind::ApprovalNeeded,
            RunNotificationEventKind::AuthRequired,
            RunNotificationEventKind::RunBlocked,
            RunNotificationEventKind::DeliveryStatus,
        ] {
            assert_json_round_trip(value);
        }

        for value in [
            CommunicationModality::Text,
            CommunicationModality::Voice,
            CommunicationModality::Image,
            CommunicationModality::Mixed,
            CommunicationModality::Unknown,
        ] {
            assert_json_round_trip(value);
        }

        for value in [TriggerSourceKind::Schedule] {
            assert_json_round_trip(value);
        }

        for value in [
            SystemEventReasonCode::Generic,
            SystemEventReasonCode::Trigger,
            SystemEventReasonCode::Tool,
            SystemEventReasonCode::Operator,
        ] {
            assert_json_round_trip(value);
        }
    }

    #[test]
    fn communication_delivery_candidate_round_trips() {
        let candidate = CommunicationDeliveryCandidate {
            target: reply_ref("reply:candidate"),
            kind: CommunicationDeliveryKind::DeliveryStatus,
        };

        let json = to_string(&candidate).expect("serialize delivery candidate");
        let decoded: CommunicationDeliveryCandidate =
            from_str(&json).expect("deserialize delivery candidate");
        assert_eq!(decoded, candidate);
    }

    #[test]
    fn delivery_target_capabilities_round_trip() {
        let capabilities = DeliveryTargetCapabilities {
            final_replies: true,
            progress: true,
            gate_prompts: false,
            auth_prompts: true,
            modalities: vec![CommunicationModality::Text, CommunicationModality::Mixed],
        };

        let json = to_string(&capabilities).expect("serialize capabilities");
        let decoded: DeliveryTargetCapabilities =
            from_str(&json).expect("deserialize capabilities");
        assert_eq!(decoded, capabilities);
        assert_json_round_trip(SystemEventReasonCode::Generic);
        assert!(from_str::<SystemEventReasonCode>("\"backend_failure\"").is_err());
    }

    fn scope() -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-a").expect("valid tenant"),
            Some(AgentId::new("agent-a").expect("valid agent")),
            Some(ProjectId::new("project-a").expect("valid project")),
            thread_id("thread-a"),
        )
    }

    fn actor() -> TurnActor {
        TurnActor::new(UserId::new("user-a").expect("valid user"))
    }

    fn thread_id(value: &str) -> ThreadId {
        ThreadId::new(value).expect("valid thread")
    }

    fn reply_ref(value: &str) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("valid reply target")
    }

    fn source_route_context() -> SourceRouteContext {
        SourceRouteContext {
            adapter_kind: AdapterKind::new("telegram").expect("valid adapter kind"),
            adapter_installation_id: AdapterInstallationId::new("install-alpha")
                .expect("valid installation id"),
            external_actor_ref: ExternalActorRef::new("user", "alice")
                .expect("valid external actor"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("space-alpha"),
                "conversation-alpha",
                Some("topic-alpha"),
                Some("message-alpha"),
            )
            .expect("valid conversation ref"),
            reply_target_binding_ref: reply_ref("reply:source-route"),
        }
    }

    fn trigger_context() -> TriggerCommunicationContext {
        TriggerCommunicationContext {
            trigger_id: TriggerId::new("trigger:daily").expect("valid trigger id"),
            trigger_source_kind: TriggerSourceKind::Schedule,
            fire_slot: TriggerFireSlot::new("2026-05-29T09:00:00Z").expect("valid fire slot"),
        }
    }

    fn assert_json_round_trip<T>(value: T)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = to_string(&value).expect("serialize value");
        let decoded: T = from_str(&json).expect("deserialize value");
        assert_eq!(decoded, value);
    }
}
