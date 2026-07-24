//! Product-adapter contracts shared by host surfaces and turn workflow.

pub mod auth;
pub mod capabilities;
mod channel_adapter;
mod egress;
mod error;
pub mod external;
pub mod identity;
pub mod inbound;
pub mod interaction_commands;
mod outbound;
mod projection;
pub mod redaction;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use crate::ProtocolAuthFailure;
pub use crate::ProtocolHttpEgressError;
pub use auth::{AuthRequirement, ProtocolAuthEvidence, VerifiedAuthClaim};
#[cfg(feature = "host-auth-mint")]
pub use auth::{
    mark_bearer_token_verified, mark_bearer_token_verified_for_tenant,
    mark_request_signature_verified, mark_request_signature_verified_for_tenant,
    mark_session_verified, mark_session_verified_for_tenant, mark_shared_secret_header_verified,
    mark_shared_secret_header_verified_for_tenant,
};
pub use capabilities::{ProductAdapterCapabilities, ProductCapabilityFlag};
pub use channel_adapter::{
    AttachmentRef, ChannelAdapter, ChannelContext, ChannelError, DeliveryReport, ImmediateResponse,
    InboundOutcome, MAX_IMMEDIATE_RESPONSE_BYTES, MAX_REPLY_CONTEXT_BYTES,
    NormalizedInboundMessage, OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome,
    TargetCandidate, TargetQuery, VerifiedInbound,
};
pub use egress::{
    DeclaredEgressHost, DeclaredEgressTarget, DeliveryAttemptId, DeliveryStatus,
    EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath, EgressRequest, EgressResponse,
    OutboundDeliverySink, ProtocolHttpEgress,
};
pub use error::{ProductAdapterError, ProductWorkflowRejectionKind};
pub use external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
    ProductAttachmentKind,
};
pub use identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
pub use inbound::{
    ApprovalDecision, ApprovalResolutionPayload, AuthResolutionPayload, AuthResolutionResult,
    ChannelInboundClassification, InboundCommandPayload, InboundRetryDisposition,
    LinkedThreadActionPayload, ParsedProductInbound, ProductCommandResultPayload,
    ProductControlActionPayload, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionDisposition, ProductRejectionKind,
    ProductSlashCommandParseError, ProductSourceChannel, ProductTriggerReason,
    ProjectionReadPayload, ProjectionSubscriptionPayload, ScopedApprovalResolutionPayload,
    TrustedInboundContext, UserMessagePayload, parse_product_slash_command,
};
pub use interaction_commands::{parse_interaction_resolution_text, strip_wrapping_inline_code};
pub use outbound::{
    ApprovalPromptActionView, ApprovalPromptContextView, ApprovalPromptDestinationView,
    ApprovalPromptDetailView, ApprovalPromptScopeView, AuthPromptChallengeKind,
    AuthPromptContextView, AuthPromptView, CAPABILITY_DISPLAY_KIND_MAX_BYTES,
    CAPABILITY_DISPLAY_PREVIEW_MAX_BYTES, CAPABILITY_DISPLAY_RESULT_REF_MAX_BYTES,
    CAPABILITY_DISPLAY_SUMMARY_MAX_BYTES, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput,
    ConnectionPromptContext, FinalReplyView, GatePromptView, PROJECTION_SKILL_ACTIVATION_MAX_ITEMS,
    PROJECTION_SKILL_FEEDBACK_MAX_BYTES, PROJECTION_SKILL_NAME_MAX_BYTES,
    PROJECTION_TEXT_MAX_BYTES, PairingPromptView, PreferenceTargetCodec,
    PreferenceTargetEncodeRequest, ProductGateKind, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem, ProductProjectionState,
    ProductRenderOutcome, ProductSynchronousResponse, ProductWorkSummaryPhase, ProgressKind,
    ProgressUpdateView, ProjectionCursor, render_channel_auth_prompt,
};
pub use projection::{
    ProductProjectionReadInput, ProductProjectionSubject, ProductProjectionSubscribeInput,
    ProjectionReadRequest, ProjectionStream, ProjectionStreamSubscription,
    ProjectionSubscriptionRequest,
};
pub use redaction::{REDACTED_PLACEHOLDER, RedactedDebug, RedactedString};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::*;
