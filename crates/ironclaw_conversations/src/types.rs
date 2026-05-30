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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedInboundTurnRequest {
    pub(crate) tenant_id: TenantId,
    pub(crate) adapter_kind: AdapterKind,
    pub(crate) adapter_installation_id: AdapterInstallationId,
    pub(crate) external_actor_ref: ExternalActorRef,
    pub(crate) external_conversation_ref: ExternalConversationRef,
    pub(crate) external_event_id: ExternalEventId,
    pub(crate) route_kind: ConversationRouteKind,
    pub(crate) content_ref: InboundMessageContentRef,
    pub(crate) creator_user_id: UserId,
    pub(crate) trusted_agent_id: Option<AgentId>,
    pub(crate) trusted_project_id: Option<ProjectId>,
    pub(crate) received_at: DateTime<Utc>,
    pub(crate) requested_run_profile: Option<RunProfileRequest>,
}

impl TrustedInboundTurnRequest {
    pub fn new(
        request: InboundTurnRequest,
        creator_user_id: UserId,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
    ) -> Self {
        let InboundTurnRequest {
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            external_conversation_ref,
            external_event_id,
            route_kind,
            content_ref,
            requested_agent_id: _,
            requested_project_id: _,
            received_at,
            requested_run_profile,
        } = request;

        Self {
            tenant_id,
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            external_conversation_ref,
            external_event_id,
            route_kind,
            content_ref,
            creator_user_id,
            trusted_agent_id,
            trusted_project_id,
            received_at,
            requested_run_profile,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnResponse {
    pub resolution: ConversationBindingResolution,
    pub accepted_message: AcceptedInboundMessage,
    pub turn_submission: Option<SubmitTurnResponse>,
}
