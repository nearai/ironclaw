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

/// Host-owned authority over a binding's thread ownership that a trusted caller
/// may supply when resolving or creating a conversation binding.
///
/// This enum is trusted-scope–only input. Raw adapter paths (i.e.
/// `resolve_or_create_binding` without trusted scope) must not be able to
/// express `Project`; they must always pass or receive `Unspecified`.
///
/// Variants:
/// - `Unspecified` – the caller makes no ownership claim; existing stored
///   ownership is unchanged and new bindings receive no stored owner.
///   Equivalent to the former `trusted_owner_user_id: None` path.
/// - `User(UserId)` – the binding is owned by the named user; stored on first
///   bind and may be adopted on shared-route re-entry when the binding has no
///   stored owner. Equivalent to the former `trusted_owner_user_id: Some(u)`.
/// - `Project` – the caller explicitly asserts the binding has no personal
///   owner (owned by the binding's project scope). The binding's ownerless
///   state is stored and `BindingRecord::resolution()` emits
///   `TurnScope::new_with_owner(..., None)`, carrying the explicit-ownerless
///   marker through to the run's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedOwnerScope {
    /// No ownership claim from the caller; no stored-state change.
    Unspecified,
    /// Explicit personal ownership by the given user.
    User(UserId),
    /// Explicitly project-owned (no personal user owner).
    Project,
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
pub(crate) struct TrustedInboundTurnRequest {
    request: InboundTurnRequest,
    trusted_agent_id: Option<AgentId>,
    trusted_project_id: Option<ProjectId>,
    trusted_owner: TrustedOwnerScope,
}

impl TrustedInboundTurnRequest {
    pub(crate) fn new(
        request: InboundTurnRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            request,
            trusted_agent_id,
            trusted_project_id,
            // Default to Unspecified when the caller (e.g. existing trigger
            // materializer) does not yet supply an explicit owner scope.
            trusted_owner: TrustedOwnerScope::Unspecified,
        }
    }

    /// Construct a trusted request with an explicit owner scope.
    ///
    /// Called by the composition materializer (wave-2 workstream) to pass
    /// `User(creator)` for personal fires and `Project` for project fires.
    #[allow(dead_code)] // used by ironclaw_reborn_composition (wave 2)
    pub(crate) fn new_with_owner(
        request: InboundTurnRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
        trusted_owner: TrustedOwnerScope,
    ) -> Self {
        Self {
            request,
            trusted_agent_id,
            trusted_project_id,
            trusted_owner,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        InboundTurnRequest,
        Option<AgentId>,
        Option<ProjectId>,
        TrustedOwnerScope,
    ) {
        (
            self.request,
            self.trusted_agent_id,
            self.trusted_project_id,
            self.trusted_owner,
        )
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
