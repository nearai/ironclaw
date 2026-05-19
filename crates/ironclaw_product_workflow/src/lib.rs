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
//! - [`IdempotencyLedger`] — durable action deduplication port.
//! - [`ProductInboundAction`] — durable ledger record for inbound actions.

#![forbid(unsafe_code)]

mod action;
mod binding;
mod error;
#[cfg(any(test, feature = "test-support"))]
mod fakes;
mod inbound_turn;
mod ledger;
mod reborn_services;
mod webui_inbound;
mod workflow;

pub use action::{
    ActionDispatchKind, ActionFingerprintKey, ActionPhase, AuthRequestRef, LinkedThreadActionId,
    ProductActionId, ProductCommandName, ProductInboundAction, SourceBindingKey,
};
pub use binding::{ConversationBindingService, ResolveBindingRequest, ResolvedBinding};
pub use error::ProductWorkflowError;
#[cfg(any(test, feature = "test-support"))]
pub use fakes::{FakeConversationBindingService, FakeIdempotencyLedger, FakeInboundTurnService};
pub use inbound_turn::{DefaultInboundTurnService, InboundTurnOutcome, InboundTurnService};
pub use ledger::{IdempotencyDecision, IdempotencyLedger};
// Projection types that route handlers need to thread through SSE
// (parse the resume cursor, emit each envelope as JSON). Re-exported so
// `ironclaw_webui_v2` consumes them via the facade crate and does not need
// a direct dependency on `ironclaw_product_adapters` — the single-facade
// boundary is enforced by `ironclaw_architecture`.
pub use ironclaw_product_adapters::{ProductOutboundEnvelope, ProjectionCursor};
pub use reborn_services::{
    RebornCancelRunResponse, RebornCreateThreadResponse, RebornGetRunStateRequest,
    RebornGetRunStateResponse, RebornResolveGateResponse, RebornResumeGateResponse, RebornServices,
    RebornServicesApi, RebornServicesError, RebornServicesErrorCode, RebornStreamEventsRequest,
    RebornStreamEventsResponse, RebornSubmitTurnResponse, RebornTimelineRequest,
    RebornTimelineResponse,
};
pub use webui_inbound::{
    WebUiAuthenticatedCaller, WebUiCancelReason, WebUiCancelRunRequest, WebUiCreateThreadRequest,
    WebUiGateResolution, WebUiInboundCommand, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiResolveGateRequest, WebUiSendMessageRequest,
};
pub use workflow::DefaultProductWorkflow;
