//! The per-channel protocol seam of the adapter-generic delivery machinery.
//!
//! `ironclaw_channel_delivery` owns the generic
//! final-reply delivery observer and triggered-run delivery driver; the types
//! here are the contract a channel host implements to plug into them:
//! stored reply-target ref decoding, personal-DM classification, and the
//! lightweight status/notification messages posted around the adapter render
//! path. They live in this contract crate so a channel host can implement
//! [`ChannelDeliveryProtocol`] without depending on the delivery engine.

use async_trait::async_trait;
use ironclaw_product_adapters::{ExternalConversationRef, ProductAdapterError, ProtocolHttpEgress};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use thiserror::Error;

/// Per-channel protocol details the adapter-generic delivery machinery needs:
/// stored reply-target ref decoding, personal-DM classification,
/// render-response tracking, and the lightweight status/notification messages
/// posted around the adapter render path. Each channel host supplies its own
/// implementation when constructing the delivery observer/driver services —
/// the generic machinery never keys on a concrete channel.
#[async_trait]
pub trait ChannelDeliveryProtocol: Send + Sync {
    /// Stable channel-owned namespace for run-notification projections. Slack
    /// keeps its legacy `slack` prefix; every other channel must use a distinct
    /// value so composite delivery cannot alias cursor/idempotency identities.
    fn run_notification_projection_prefix(&self) -> &'static str;

    /// Decode a stored reply-target binding ref into
    /// `(conversation_id, space_id)`. `None` for a ref this channel does not
    /// own — the formats are channel-exclusive, so a foreign ref fails closed.
    fn conversation_id_from_reply_target_binding_ref(
        &self,
        target: &ReplyTargetBindingRef,
    ) -> Option<(String, Option<String>)>;

    /// `true` iff the ref is a personal direct-message target for this
    /// channel. Backs the OAuth-DM delivery rule.
    fn reply_target_is_personal_dm(&self, target: &ReplyTargetBindingRef) -> bool;

    /// Sniff an adapter-rendered egress response for a posted-message handle
    /// (used to later delete placeholder/working messages). `None` when the
    /// response is not a trackable post.
    fn posted_message_from_render_response(
        &self,
        path: &str,
        request_body: &[u8],
        response_body: &[u8],
    ) -> Option<PostedChannelMessage>;

    /// First-contact greeting for a sender who has not connected their
    /// account (fixed, host-authored text only — no agent runs).
    fn connect_nudge_message(&self) -> &'static str;

    /// Whether an external conversation id denotes a 1:1 direct message in
    /// this channel's id scheme (Slack: `D…` channel ids; Telegram: positive
    /// private-chat ids). Gates host-authored nudges out of shared surfaces.
    fn is_direct_message_conversation(&self, conversation_id: &str) -> bool;

    /// Post a lightweight host-authored status/notification message to the
    /// conversation, outside the adapter render path.
    async fn post_status_message(
        &self,
        egress: &dyn ProtocolHttpEgress,
        conversation: &ExternalConversationRef,
        text: &str,
    ) -> Result<PostedChannelMessage, FinalReplyDeliveryError>;

    /// Delete a previously posted status message. Best-effort; channels
    /// without deletion return `Ok(())`.
    async fn delete_status_message(
        &self,
        egress: &dyn ProtocolHttpEgress,
        message: &PostedChannelMessage,
    ) -> Result<(), FinalReplyDeliveryError>;
}

/// Channel-opaque handle to a posted status message (conversation plus the
/// channel's message reference, e.g. Slack's `ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedChannelMessage {
    pub conversation_id: String,
    pub message_ref: String,
}

/// Errors surfaced by the adapter-generic delivery path and by
/// [`ChannelDeliveryProtocol`] implementations.
#[derive(Debug, Error)]
pub enum FinalReplyDeliveryError {
    #[error("workflow binding failed: {0}")]
    Workflow(#[from] ironclaw_product_workflow::ProductWorkflowError),
    #[error("approval prompt lookup failed: {0}")]
    ApprovalPrompt(#[from] ironclaw_product_workflow::ApprovalPromptLookupError),
    #[error("turn coordinator failed: {0}")]
    Turn(#[from] ironclaw_turns::TurnError),
    #[error("thread service failed: {0}")]
    Thread(#[from] ironclaw_threads::SessionThreadError),
    #[error("outbound delivery failed: {0}")]
    Outbound(#[from] ironclaw_product_workflow::ProductOutboundDeliveryError),
    #[error("adapter failed: {0}")]
    Adapter(#[from] ProductAdapterError),
    #[error("channel status-message helper failed: {reason}")]
    StatusMessage { reason: String },
    #[error("outbound policy failed: {0}")]
    OutboundPolicy(#[from] ironclaw_outbound::OutboundError),
    #[error("run {run_id} did not finish before the channel delivery timeout")]
    RunWaitTimedOut { run_id: TurnRunId },
    /// Timeout after at least one blocked-state notification (approval/auth
    /// prompt) was already delivered. The user is not in silence, so no
    /// additional feedback message is needed.
    #[error("run {run_id} did not reach a terminal state after delivering a blocked notification")]
    RunWaitTimedOutAfterNotification { run_id: TurnRunId },
    #[error("invalid projection ref: {reason}")]
    InvalidProjectionRef { reason: String },
    #[error("run {run_id} produced no posted-message delivery evidence")]
    DeliveryEvidenceMissing { run_id: TurnRunId },
}
