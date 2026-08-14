//! The generic channel capability contracts (overview.md §4.2).
//!
//! An extension package is a protocol translator and may implement any subset
//! of three one-way methods: receive a complete vendor message, send a reply,
//! and deliver out of band. The host owns everything around those translations
//! (route table, verification recipes, replay, admission, target policy,
//! attempt persistence, retry, drain). It also owns authenticated-session
//! ingress and stream replies, so those manifest modes intentionally require
//! no adapter implementation. The package never reports metadata (the resolved
//! manifest is the authority) and never touches the delivery store.
//!
//! These DTOs are the seam between generic host pipelines and concrete
//! protocol crates; the old metadata-carrying `ProductAdapter` is retired as
//! its callers cut over (implementation.md §5).

use std::sync::Arc;

use async_trait::async_trait;

use ironclaw_host_api::attachment::{InboundAttachment, WorkspaceFile};
use serde::{Deserialize, Serialize};

use crate::external::{
    ExternalActorId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
    ProductAttachmentDescriptor,
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

/// **Ingress** — how a vendor request becomes complete input. Pairs with a
/// webhook/vendor `[channel.ingress]`.
///
/// `authenticated_session` ingress is normalized by the host's session door
/// and binds no implementation here. For every other ingress mode,
/// `check_binding` requires this half at activation.
#[async_trait]
pub trait ChannelIngress: Send + Sync {
    /// Translate one host-verified vendor request into a complete normalized
    /// outcome. Any attachment bytes or vendor-side conversation context are
    /// resolved here through the manifest-restricted egress before the
    /// adapter returns; the host never calls back into an adapter to finish a
    /// half-normalized message.
    async fn receive(
        &self,
        request: VerifiedInbound<'_>,
        egress: &dyn RestrictedEgress,
    ) -> Result<InboundOutcome, ChannelError>;
}

/// **Reply** — answering the run's input, source-routed. Pairs with
/// `[channel.reply]`.
///
/// A channel declaring `transport = "stream"` implements **nothing here**:
/// the host publishes to the durable projection pipeline and the adapter is
/// never called. That absence is meaningful rather than a mystery — it is
/// what `stream` means.
#[async_trait]
pub trait ChannelReply: Send + Sync {
    /// Render and send one run answer back to where its input came from.
    /// Owns vendor formatting, provider-specific message splitting, target
    /// syntax, and safe error mapping. Never
    /// touches the delivery store.
    async fn send_reply(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError>;
}

/// **Delivery** — reaching someone out of band, target-resolved. Pairs with
/// `[channel.delivery]`.
///
/// Orthogonal to [`ChannelReply`], not an alternative: a channel may
/// implement both (one run streams an answer into an open tab *and* fires a
/// push), either, or neither.
#[async_trait]
pub trait ChannelDelivery: Send + Sync {
    /// Render and send one out-of-band delivery to an already-resolved,
    /// already-authorized target. The coordinator decided the axis and the
    /// target; this renders and sends.
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError>;

    /// Optional: provision the direct conversation for one proven external
    /// actor. This is deliberately not target search; the host supplies the
    /// typed actor and the adapter returns at most that actor's conversation.
    async fn provision_direct_target(
        &self,
        _request: DirectTargetProvisionRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<Option<ExternalConversationRef>, ChannelError> {
        Err(ChannelError::Unsupported)
    }
}

/// The halves one extension's channel surface actually implements.
///
/// Eleven methods on one trait became three core translation methods across
/// three traits plus one optional, typed direct-target provisioning hook. This
/// is what the host holds instead of a single `Arc<dyn ChannelAdapter>` whose
/// unsupported methods were discovered at call time. A `None` here is the
/// required fact for host-owned modes (`authenticated_session` ingress and
/// `stream` reply); `check_binding` proves every manifest axis agrees with its
/// implementation at activation.
///
/// What left the trait entirely, and where it went:
/// - `activate`/`cleanup` → `[channel.ingress.registration]` /
///   `[channel.ingress.deregistration]` recipes the host runs through
///   existing restricted egress with existing credential injection.
/// - notification delivery → [`ChannelDelivery::deliver`]. One run can stream an answer
///   into an open tab *and* fire a push; those are two axes, not two intents.
/// - the three notification-setup methods → host-owned per-user delivery
///   registrations, so the host can answer "is this user set up?" before a
///   send instead of discovering it inside the vendor path.
///
/// Ingress has the same complete-envelope shape as output: one translator
/// method each way. [`ChannelIngress::receive`] resolves vendor handles before
/// returning, while the generic host keeps validation, policy, persistence,
/// and turn orchestration.
#[derive(Clone, Default)]
pub struct ChannelSurfaces {
    pub ingress: Option<Arc<dyn ChannelIngress>>,
    pub reply: Option<Arc<dyn ChannelReply>>,
    pub delivery: Option<Arc<dyn ChannelDelivery>>,
}

impl ChannelSurfaces {
    pub fn with_ingress(mut self, ingress: Arc<dyn ChannelIngress>) -> Self {
        self.ingress = Some(ingress);
        self
    }

    pub fn with_reply(mut self, reply: Arc<dyn ChannelReply>) -> Self {
        self.reply = Some(reply);
        self
    }

    pub fn with_delivery(mut self, delivery: Arc<dyn ChannelDelivery>) -> Self {
        self.delivery = Some(delivery);
        self
    }

    /// Whether this surface set emits anything at all.
    pub fn has_outbound(&self) -> bool {
        self.reply.is_some() || self.delivery.is_some()
    }
}

impl std::fmt::Debug for ChannelSurfaces {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelSurfaces")
            .field("ingress", &self.ingress.is_some())
            .field("reply", &self.reply.is_some())
            .field("delivery", &self.delivery.is_some())
            .finish()
    }
}

/// One per-user delivery registration, as the adapter sees it at delivery.
///
/// This is what replaced the three notification-setup adapter methods
/// (design §8). The host owns the records; the adapter parses one at the
/// moment it needs the endpoint and the key material anyway, so there is no
/// `validate_enrollment` and no setup surface on any trait.
///
/// **Two fields, two trust stories.** `endpoint` is host-visible on purpose —
/// the host checks it against the channel's declared `[[channel.egress]]`
/// hosts *before storage*, because without that check enrollment is an SSRF
/// primitive that makes the host POST wherever an attacker names. `document`
/// is channel-opaque: the host bounds its size and never interprets it, and a
/// malformed one fails that single delivery and is pruned on the same path
/// that already prunes an expired endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryRegistration {
    /// Host-minted opaque record identity. Stable across a refresh of the
    /// same registration; never derived from provider addressing. Adapters
    /// echo it back in [`DeliveryReport::prune_registrations`]; they never
    /// mint one.
    pub registration_id: String,
    /// The absolute URL this registration delivers to. Host-checked against
    /// the channel's declared egress hosts before it is ever stored.
    pub endpoint: String,
    /// Channel-opaque remainder (key material, client metadata). Bounded by
    /// [`MAX_DELIVERY_REGISTRATION_DOCUMENT_BYTES`]; never interpreted by
    /// generic code.
    pub document: String,
    /// RFC 3339, host-stamped at first storage.
    pub created_at: String,
}

/// Bound on one registration's opaque `document`.
pub const MAX_DELIVERY_REGISTRATION_DOCUMENT_BYTES: usize = 16 * 1024;
/// Bound on one registration's `endpoint` URL.
pub const MAX_DELIVERY_REGISTRATION_ENDPOINT_BYTES: usize = 2 * 1024;
/// Bound on how many registrations one user may hold per channel.
pub const MAX_DELIVERY_REGISTRATIONS_PER_USER: usize = 20;

/// One host-verified inbound request. Signing secrets are never in scope —
/// the host executed the verification recipe before calling `receive`.
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
    /// Complete, channel-neutral attachment bytes. Provider descriptors and
    /// download handles are edge-only parse state; the adapter must reconcile
    /// them before returning this canonical host attachment.
    pub attachments: Vec<InboundAttachment>,
    /// Recent vendor-side history when the adapter knows this message came
    /// from a shared conversation and the vendor exposes a history API.
    /// Untrusted content: the host sanitizes and frames it before model use.
    pub conversation_context: Option<ChannelConversationContext>,
    /// Opaque per-message context (≤ 4 KiB) the host stores server-side and
    /// hands back at delivery time (reply routing). Never interpreted by the
    /// host.
    pub reply_context: Option<Vec<u8>>,
}

