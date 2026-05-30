use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, ReplyTargetBindingRef, RunProfileRequest, SourceBindingRef,
    SubmitTurnResponse, TurnActor, TurnScope,
};
use serde::{Deserialize, Serialize};

use crate::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    InboundMessageContentRef,
};

#[allow(dead_code)]
pub(crate) mod trusted_ingress {
    use core::marker::PhantomData;

    use super::private;

    mod sealed {
        pub trait Sealed {}
    }

    /// Sealed host-only token proving the caller already crossed the host
    /// ingress boundary. Only crate-local host code can mint it, so product
    /// adapters cannot fabricate trusted scope on their own. A later
    /// host-owned shim in this crate can mint the token without widening the
    /// public API.
    pub(crate) trait TrustedIngressToken: sealed::Sealed {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TrustedIngressWitness(PhantomData<private::TrustedIngressSeal>);

    impl sealed::Sealed for TrustedIngressWitness {}

    impl TrustedIngressToken for TrustedIngressWitness {}

    pub(crate) fn mint() -> impl TrustedIngressToken {
        TrustedIngressWitness(PhantomData)
    }
}

#[allow(dead_code)]
mod private {
    pub(crate) struct TrustedIngressSeal;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRouteKind {
    Direct,
    Shared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveConversationRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
    pub route_kind: ConversationRouteKind,
    pub requested_agent_id: Option<AgentId>,
    pub requested_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationBindingResolution {
    pub tenant_id: TenantId,
    pub actor: TurnActor,
    pub turn_scope: TurnScope,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub access: ThreadAccessDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkConversationRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub route_kind: ConversationRouteKind,
    pub target_thread_id: ThreadId,
    pub target_agent_id: Option<AgentId>,
    pub target_project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedConversationBinding {
    pub thread_id: ThreadId,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateReplyTargetRequest {
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub current_thread_id: ThreadId,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyTargetBinding {
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub thread_id: ThreadId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_conversation_ref: ExternalConversationRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadAccessDecision {
    Allowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageIdempotencyStatus {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedInboundMessageLookup {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedInboundMessageReplay {
    pub resolution: ConversationBindingResolution,
    pub accepted_message: AcceptedInboundMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptInboundMessageRequest {
    pub tenant_id: TenantId,
    pub thread_id: ThreadId,
    pub actor: TurnActor,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
    pub route_kind: ConversationRouteKind,
    pub content_ref: InboundMessageContentRef,
    pub received_at: DateTime<Utc>,
    pub requested_run_profile: Option<RunProfileRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedInboundMessage {
    pub tenant_id: TenantId,
    pub thread_id: ThreadId,
    pub actor: TurnActor,
    pub message_ref: AcceptedMessageRef,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub received_at: DateTime<Utc>,
    pub requested_run_profile: Option<RunProfileRequest>,
    pub idempotency: MessageIdempotencyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessageRecord {
    pub accepted: AcceptedInboundMessage,
    pub actor: TurnActor,
    pub external_event_id: ExternalEventId,
    pub content_ref: InboundMessageContentRef,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnRequest {
    pub tenant_id: TenantId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub external_event_id: ExternalEventId,
    pub route_kind: ConversationRouteKind,
    pub content_ref: InboundMessageContentRef,
    pub requested_agent_id: Option<AgentId>,
    pub requested_project_id: Option<ProjectId>,
    pub received_at: DateTime<Utc>,
    pub requested_run_profile: Option<RunProfileRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedInboundTurnRequest {
    request: InboundTurnRequest,
    trusted_agent_id: Option<AgentId>,
    trusted_project_id: Option<ProjectId>,
}

#[allow(dead_code)]
impl TrustedInboundTurnRequest {
    /// Crate-local constructor for host-owned trusted ingress.
    /// The sealed token keeps external adapters from minting trusted scope,
    /// while still letting this crate add a future host shim without changing
    /// the public API.
    pub(crate) fn new(
        _witness: impl trusted_ingress::TrustedIngressToken,
        request: InboundTurnRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            request,
            trusted_agent_id,
            trusted_project_id,
        }
    }

    pub(crate) fn into_parts(self) -> (InboundTurnRequest, Option<AgentId>, Option<ProjectId>) {
        (self.request, self.trusted_agent_id, self.trusted_project_id)
    }
}

#[cfg(test)]
mod tests {
    use super::trusted_ingress;

    #[test]
    fn trusted_ingress_witness_is_host_minted_and_zero_sized() {
        let witness = trusted_ingress::mint();

        assert_eq!(core::mem::size_of_val(&witness), 0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnResponse {
    pub resolution: ConversationBindingResolution,
    pub accepted_message: AcceptedInboundMessage,
    pub turn_submission: Option<SubmitTurnResponse>,
}
