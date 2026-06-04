//! Product-facing auth interaction boundary.
//!
//! This module owns adapter/UI-safe auth-required interaction handling. It
//! validates caller scope, exposes redacted auth challenge DTOs, and routes
//! resume/cancel decisions through typed auth-flow and turn-coordinator ports.

mod gate_ref;
mod service;
mod types;

pub use gate_ref::is_auth_gate_ref;
pub(super) use gate_ref::{auth_reply_binding_ref, auth_source_binding_ref};
pub(crate) use service::RejectingAuthInteractionService;
pub use service::{
    AuthInteractionReadModel, AuthInteractionService, DefaultAuthInteractionService,
};
pub use types::{
    AuthCredentialAccountChoiceView, AuthGateRecord, AuthInteractionChallengeView,
    AuthInteractionDecision, AuthInteractionRejectionKind, AuthInteractionScope,
    AuthInteractionStatus, ListPendingAuthInteractionsRequest, ListPendingAuthInteractionsResponse,
    PendingAuthInteractionView, ResolveAuthInteractionRequest, ResolveAuthInteractionResponse,
};

use crate::error::ProductWorkflowError;

fn auth_rejected(kind: AuthInteractionRejectionKind) -> ProductWorkflowError {
    ProductWorkflowError::AuthInteractionRejected { kind }
}
