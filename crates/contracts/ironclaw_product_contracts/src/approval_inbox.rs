//! Cross-thread pending-approval wire contracts.
//!
//! A flat, caller-scoped view over every gate awaiting the caller's approval
//! decision, regardless of which thread parked it. Complements the per-thread
//! approval interaction service: that service resolves one gate on one
//! thread, this view lists every gate across threads so a client can show a
//! single pending-approvals surface without walking the thread list itself.

use serde::{Deserialize, Serialize};

use crate::descriptors::ProductView;

pub const APPROVALS_PENDING_VIEW: ProductView<
    ProductListPendingApprovalsRequest,
    ProductListPendingApprovalsResponse,
> = ProductView::unpaginated("approvals.pending");

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductListPendingApprovalsRequest {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProductPendingApprovalAction {
    Dispatch { capability_id: String },
    SpawnCapability { capability_id: String },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductPendingApproval {
    pub thread_id: String,
    pub run_id: String,
    pub gate_ref: String,
    pub approval_request_id: String,
    pub summary: String,
    pub action: ProductPendingApprovalAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductListPendingApprovalsResponse {
    pub approvals: Vec<ProductPendingApproval>,
}
