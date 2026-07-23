//! The generic **channel adapter** contract (overview.md §4.2).
//!
//! One adapter per extension channel surface. It implements protocol
//! behavior only — parse one host-verified inbound request, render and send
//! one normalized outbound envelope, and the idempotent activate/cleanup
//! vendor-wiring hooks. Everything around it (route table, verification
//! recipes, replay, admission, target policy, attempt persistence, retry,
//! drain) is the host ingress router and delivery coordinator, implemented
//! once. The adapter never reports metadata (the resolved manifest is the
//! authority) and never touches the delivery store.
//!
//! These DTOs are the seam between generic host pipelines and concrete
//! protocol crates; the old metadata-carrying `ProductAdapter` is retired as
//! its callers cut over (implementation.md §5).

use async_trait::async_trait;

use crate::RestrictedEgress;

use crate::product_adapter::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use crate::product_adapter::inbound::ProductTriggerReason;

/// A channel adapter: protocol behavior for one extension's channel surface.
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Idempotent vendor-side wiring + config validation, run during
    /// activation (e.g. a webhook registration, an auth probe). Failure
    /// fails activation.
    async fn activate(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Idempotent, best-effort vendor-side unwiring, run during
    /// deactivation/removal. Failure is recorded and retryable; it does not
    /// block removal forever.
    async fn cleanup(
        &self,
        _ctx: &ChannelContext<'_>,
        _egress: &dyn RestrictedEgress,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Parse one host-verified inbound request into a normalized outcome.
    /// Pure protocol work: no I/O, no secrets, bounded input.
    fn inbound(&self, request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError>;

    /// Render and send one normalized outbound envelope through restricted
    /// egress. Owns vendor formatting, splitting, target syntax, DM
    /// provisioning, and safe error mapping. Never touches the delivery
    /// store.
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError>;

    /// Optional: list/search delivery targets for pickers.
    async fn list_targets(
        &self,
        _query: TargetQuery,
        _egress: &dyn RestrictedEgress,
    ) -> Result<Vec<TargetCandidate>, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

/// Activation/cleanup context: installation identity, the extension's
/// non-secret config values, and the resolved channel descriptor. Secrets
/// exist only behind host egress injection.
pub struct ChannelContext<'a> {
    pub extension_id: &'a str,
    pub installation_id: &'a str,
    /// Non-secret operator config values keyed by field handle.
    pub config: &'a [(String, String)],
}

/// One host-verified inbound request. Signing secrets are never in scope —
/// the host executed the verification recipe before calling `inbound`.
pub struct VerifiedInbound<'a> {
    pub extension_id: &'a str,
    pub installation_id: &'a str,
    /// Request body bytes (bounded by the ingress body limit).
    pub body: &'a [u8],
    /// Request headers the host chose to forward (verification headers are
    /// consumed by the host and not exposed).
    pub headers: &'a [(String, String)],
}

/// The normalized result of parsing one inbound request.
pub enum InboundOutcome {
    /// Normalized message(s) for the workflow.
    Messages(Vec<NormalizedInboundMessage>),
    /// Bounded immediate response (e.g. a URL-verification challenge).
    Respond(ImmediateResponse),
    /// Authenticated no-op (ignored event types).
    Ignore,
}

/// One normalized inbound message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedInboundMessage {
    pub actor: ExternalActorRef,
    pub conversation: ExternalConversationRef,
    pub event_id: ExternalEventId,
    pub text: String,
    /// Why the protocol forwarded this message (direct chat, bot mention,
    /// thread reply, …). The workflow's user-message payload requires it, so
    /// any host sink mapping normalized messages into the workflow needs it.
    pub trigger: ProductTriggerReason,
    pub attachments: Vec<AttachmentRef>,
    /// Opaque per-message context (≤ 4 KiB) the host stores server-side and
    /// hands back at delivery time (reply routing). Never interpreted by the
    /// host.
    pub reply_context: Option<Vec<u8>>,
}

/// Maximum size of an inbound message's opaque `reply_context`.
pub const MAX_REPLY_CONTEXT_BYTES: usize = 4 * 1024;

/// An attachment reference — the vendor URL/id plus a mime hint. Bytes are
/// fetched host-side through restricted egress with the channel credential
/// only when a consumer needs them, keeping `inbound` pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub descriptor: ProductAttachmentDescriptor,
    pub vendor_ref: String,
    pub mime_hint: Option<String>,
}

/// A bounded immediate response (returned after verification, before any
/// enqueue).
#[derive(Debug, Clone)]
pub struct ImmediateResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Maximum size of an [`ImmediateResponse`] body.
pub const MAX_IMMEDIATE_RESPONSE_BYTES: usize = 64 * 1024;

