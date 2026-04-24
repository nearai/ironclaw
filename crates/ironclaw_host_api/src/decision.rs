//! Authorization decision contracts.
//!
//! [`Decision`] is the host-facing result of evaluating an action in context:
//! allow with required [`Obligation`]s, deny with a structured [`DenyReason`],
//! or require a user approval request. Allowing an action is not enough by
//! itself; runtime services must also satisfy attached obligations such as
//! resource reservation, audit events, output limits, secret injection, and
//! scoped mounts.

use serde::{Deserialize, Serialize};

use crate::ApprovalRequest;
use crate::{MountView, NetworkPolicy, ResourceReservationId, SecretHandle};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Decision {
    Allow { obligations: Vec<Obligation> },
    Deny { reason: DenyReason },
    RequireApproval { request: ApprovalRequest },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    MissingGrant,
    InvalidPath,
    PathOutsideMount,
    UnknownCapability,
    UnknownSecret,
    NetworkDenied,
    BudgetDenied,
    ApprovalDenied,
    PolicyDenied,
    ResourceLimitExceeded,
    InternalInvariantViolation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Obligation {
    AuditBefore,
    AuditAfter,
    RedactOutput,
    ReserveResources {
        reservation_id: ResourceReservationId,
    },
    UseScopedMounts {
        mounts: MountView,
    },
    InjectSecretOnce {
        handle: SecretHandle,
    },
    ApplyNetworkPolicy {
        policy: NetworkPolicy,
    },
    EnforceOutputLimit {
        bytes: u64,
    },
}
