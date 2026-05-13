//! Product-adapter contracts for IronClaw Reborn.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod auth;
pub mod capabilities;
pub mod egress;
pub mod error;
pub mod external;
#[cfg(any(test, feature = "test-support"))]
pub mod fakes;
pub mod identity;
pub mod inbound;
pub mod outbound;
pub mod projection;
pub mod redaction;
pub mod workflow;

pub use adapter::{ProductAdapter, ProductAdapterHealth};
pub use auth::{AuthRequirement, ProtocolAuthEvidence, ProtocolAuthFailure, VerifiedAuthClaim};
#[cfg(feature = "host-auth-mint")]
pub use auth::{
    mark_bearer_token_verified, mark_request_signature_verified, mark_session_verified,
    mark_shared_secret_header_verified,
};
pub use capabilities::{ProductAdapterCapabilities, ProductCapabilityFlag};
pub use egress::{
    DeclaredEgressHost, DeclaredEgressTarget, DeliveryAttemptId, DeliveryStatus,
    EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath, EgressRequest, EgressResponse,
    OutboundDeliverySink, ProtocolHttpEgress, ProtocolHttpEgressError,
};
pub use error::{ProductAdapterError, ProductWorkflowRejectionKind};
pub use external::{
    ExternalActorRef, ExternalConversationRef, ExternalEventId, ProductAttachmentDescriptor,
    ProductAttachmentKind,
};
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeOutboundDeliverySink, FakeProductWorkflow, FakeProjectionStream, FakeProtocolHttpEgress,
    RecordedEgressCall,
};
pub use identity::{AdapterInstallationId, ProductAdapterId, ProductSurfaceKind};
pub use inbound::{
    ApprovalDecision, ApprovalResolutionPayload, AuthResolutionPayload, AuthResolutionResult,
    InboundCommandPayload, InboundRetryDisposition, LinkedThreadActionId,
    LinkedThreadActionPayload, LoopGateRef, MissionActionPayload, MissionFireRef,
    MissionFireSuppressionReason, ParsedProductInbound, ProductCommandName, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductRejectionDisposition,
    ProductRejectionKind, ProductTriggerReason, ProjectionSubscriptionPayload, SystemActionPayload,
    TrustedInboundContext, UserMessagePayload,
};
pub use outbound::{
    AuthPromptView, FinalReplyView, GatePromptView, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem, ProductProjectionState,
    ProductRenderOutcome, ProductSynchronousResponse, ProgressKind, ProgressUpdateView,
    ProjectionCursor,
};
pub use projection::{ProjectionStream, ProjectionSubscriptionRequest};
pub use redaction::{REDACTED_PLACEHOLDER, RedactedDebug, RedactedString};
pub use workflow::ProductWorkflow;
