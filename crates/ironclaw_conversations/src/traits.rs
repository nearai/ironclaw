use async_trait::async_trait;
use ironclaw_turns::{AcceptedMessageRef, IdempotencyKey};

use crate::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, ConversationBindingResolution,
    InboundTurnError, LinkConversationRequest, LinkedConversationBinding, ReplyTargetBinding,
    ResolveConversationRequest, ValidateReplyTargetRequest,
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
        request: ValidateReplyTargetRequest,
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
    ) -> Result<(), InboundTurnError>;
}
