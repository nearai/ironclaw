use async_trait::async_trait;
use ironclaw_turns::{AcceptedMessageRef, IdempotencyKey, SubmitTurnResponse};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AcceptedInboundMessageReplay, AdapterInstallationId, AdapterKind,
    ConversationBindingResolution, ExternalActorRef, InboundTurnError, LinkConversationRequest,
    LinkedConversationBinding, ReplyTargetBinding, ResolveConversationRequest,
    ValidateReplyTargetRequest,
};

#[async_trait]
pub trait ConversationBindingService: Send + Sync {
    /// Resolve an existing binding or create a first-contact binding without
    /// trusting adapter-supplied requested scope hints.
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError>;

    /// Resolve or create a binding while applying host-owned default scope.
    ///
    /// The trusted scope must come from host configuration, not adapter input.
    /// Implementations that persist bindings should persist these values on
    /// first bind so later configuration changes do not silently reinterpret
    /// the existing external conversation route. `trusted_owner_user_id`, when
    /// present, is the explicit thread owner for that first-bound route.
    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        request: ResolveConversationRequest,
        trusted_agent_id: Option<ironclaw_host_api::AgentId>,
        trusted_project_id: Option<ironclaw_host_api::ProjectId>,
        trusted_owner_user_id: Option<ironclaw_host_api::UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError>;

    /// Look up an existing binding without creating or widening binding state.
    async fn lookup_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError>;

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError>;

    async fn validate_reply_target(
        &self,
        request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError>;
}

#[async_trait]
pub trait ConversationActorPairingService: Send + Sync {
    /// Pair an adapter-scoped external actor with a canonical Reborn user.
    ///
    /// Callers must supply only host-trusted pairings. This is not a self-service
    /// code approval flow; it persists an already-authorized actor mapping for
    /// subsequent binding resolution.
    async fn pair_external_actor(
        &self,
        tenant_id: ironclaw_host_api::TenantId,
        adapter_kind: AdapterKind,
        adapter_installation_id: AdapterInstallationId,
        external_actor_ref: ExternalActorRef,
        user_id: ironclaw_host_api::UserId,
    ) -> Result<(), InboundTurnError>;
}

#[async_trait]
pub trait SessionThreadService: Send + Sync {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError>;

    async fn replay_accepted_inbound_message(
        &self,
        lookup: AcceptedInboundMessageLookup,
    ) -> Result<Option<AcceptedInboundMessageReplay>, InboundTurnError>;

    async fn inbound_message_turn_submission(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<Option<SubmitTurnResponse>, InboundTurnError>;

    async fn inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<IdempotencyKey, InboundTurnError>;

    async fn rotate_inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError>;

    async fn mark_inbound_message_turn_submitted(
        &self,
        message_ref: &AcceptedMessageRef,
        response: SubmitTurnResponse,
    ) -> Result<(), InboundTurnError>;
}
