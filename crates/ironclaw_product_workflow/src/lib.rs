//! Product-facing workflow facade for IronClaw Reborn.
//!
//! `ironclaw_product_workflow` sits between product adapters and host-layer
//! Reborn services. It owns the product action orchestration so that adapters
//! (Web, API, CLI, Telegram, etc.) do not each reimplement binding resolution,
//! message staging, idempotency, busy/deferred handling, gate routing, mission
//! routing, and redacted acknowledgements.
//!
//! ## Key types
//!
//! - [`DefaultProductWorkflow`] — top-level orchestrator that implements
//!   [`ironclaw_product_adapters::ProductWorkflow`].
//! - [`InboundTurnService`] / [`DefaultInboundTurnService`] — the narrower
//!   user-message path that coordinates binding + turn submission.
//! - [`ConversationBindingService`] — resolves external adapter refs to
//!   canonical Reborn identifiers.
//! - [`ProductConversationBindingService`] — bridges product adapter bindings to
//!   `ironclaw_conversations` using trusted installation configuration for
//!   tenant/default scope selection.
//! - [`IdempotencyLedger`] — durable action deduplication port.
//! - [`InMemoryIdempotencyLedger`] — local-dev/test ledger with in-flight lease
//!   recovery semantics.
//! - [`ProductInboundAction`] — durable ledger record for inbound actions.

#![forbid(unsafe_code)]

mod action;
mod binding;
mod conversation_binding;
mod error;
#[cfg(any(test, feature = "test-support"))]
mod fakes;
mod in_memory_ledger;
mod inbound_turn;
mod ledger;
mod policy;
mod reborn_services;
mod webui_inbound;
mod workflow;

pub use action::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, AuthRequestRef, LinkedThreadActionId,
    ProductActionId, ProductCommandName, ProductInboundAction, SourceBindingKey,
};
pub use binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
pub use conversation_binding::{
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    StaticProductInstallationResolver,
};
pub use error::ProductWorkflowError;
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService,
};
pub use in_memory_ledger::InMemoryIdempotencyLedger;
pub use inbound_turn::{
    DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
};
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
pub use policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};
// Projection types that route handlers need to thread through SSE
// (parse the resume cursor, emit each envelope as JSON). Re-exported so
// `ironclaw_webui_v2` consumes them via the facade crate and does not need
// a direct dependency on `ironclaw_product_adapters` — the single-facade
// boundary is enforced by `ironclaw_architecture`.
pub use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
// Re-exported so the WebUI v2 handler crate can validate the
// `extension_name` path segment at the handler/facade boundary
// without pulling `ironclaw_common` into its forbidden-imports set.
pub use ironclaw_common::ExtensionName;
pub use reborn_services::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornListThreadsResponse, RebornResolveGateResponse,
    RebornResumeGateResponse, RebornServices, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, RebornSetupExtensionResponse,
    RebornSetupExtensionStatus, RebornStreamEventsRequest, RebornStreamEventsResponse,
    RebornSubmitTurnResponse, RebornTimelineRequest, RebornTimelineResponse,
};
pub use webui_inbound::{
    WebUiAuthenticatedCaller, WebUiCancelReason, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiGateResolution, WebUiInboundCommand, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiListThreadsRequest, WebUiResolveGateRequest,
    WebUiSendMessageRequest, WebUiSetupExtensionRequest,
};
pub use workflow::DefaultProductWorkflow;
