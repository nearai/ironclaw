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

use ironclaw_host_api::attachment::{InboundAttachment, WorkspaceFile};
use serde::{Deserialize, Serialize};

use crate::external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
};
use crate::tool_adapter::RestrictedEgress;

/// Why an adapter is forwarding a group/supergroup/channel message into the
/// canonical pipeline.
///
/// Stamped by the adapter on every [`NormalizedInboundMessage`], and carried
/// unchanged into the product-tier inbound DTOs
/// (`ironclaw_product_contracts::inbound`) that classify it. It lives on this
/// side of the membrane because the adapter is what decides it: the product
/// tier may depend on the extension tier, never the reverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductTriggerReason {
    DirectChat,
    BotMention,
    ReplyToBot,
    BotCommand,
    LinkedThreadAction,
}

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

    /// Fetch one inbound attachment's bytes through the channel's restricted
    /// egress. The generic workflow calls this only after duplicate replay has
    /// missed and before-inbound policy has returned Allow or Rewrite, then
    /// lands the returned bytes through the canonical project filesystem path.
    async fn fetch_attachment(
        &self,
        _attachment: &ChannelAttachmentRef,
        _egress: &dyn RestrictedEgress,
    ) -> Result<InboundAttachment, ChannelError> {
        Err(ChannelError::Unsupported)
    }

    /// Fetch recent vendor-side conversation context for one inbound shared
    /// message, through the channel's restricted egress. `topic`-bearing
    /// conversations fetch that thread's messages; top-level conversations
    /// fetch recent channel history. Returns `Ok(None)` when the channel has
    /// no such capability (the default), when scopes are missing, or when the
    /// vendor call fails — context is advisory and must never fail admission.
    async fn fetch_conversation_context(
        &self,
        _conversation: &ExternalConversationRef,
        _egress: &dyn RestrictedEgress,
    ) -> Result<Option<ChannelConversationContext>, ChannelError> {
        Ok(None)
    }

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
    /// Host-resolved, manifest-declared non-secret configuration for the
    /// verified installation. Secret material remains host-side.
    pub config: &'a [(String, String)],
    /// Request body bytes (bounded by the ingress body limit).
    pub body: &'a [u8],
    /// Request headers the host chose to forward (verification headers are
    /// consumed by the host and not exposed).
    pub headers: &'a [(String, String)],
    /// This channel's declared `presentation.can_reply_in_threads`: whether a
    /// top-level shared-conversation message should be rooted as its own
    /// vendor thread (so the whole exchange threads) rather than kept flat
    /// with anchored replies. The host reads it from the resolved manifest
    /// and passes it here so an adapter's conversation-rooting honors the
    /// declaration instead of hardcoding it — a channel that declares
    /// `false` keeps replies flat even on a threading-capable vendor.
    pub can_reply_in_threads: bool,
}

/// The normalized result of parsing one inbound request.
pub enum InboundOutcome {
    /// Normalized message(s) for the workflow.
    Messages(Vec<NormalizedInboundMessage>),
    /// One fragment of a provider-level message batch. The generic host
    /// settles concurrent fragments before admitting one atomic normalized
    /// message.
    BatchFragment(Box<InboundBatchFragment>),
    /// Bounded immediate response (e.g. a URL-verification challenge).
    Respond(ImmediateResponse),
    /// Authenticated no-op (ignored event types).
    Ignore,
}

/// Maximum provider batch-key or fragment-id length accepted from an adapter.
pub const MAX_INBOUND_BATCH_REF_BYTES: usize = 512;
/// Maximum settle window an adapter may request for provider batch fragments.
pub const MAX_INBOUND_BATCH_SETTLE_MILLIS: u64 = 2_000;

