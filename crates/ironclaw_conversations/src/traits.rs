use async_trait::async_trait;
use ironclaw_host_api::{TenantId, ThreadId, UserId};
use ironclaw_turns::{AcceptedMessageRef, ReplyTargetBindingRef};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, ConversationBindingResolution,
    InboundTurnError, LinkConversationRequest, LinkedConversationBinding, ReplyTargetBinding,
    ResolveConversationRequest,
};

#[async_trait]
pub trait ConversationBindingService: Send + Sync {
    async fn resolve_or_create_binding(
        &self,
        request: ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError>;

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError>;

    async fn validate_reply_target(
        &self,
        tenant_id: &TenantId,
        actor_user_id: &UserId,
        current_thread_id: &ThreadId,
        reply_target_binding_ref: &ReplyTargetBindingRef,
    ) -> Result<ReplyTargetBinding, InboundTurnError>;
}

pub trait ConversationBindingServiceExt: ConversationBindingService {}
impl<T> ConversationBindingServiceExt for T where T: ConversationBindingService {}

#[async_trait]
pub trait SessionThreadService: Send + Sync {
    async fn accept_inbound_message(
        &self,
        request: AcceptInboundMessageRequest,
    ) -> Result<AcceptedInboundMessage, InboundTurnError>;

    async fn inbound_message_turn_submitted(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<bool, InboundTurnError>;

    async fn mark_inbound_message_turn_submitted(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError>;
}
