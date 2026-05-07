//! Product-adapter contracts for IronClaw Reborn.
//!
//! This crate defines the boundary between channel/transport-specific code and
//! the canonical Reborn pipeline ([`ironclaw_turns::TurnCoordinator`] via the
//! [`ProductWorkflow`] facade). Concrete adapters (Telegram v2, Slack v2, Web,
//! CLI, API) live in separate crates/components and depend on this contract.
//!
//! See `CLAUDE.md` for the full guardrail list. The high-level shape is:
//!
//! ```text
//! protocol event
//!   -> host verifies protocol auth (mints ProtocolAuthEvidence::Verified)
//!   -> adapter parses payload into ProductInboundEnvelope
//!   -> ProductWorkflow resolves canonical actor/thread, dedupes by external_event_id,
//!      stages attachments, and submits/defers/rejects via TurnCoordinator
//!   -> ProductInboundAck returned to adapter (used to choose protocol response code)
//!
//! projection update
//!   -> ProductOutboundEnvelope (FinalReply / Progress / GatePrompt / ...)
//!   -> adapter renders + delivers via ProtocolHttpEgress
//!   -> OutboundDeliverySink records DeliveryStatus
//! ```

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
pub use auth::{
    AuthRequirement, HostAuthSeal, ProtocolAuthEvidence, ProtocolAuthFailure, VerifiedAuthClaim,
};
pub use capabilities::{ProductAdapterCapabilities, ProductCapabilityFlag};
pub use egress::{
    DeclaredEgressHost, DeliveryAttemptId, DeliveryStatus, EgressCredentialHandle, EgressRequest,
    EgressResponse, OutboundDeliverySink, ProtocolHttpEgress, ProtocolHttpEgressError,
};
pub use error::ProductAdapterError;
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
    InboundCommandPayload, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductTriggerReason, UserMessagePayload,
};
pub use outbound::{
    AuthPromptView, FinalReplyView, GatePromptView, ProductOutboundEnvelope,
    ProductOutboundPayload, ProgressKind, ProgressUpdateView, ProjectionCursor, ProjectionSnapshot,
    ProjectionUpdate,
};
pub use projection::{ProjectionStream, ProjectionSubscriptionRequest};
pub use redaction::{REDACTED_PLACEHOLDER, RedactedDebug, RedactedString};
pub use workflow::ProductWorkflow;