/// One fragment of a provider-level message batch.
///
/// The adapter assigns every fragment in one provider batch the same
/// `batch_key` and normalized `message.event_id`, while `fragment_id` remains
/// unique per vendor delivery. `order` preserves provider order through the
/// host-owned merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundBatchFragment {
    pub batch_key: String,
    pub fragment_id: String,
    pub order: u64,
    pub settle_millis: u64,
    /// Whether this fragment independently satisfies the channel's trigger
    /// policy. The host admits the merged batch only when at least one
    /// fragment is triggered, allowing uncaptioned group-album fragments to
    /// contribute attachments without forwarding ambient group traffic.
    pub triggered: bool,
    pub message: NormalizedInboundMessage,
}

impl InboundBatchFragment {
    /// Validate untrusted adapter-supplied batching metadata and the enclosed
    /// normalized message before the host retains it.
    pub fn validate(&self) -> Result<(), ChannelError> {
        validate_batch_ref("batch_key", &self.batch_key)?;
        validate_batch_ref("fragment_id", &self.fragment_id)?;
        if self.settle_millis == 0 || self.settle_millis > MAX_INBOUND_BATCH_SETTLE_MILLIS {
            return Err(ChannelError::Parse {
                reason: format!(
                    "batch settle window must be between 1 and \
                     {MAX_INBOUND_BATCH_SETTLE_MILLIS} milliseconds"
                ),
            });
        }
        self.message.validate()
    }
}

fn validate_batch_ref(kind: &str, value: &str) -> Result<(), ChannelError> {
    if value.is_empty()
        || value.len() > MAX_INBOUND_BATCH_REF_BYTES
        || value.chars().any(|character| character.is_control())
    {
        return Err(ChannelError::Parse {
            reason: format!(
                "{kind} must be 1..={MAX_INBOUND_BATCH_REF_BYTES} bytes without control characters"
            ),
        });
    }
    Ok(())
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
    pub attachments: Vec<ChannelAttachmentRef>,
    /// Opaque per-message context (≤ 4 KiB) the host stores server-side and
    /// hands back at delivery time (reply routing). Never interpreted by the
    /// host.
    pub reply_context: Option<Vec<u8>>,
}

/// Maximum size of an inbound message's opaque `reply_context`.
pub const MAX_REPLY_CONTEXT_BYTES: usize = 4 * 1024;

/// A transient vendor attachment reference: the descriptor the message
/// declares plus the opaque provider handle used to fetch it. Bytes are
/// fetched host-side through restricted egress with the channel credential
/// only when a consumer needs them, keeping `inbound` pure.
///
/// Named distinctly from `ironclaw_common::AttachmentRef`, which is the
/// durable byte-free transcript reference — a different concept that used to
/// share this name and forced import aliases wherever both appeared.
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelAttachmentRef {
    pub descriptor: ProductAttachmentDescriptor,
    pub vendor_ref: String,
}

impl std::fmt::Debug for ChannelAttachmentRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelAttachmentRef")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// Maximum size of one [`ChannelConversationContext`] text payload.
pub const MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES: usize = 32 * 1024;

/// Recent vendor-side conversation history fetched host-side for one inbound
/// shared-channel message.
///
/// The text is UNTRUSTED third-party content (whatever other channel members
/// wrote): consumers must frame it as quoted information, never as
/// instructions, before it reaches a model. It is advisory context — absence
/// or loss never fails admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelConversationContext {
    pub text: String,
}