/// Maximum size of an inbound message's opaque `reply_context`.
pub const MAX_REPLY_CONTEXT_BYTES: usize = 4 * 1024;

/// Package-internal parse/fetch state: the descriptor a vendor payload
/// declares plus the opaque provider handle the adapter uses while completing
/// [`ChannelIngress::receive`]. This is not part of the host inbound contract
/// and must never cross adapter admission.
///
/// Named distinctly from `ironclaw_common::AttachmentRef`, which is the
/// durable byte-free transcript reference — a different concept that used to
/// share this name and forced import aliases wherever both appeared.
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelAttachmentRef {
    pub descriptor: ProductAttachmentDescriptor,
    pub vendor_ref: String,
}

impl ChannelAttachmentRef {
    /// Reconcile provider-declared metadata with the fetched bytes at the
    /// adapter edge, then discard the provider handle and descriptor. The
    /// internal host receives exactly one canonical attachment shape.
    pub fn complete(self, fetched: InboundAttachment) -> Result<InboundAttachment, ChannelError> {
        if fetched.id != self.descriptor.external_file_id {
            return Err(ChannelError::Parse {
                reason: format!(
                    "fetched attachment id `{}` does not match descriptor `{}`",
                    fetched.id, self.descriptor.external_file_id
                ),
            });
        }
        if let Some(declared_size) = self.descriptor.size_bytes
            && declared_size != fetched.bytes.len() as u64
        {
            return Err(ChannelError::Parse {
                reason: format!(
                    "attachment `{}` declared {declared_size} bytes but fetched {} bytes",
                    fetched.id,
                    fetched.bytes.len()
                ),
            });
        }
        Ok(fetched)
    }
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

/// Recent vendor-side conversation history fetched by the adapter through
/// manifest-restricted egress for one inbound shared-channel message.
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
        let context = Self { text };
        context.validate()?;
        Ok(context)
    }

    /// Reapply the context invariant at a host boundary. The field remains
    /// public for adapter construction, so callers must not assume `new` was
    /// used by an untrusted implementation.
    pub fn validate(&self) -> Result<(), ChannelError> {
        if self.text.trim().is_empty() {
            return Err(ChannelError::Parse {
                reason: "conversation context text must not be empty".to_string(),
            });
        }
        if self.text.len() > MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES {
            return Err(ChannelError::Parse {
                reason: format!(
                    "conversation context exceeds the \
                     {MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES}-byte bound"
                ),
            });
        }
        Ok(())
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
    /// Resolved target (source-route reply or preference target).
    pub target: OutboundTarget,
    /// The rendered message parts, already reduced from the semantic intent by
    /// the coordinator.
    pub parts: Vec<OutboundPart>,
    /// The stored `reply_context` from the originating inbound message, if
    /// this delivery replies to one.
    pub reply_context: Option<Vec<u8>>,
    /// The recipient's per-user delivery registrations, resolved host-side
    /// (design §8). Empty for every channel that declares no
    /// `requires_enrollment` — and for one that does, the coordinator resolves
    /// zero registrations to a "no target" outcome *before* calling the
    /// adapter, so a non-empty list is what an enrollment-gated adapter can
    /// rely on rather than discovering the emptiness inside the vendor path.
    #[allow(clippy::doc_markdown)]
    pub registrations: Vec<DeliveryRegistration>,
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
    /// Add or remove a run-lifecycle reaction on an existing vendor message —
    /// typically the inbound message that triggered the run, so a channel with
    /// several runs in flight shows which message each one is working on.
    /// `vendor_message_ref` is the target message's vendor id (a source
    /// message's `reply_target_message_id`, or a bot message ref). Each adapter
    /// maps the neutral [`RunReaction`] to a vendor-safe emoji — a raw emoji
    /// would be unsafe because some vendors' reaction APIs only accept a fixed
    /// emoji allowlist. Best-effort: a failed reaction never fails the run.
    React {
        vendor_message_ref: String,
        reaction: RunReaction,
        action: ReactionAction,
    },
}

