//! Product-facing auth interaction boundary.
//!
//! This module owns adapter/UI-safe auth-required interaction handling. It
//! validates caller scope, exposes redacted auth challenge DTOs, and routes
//! resume/cancel decisions through typed auth-flow and turn-coordinator ports.

mod service;
mod types;

pub(crate) use service::RejectingAuthInteractionService;
pub use service::{
    AuthInteractionReadModel, AuthInteractionService, DefaultAuthInteractionService,
};
pub use types::{
    AuthGateRecord, AuthInteractionChallengeView, AuthInteractionDecision,
    AuthInteractionRejectionKind, AuthInteractionScope, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, PendingAuthInteractionView, ResolveAuthInteractionRequest,
    ResolveAuthInteractionResponse, is_auth_gate_ref,
};

use crate::error::ProductWorkflowError;

fn auth_rejected(kind: AuthInteractionRejectionKind) -> ProductWorkflowError {
    ProductWorkflowError::AuthInteractionRejected { kind }
}
