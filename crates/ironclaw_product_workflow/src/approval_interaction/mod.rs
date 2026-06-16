//! Product-facing approval interaction boundary.
//!
//! This module owns the click-approval service shape used by WebUI/product
//! surfaces. It deliberately returns redacted DTOs and routes decisions through
//! injected canonical approval + turn coordination ports; it does not keep an
//! ad hoc approval queue or execute capabilities directly.

mod gate_ref;
mod read_model;
mod resolver;
mod service;
mod types;

pub use gate_ref::{approval_gate_ref, approval_request_id_from_gate_ref, is_approval_gate_ref};
pub use read_model::{
    ApprovalBlockedTurnRun, ApprovalInteractionReadModel, ApprovalTurnRunLocator,
    RunStateApprovalInteractionReadModel,
};
pub use resolver::{ApprovalLeaseTermsProvider, ApprovalResolutionPort, ApprovalResolverPort};
pub(crate) use service::RejectingApprovalInteractionService;
pub use service::{ApprovalInteractionService, DefaultApprovalInteractionService};
pub use types::{
    ApprovalGateRecord, ApprovalInteractionActionView, ApprovalInteractionDecision,
    ApprovalInteractionRejectionKind, ApprovalInteractionScope, ListPendingApprovalsRequest,
    ListPendingApprovalsResponse, PendingApprovalInteractionView,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
};

use crate::error::ProductWorkflowError;

fn approval_rejected(kind: ApprovalInteractionRejectionKind) -> ProductWorkflowError {
    ProductWorkflowError::ApprovalInteractionRejected { kind }
}