/// A neutral run-lifecycle reaction. The adapter owns the vendor emoji so the
/// mapping stays inside each vendor's allowed-reaction set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunReaction {
    /// The run is actively working on the message (👀).
    Working,
    /// The run finished successfully (✅ / vendor equivalent).
    Done,
    /// The run is parked waiting on the user — an approval or auth prompt
    /// (⚠️ / vendor equivalent).
    NeedsInput,
    /// The run failed or timed out (❌ / vendor equivalent).
    Failed,
}

/// Whether a [`OutboundPart::React`] adds or clears a reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionAction {
    Add,
    Remove,
}

/// Structured per-attempt delivery report. The adapter cannot mark anything
/// delivered in a store; it only describes what the vendor did.
#[derive(Debug, Clone, Default)]
pub struct DeliveryReport {
    pub parts: Vec<PartDeliveryOutcome>,
    /// Registration ids the vendor reported gone (an expired push
    /// subscription, a revoked device). The **host** prunes them — the
    /// adapter reports, exactly as it reports part outcomes without touching
    /// the delivery store. Ids the host did not hand this adapter in
    /// [`OutboundEnvelope::registrations`] are ignored.
    pub prune_registrations: Vec<String>,
}

impl DeliveryReport {
    /// The common case: per-part outcomes with nothing to prune.
    pub fn from_parts(parts: Vec<PartDeliveryOutcome>) -> Self {
        Self {
            parts,
            prune_registrations: Vec::new(),
        }
    }
}

