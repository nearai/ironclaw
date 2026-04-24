//! Approval contracts for user-mediated authority.
//!
//! Approval is a scoped grant to continue a specific action, not a vague
//! confirmation. [`ApprovalRequest`] carries the exact action that needs a
//! decision and may optionally describe a reusable [`ApprovalScope`] such as a
//! capability, path prefix, or network target. Matching must be exact or
//! policy-defined by the host; callers must not infer broader authority from a
//! one-off approval.

use serde::{Deserialize, Serialize};

use crate::{
    Action, ApprovalRequestId, CapabilityId, CorrelationId, NetworkTargetPattern, Principal,
    ScopedPath, Timestamp,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalRequestId,
    pub correlation_id: CorrelationId,
    pub requested_by: Principal,
    pub action: Box<Action>,
    pub reason: String,
    pub reusable_scope: Option<ApprovalScope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalScope {
    pub principal: Principal,
    pub action_pattern: ActionPattern,
    pub expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ActionPattern {
    ExactAction {
        action: Box<Action>,
    },
    Capability {
        capability: CapabilityId,
    },
    PathPrefix {
        action_kind: FileActionKind,
        prefix: ScopedPath,
    },
    NetworkTarget {
        target: NetworkTargetPattern,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileActionKind {
    Read,
    List,
    Write,
    Delete,
}