impl ChannelConversationContext {
    /// Validate adapter-supplied context text (the adapter is untrusted for
    /// size): non-empty and within the host byte bound.
    pub fn new(text: String) -> Result<Self, ChannelError> {
        if text.trim().is_empty() {
            return Err(ChannelError::Parse {
                reason: "conversation context text must not be empty".to_string(),
            });
        }
        if text.len() > MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES {
            return Err(ChannelError::Parse {
                reason: format!(
                    "conversation context exceeds the \
                     {MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES}-byte bound"
                ),
            });
        }
        Ok(Self { text })
    }
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
    /// A project-workspace file materialized immediately before adapter
    /// delivery. Raw bytes are transient: this part is never persisted in a
    /// delivery attempt, event, projection, or transcript.
    File(WorkspaceFile),
    /// Structured authentication challenge. The coordinator forwards this
    /// unchanged; each channel adapter owns native rendering while preserving
    /// the same recipe materialization WebUI consumes.
    AuthPrompt {
        view: Box<crate::auth_prompt::AuthPromptView>,
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
    /// Host-supplied adapter configuration is missing or invalid. Inbound
    /// routers treat this as retryable because vendor redelivery may succeed
    /// after an operator repairs configuration.
    #[error("channel configuration is unavailable: {reason}")]
    Configuration { reason: String },
    #[error("outbound rendering failed: {reason}")]
    Render { reason: String },
    #[error("vendor wiring failed: {reason}")]
    VendorWiring { reason: String },
    #[error("attachment transfer failed: {reason}")]
    AttachmentTransfer { reason: String, retryable: bool },
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
    use crate::external::ProductAttachmentKind;

    #[test]
    fn channel_attachment_ref_debug_redacts_the_vendor_reference() {
        let attachment = ChannelAttachmentRef {
            descriptor: ProductAttachmentDescriptor::new(
                "file-1",
                "application/pdf",
                Some("report.pdf".to_string()),
                Some(4),
                ProductAttachmentKind::Document,
            )
            .expect("descriptor"),
            vendor_ref: "opaque-provider-secret-reference".to_string(),
        };

        let debug = format!("{attachment:?}");
        assert!(debug.contains("file-1"));
        assert!(!debug.contains("opaque-provider-secret-reference"));
    }

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

    #[test]
    fn conversation_context_bounds_fail_closed() {
        assert!(matches!(
            ChannelConversationContext::new(String::new()),
            Err(ChannelError::Parse { .. })
        ));
        assert!(matches!(
            ChannelConversationContext::new("   \n\t".to_string()),
            Err(ChannelError::Parse { .. })
        ));
        assert!(matches!(
            ChannelConversationContext::new("x".repeat(MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES + 1)),
            Err(ChannelError::Parse { .. })
        ));
        let context = ChannelConversationContext::new("<@U1>: hello".to_string())
            .expect("bounded context text is accepted");
        assert_eq!(context.text, "<@U1>: hello");
    }

    fn valid_batch_fragment() -> InboundBatchFragment {
        InboundBatchFragment {
            batch_key: "album-1".to_string(),
            fragment_id: "message-1".to_string(),
            order: 1,
            settle_millis: 1_000,
            triggered: true,
            message: NormalizedInboundMessage {
                actor: ExternalActorRef::new("user", "u-1", None::<&str>).expect("actor"),
                conversation: ExternalConversationRef::new(None, "c-1", None, None)
                    .expect("conversation"),
                event_id: ExternalEventId::new("album-event").expect("event"),
                text: "read both".to_string(),
                trigger: ProductTriggerReason::DirectChat,
                attachments: Vec::new(),
                reply_context: None,
            },
        }
    }

    #[test]
    fn inbound_batch_metadata_bounds_fail_closed() {
        let mut fragment = valid_batch_fragment();
        assert!(fragment.validate().is_ok());

        fragment.batch_key.clear();
        assert!(matches!(
            fragment.validate(),
            Err(ChannelError::Parse { .. })
        ));

        fragment = valid_batch_fragment();
        fragment.fragment_id = "contains\ncontrol".to_string();
        assert!(matches!(
            fragment.validate(),
            Err(ChannelError::Parse { .. })
        ));

        fragment = valid_batch_fragment();
        fragment.batch_key = "x".repeat(MAX_INBOUND_BATCH_REF_BYTES + 1);
        assert!(matches!(
            fragment.validate(),
            Err(ChannelError::Parse { .. })
        ));

        fragment = valid_batch_fragment();
        fragment.settle_millis = 0;
        assert!(matches!(
            fragment.validate(),
            Err(ChannelError::Parse { .. })
        ));

        fragment = valid_batch_fragment();
        fragment.settle_millis = MAX_INBOUND_BATCH_SETTLE_MILLIS + 1;
        assert!(matches!(
            fragment.validate(),
            Err(ChannelError::Parse { .. })
        ));
    }
}