/// The outcome of delivering one part.
#[derive(Debug, Clone)]
pub enum PartDeliveryOutcome {
    /// Delivered; the vendor message reference, when the protocol returns one.
    Sent { vendor_message_ref: Option<String> },
    /// Transient failure; the coordinator may retry.
    Retryable { reason: String },
    /// The request crossed into transport, but the adapter cannot prove
    /// whether the vendor accepted it. The coordinator must persist an
    /// `Unknown` attempt and never retry blindly because doing so can
    /// duplicate a user-visible message.
    Ambiguous { reason: String },
    /// Permanent failure; the coordinator will not retry.
    Permanent { reason: String },
    /// The vendor rejected authorization; the coordinator raises re-auth.
    Unauthorized { reason: String },
}

/// Request to provision one proven actor's direct conversation.
#[derive(Debug, Clone)]
pub struct DirectTargetProvisionRequest {
    pub actor_id: ExternalActorId,
}

/// Typed channel-adapter failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
/// Channel capability failures, split by what vendor redelivery can do about
/// them. Ingress answers **5xx** only for the transient variants
/// (`Configuration`, `VendorWiring`, `AttachmentTransfer { retryable: true }`)
/// — redelivering the same update later may genuinely succeed. Every other
/// variant is deterministic for a given payload: redelivery replays the same
/// bytes into the same failure, and vendors with strictly ordered webhook
/// redelivery re-send any non-2xx update and hold every later update in
/// the conversation behind it. Ingress therefore acknowledges (2xx),
/// discards, and warn-logs deterministic failures instead of rejecting them.
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
    /// `retryable` decides the ingress disposition: `true` means a transient
    /// transfer fault (5xx, vendor may redeliver with success), `false` means
    /// the transfer can never succeed for this payload (adapters degrade or
    /// ingress acknowledges-and-discards).
    #[error("attachment transfer failed: {reason}")]
    AttachmentTransfer { reason: String, retryable: bool },
    #[error("channel operation is not supported by this adapter")]
    Unsupported,
}