/// One outbound envelope the delivery coordinator hands the adapter.
#[derive(Debug, Clone)]
pub struct OutboundEnvelope {
    pub extension_id: String,
    pub installation_id: String,
    pub delivery_attempt_id: String,
    /// Resolved target (source-route reply or preference target).
    pub target: OutboundTarget,
    /// The rendered message parts, already reduced from the semantic intent by
    /// the coordinator.
    pub parts: Vec<OutboundPart>,
    /// The stored `reply_context` from the originating inbound message, if
    /// this delivery replies to one.
    pub reply_context: Option<Vec<u8>>,
}

/// A resolved outbound target for one delivery.
#[derive(Debug, Clone)]
pub struct OutboundTarget {
    /// Vendor conversation reference (channel/DM/chat id).
    pub conversation: ExternalConversationRef,
    /// Optional threading anchor within the conversation.
    pub thread_anchor: Option<String>,
}

/// One part of an outbound message.
#[derive(Debug, Clone)]
pub enum OutboundPart {
    Text(String),
    /// Structured authentication challenge. The coordinator forwards this
    /// unchanged; each channel adapter owns native rendering while preserving
    /// the same recipe materialization WebUI consumes.
    AuthPrompt {
        view: Box<crate::AuthPromptView>,
        direct_message: bool,
    },
    /// Remove an earlier delivery in the target conversation (the `Cleanup`
    /// intent, e.g. deleting a working indicator). `vendor_message_ref` is
    /// the reference a previous [`PartDeliveryOutcome::Sent`] returned; the
    /// adapter resolves it against the envelope's target conversation.
    Retract {
        vendor_message_ref: String,
    },
}

/// Structured per-attempt delivery report. The adapter cannot mark anything
/// delivered in a store; it only describes what the vendor did.
#[derive(Debug, Clone)]
pub struct DeliveryReport {
    pub parts: Vec<PartDeliveryOutcome>,
}

/// The outcome of delivering one part.
#[derive(Debug, Clone)]
pub enum PartDeliveryOutcome {
    /// Delivered; the vendor message reference, when the protocol returns one.
    Sent { vendor_message_ref: Option<String> },
    /// Transient failure; the coordinator may retry.
    Retryable { reason: String },
    /// Permanent failure; the coordinator will not retry.
    Permanent { reason: String },
    /// The vendor rejected authorization; the coordinator raises re-auth.
    Unauthorized { reason: String },
}

/// A target-listing/search query for pickers.
#[derive(Debug, Clone)]
pub struct TargetQuery {
    pub extension_id: String,
    pub installation_id: String,
    /// Optional free-text filter.
    pub query: Option<String>,
    pub limit: u32,
}

/// One candidate delivery target.
#[derive(Debug, Clone)]
pub struct TargetCandidate {
    pub conversation: ExternalConversationRef,
    pub display_name: String,
}

/// Typed channel-adapter failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelError {
    #[error("inbound request could not be parsed: {reason}")]
    Parse { reason: String },
    #[error("outbound rendering failed: {reason}")]
    Render { reason: String },
    #[error("vendor wiring failed: {reason}")]
    VendorWiring { reason: String },
    #[error("channel operation is not supported by this adapter")]
    Unsupported,
}

impl NormalizedInboundMessage {
    /// Validate host-enforceable bounds on a normalized message before it
    /// enters the workflow (the adapter is untrusted for size).
    pub fn validate(&self) -> Result<(), ChannelError> {
        if let Some(context) = &self.reply_context
            && context.len() > MAX_REPLY_CONTEXT_BYTES
        {
            return Err(ChannelError::Parse {
                reason: "reply_context exceeds the 4 KiB bound".to_string(),
            });
        }
        Ok(())
    }
}

impl ImmediateResponse {
    /// Validate an immediate response is within host bounds.
    pub fn validate(&self) -> Result<(), ChannelError> {
        if self.body.len() > MAX_IMMEDIATE_RESPONSE_BYTES {
            return Err(ChannelError::Render {
                reason: "immediate response body exceeds the host bound".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_context_bound_is_enforced_host_side() {
        let message = NormalizedInboundMessage {
            actor: ExternalActorRef::new("user", "u-1", None::<&str>).expect("actor"),
            conversation: ExternalConversationRef::new(None, "c-1", None, None).expect("conv"),
            event_id: ExternalEventId::new("e-1").expect("event"),
            text: "hi".to_string(),
            trigger: ProductTriggerReason::DirectChat,
            attachments: Vec::new(),
            reply_context: Some(vec![0u8; MAX_REPLY_CONTEXT_BYTES + 1]),
        };
        assert!(matches!(
            message.validate().unwrap_err(),
            ChannelError::Parse { .. }
        ));
    }

    #[test]
    fn immediate_response_bound_is_enforced() {
        let response = ImmediateResponse {
            status: 200,
            content_type: None,
            body: vec![0u8; MAX_IMMEDIATE_RESPONSE_BYTES + 1],
        };
        assert!(response.validate().is_err());
    }
}
