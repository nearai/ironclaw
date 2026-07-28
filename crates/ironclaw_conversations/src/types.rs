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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ExternalActorBindingEpoch(String);

impl ExternalActorBindingEpoch {
    fn validate(value: &str) -> Result<(), crate::InboundTurnError> {
        crate::ids::validate_external_id("external_actor_binding_epoch", value)
    }

    pub fn new(value: impl Into<String>) -> Result<Self, crate::InboundTurnError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for ExternalActorBindingEpoch {
    type Error = crate::InboundTurnError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for ExternalActorBindingEpoch {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for ExternalActorBindingEpoch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<ExternalActorBindingEpoch> for String {
    fn from(epoch: ExternalActorBindingEpoch) -> Self {
        epoch.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalUnpairOutcome {
    Unpaired,
    AlreadyAbsent,
    OwnerChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedExternalActorOwner {
    pub user_id: UserId,
    pub binding_epoch: Option<ExternalActorBindingEpoch>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_epoch: Option<ExternalActorBindingEpoch>,
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

/// Authority required when resolving a previously sealed reply target from
/// durable turn state.
///
/// Ordinary replies may use a shared route when the run actor is still a
/// participant in the bound thread. Authority-bearing prompts (approval and
/// authentication) require the exact external actor that originally owned the
/// route to remain paired to the run actor; shared-route widening is never
/// sufficient for those prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredReplyTargetAccess {
    OrdinaryReply,
    ExactOriginActor,
}

/// Resolves the opaque reply-target reference persisted on a run without
/// accepting adapter-supplied route metadata a second time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveStoredReplyTargetRequest {
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub current_thread_id: ThreadId,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub access: StoredReplyTargetAccess,
}

/// Revalidated source route for event-driven delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredReplyTargetBinding {
    pub tenant_id: TenantId,
    pub actor_user_id: UserId,
    pub thread_id: ThreadId,
    pub adapter_kind: AdapterKind,
    pub adapter_installation_id: AdapterInstallationId,
    pub external_conversation_ref: ExternalConversationRef,
    pub route_kind: ConversationRouteKind,
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

/// Whether a trusted inbound submission came from the trusted-trigger fire
/// path. Carried as a type so origin classification never re-derives
/// trigger-ness from the adapter-kind string (see `.claude/rules/types.md`).
/// Only the trusted-trigger submit seam constructs `Trigger`; every other
/// trusted ingress is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustedInboundKind {
    Trigger,
    // arch-exempt: dead_code, reserved trusted non-trigger ingress — the only
    // trusted production path today is the trigger submit seam (which builds
    // `Trigger`); `Other` is exercised by the trusted-non-trigger inbound tests
    // and becomes live when a trusted non-trigger ingress is added. Until then
    // it must stay a typed variant so classification cannot silently fall back
    // to `TrustedTrigger` for a future trusted caller.
    #[allow(dead_code)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedInboundTurnRequest {
    pub(crate) request: InboundTurnRequest,
    pub(crate) trusted_agent_id: Option<AgentId>,
    pub(crate) trusted_project_id: Option<ProjectId>,
    pub(crate) trusted_owner_user_id: Option<UserId>,
    pub(crate) kind: TrustedInboundKind,
}

impl TrustedInboundTurnRequest {
    pub(crate) fn new(
        request: InboundTurnRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
        trusted_owner_user_id: Option<UserId>,
        kind: TrustedInboundKind,
    ) -> Self {
        Self {
            request,
            trusted_agent_id,
            trusted_project_id,
            trusted_owner_user_id,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundTurnResponse {
    pub resolution: ConversationBindingResolution,
    pub accepted_message: AcceptedInboundMessage,
    pub turn_submission: Option<SubmitTurnResponse>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replayed_turn_submission: bool,
}