impl NormalizedInboundMessage {
    /// Validate host-enforceable bounds on a normalized message before it
    /// enters the workflow (the adapter is untrusted for size).
    pub fn validate(&self) -> Result<(), ChannelError> {
        let mut attachment_ids = std::collections::HashSet::new();
        for attachment in &self.attachments {
            if !attachment_ids.insert(attachment.id.as_str()) {
                return Err(ChannelError::Parse {
                    reason: format!("duplicate attachment external_file_id `{}`", attachment.id),
                });
            }
        }
        if let Some(context) = &self.reply_context
            && context.len() > MAX_REPLY_CONTEXT_BYTES
        {
            return Err(ChannelError::Parse {
                reason: "reply_context exceeds the 4 KiB bound".to_string(),
            });
        }
        if let Some(context) = &self.conversation_context {
            context.validate()?;
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
    use ironclaw_host_api::attachment::InboundAttachment;

    fn normalized_attachment(id: &str, bytes: &[u8]) -> InboundAttachment {
        InboundAttachment {
            id: id.to_string(),
            mime_type: "application/pdf".to_string(),
            filename: Some("vendor-name.pdf".to_string()),
            bytes: bytes.to_vec(),
        }
    }

    fn pending_attachment(id: &str, size: u64) -> ChannelAttachmentRef {
        ChannelAttachmentRef {
            descriptor: ProductAttachmentDescriptor::new(
                id,
                "application/pdf",
                Some(format!("{id}.pdf")),
                Some(size),
                ProductAttachmentKind::Document,
            )
            .expect("descriptor"),
            vendor_ref: "provider-handle".to_string(),
        }
    }

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
    fn normalized_attachment_debug_redacts_fetched_bytes() {
        let attachment = normalized_attachment("file-1", b"complete-byte-sentinel-secret");

        let debug = format!("{attachment:?}");
        assert!(debug.contains("file-1"));
        assert!(debug.contains("size_bytes"));
        assert!(debug.contains("29"));
        assert!(!debug.contains("complete-byte-sentinel-secret"));
        assert!(!debug.contains("99, 111, 109, 112"));
    }

    #[test]
    fn complete_attachment_shape_rejects_duplicate_ids_and_declared_size_mismatch() {
        let valid = || NormalizedInboundMessage {
            actor: ExternalActorRef::new("user", "u-1", None::<&str>).expect("actor"),
            conversation: ExternalConversationRef::new(None, "c-1", None, None)
                .expect("conversation"),
            event_id: ExternalEventId::new("e-1").expect("event"),
            text: "hi".to_string(),
            trigger: ProductTriggerReason::DirectChat,
            attachments: vec![normalized_attachment("file-1", b"data")],
            conversation_context: None,
            reply_context: None,
        };

        assert!(valid().validate().is_ok());

        let mut duplicate = valid();
        duplicate
            .attachments
            .push(normalized_attachment("file-1", b"data"));
        assert!(matches!(
            duplicate.validate(),
            Err(ChannelError::Parse { reason }) if reason.contains("duplicate")
        ));

        let wrong_size =
            pending_attachment("file-1", 5).complete(normalized_attachment("file-1", b"data"));
        assert!(matches!(
            wrong_size,
            Err(ChannelError::Parse { reason }) if reason.contains("declared")
        ));

        let wrong_id = pending_attachment("file-1", 4)
            .complete(normalized_attachment("different-file", b"data"));
        assert!(matches!(
            wrong_id,
            Err(ChannelError::Parse { reason }) if reason.contains("does not match")
        ));
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
            conversation_context: None,
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

        let message = NormalizedInboundMessage {
            actor: ExternalActorRef::new("user", "u-1", None::<&str>).expect("actor"),
            conversation: ExternalConversationRef::new(None, "c-1", None, None)
                .expect("conversation"),
            event_id: ExternalEventId::new("e-1").expect("event"),
            text: "hi".to_string(),
            trigger: ProductTriggerReason::DirectChat,
            attachments: Vec::new(),
            // Bypass `new` the same way an untrusted adapter can today: the
            // host must reapply the invariant at message validation.
            conversation_context: Some(ChannelConversationContext {
                text: "x".repeat(MAX_CHANNEL_CONVERSATION_CONTEXT_BYTES + 1),
            }),
            reply_context: None,
        };
        assert!(matches!(
            message.validate(),
            Err(ChannelError::Parse { reason }) if reason.contains("conversation context")
        ));
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
                conversation_context: None,
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
