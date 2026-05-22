//! Approval interaction surface.
//!
//! `ApprovalInteractionService` is the adapter/UI-safe boundary for listing
//! pending approval gates and resolving them. It composes
//! [`ironclaw_run_state::ApprovalRequestStore`] (read pending records, scoped)
//! with the host-supplied [`ApprovalDecisionPort`] (typically an
//! [`ironclaw_approvals::ApprovalResolver`]) so that:
//!
//! - product/UI surfaces receive only redacted [`PendingApprovalSummary`] DTOs
//!   without raw tool input, approval reasons, invocation fingerprints,
//!   lease IDs, host paths, secrets, or runtime output;
//! - approve/deny decisions are validated against the persisted record
//!   (scope match, pending status) before being routed to the resolver;
//! - wrong-scope reads and resolutions look unknown, never leak the existence
//!   of another tenant/user/agent/project/thread's records.

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolutionError, DenyApproval, LeaseApproval};
use ironclaw_host_api::{ApprovalRequestId, CapabilityId, Principal, ResourceScope};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, RunStateError};
use thiserror::Error;

/// Redacted summary of a pending approval gate.
///
/// Carries enough information for a product surface to render
/// "Approve <capability> requested by <principal>?" without exposing any
/// fields the readiness contract treats as sensitive. The underlying
/// `ApprovalRecord` is the source of truth; this DTO is a projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApprovalSummary {
    /// Opaque approval-request identifier used to address subsequent
    /// approve/deny calls. Wire-stable.
    pub request_id: ApprovalRequestId,
    /// Capability the action targets. Capability IDs are public manifest
    /// vocabulary; including them here is intentional so the product can
    /// say "Allow `notion.search_pages`?".
    pub capability: CapabilityId,
    /// Principal that requested the action. Identity, not authority.
    pub requested_by: Principal,
}

/// Caller-facing approve/deny error taxonomy.
///
/// Deliberately stable and coarse: products render these by variant kind
/// and never surface inner backend diagnostics or store-specific strings.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApprovalInteractionError {
    /// No approval request matches `(scope, request_id)`. Returned both for
    /// genuinely missing records and for cross-scope lookups — wrong scope
    /// must not leak the existence of another scope's records.
    #[error("approval request is unknown")]
    Unknown,
    /// The approval is no longer pending (already approved, denied, or
    /// expired). Stable irrespective of which terminal status applies.
    #[error("approval request is not pending")]
    NotPending,
    /// The persisted record is missing data required to issue a lease
    /// (e.g., the invocation fingerprint). Distinct from `NotPending`
    /// because the request never reaches the resolver.
    #[error("approval request is incomplete")]
    Incomplete,
    /// Any other backend or resolution failure. Stable category; inner
    /// detail is not part of the user-visible surface.
    #[error("approval interaction failed")]
    Backend,
}

impl From<RunStateError> for ApprovalInteractionError {
    fn from(error: RunStateError) -> Self {
        match error {
            RunStateError::UnknownApprovalRequest { .. } => Self::Unknown,
            RunStateError::ApprovalNotPending { .. } => Self::NotPending,
            _ => Self::Backend,
        }
    }
}

impl From<ApprovalResolutionError> for ApprovalInteractionError {
    fn from(error: ApprovalResolutionError) -> Self {
        match error {
            ApprovalResolutionError::NotPending { .. } => Self::NotPending,
            ApprovalResolutionError::NotApproved { .. } => Self::NotPending,
            ApprovalResolutionError::MissingInvocationFingerprint => Self::Incomplete,
            ApprovalResolutionError::UnsupportedAction => Self::Incomplete,
            ApprovalResolutionError::RunState(inner) => inner.into(),
            ApprovalResolutionError::Lease(_) => Self::Backend,
        }
    }
}

/// Typed decision port the interaction service routes approve/deny through.
///
/// Production wires this to [`ironclaw_approvals::ApprovalResolver`]; tests
/// substitute a fake to drive resolver-side outcomes without coupling to
/// lease internals.
///
/// The port deliberately accepts the [`LeaseApproval`] / [`DenyApproval`]
/// shape from `ironclaw_approvals` so the interaction layer does not
/// re-invent grant attenuation vocabulary. The resulting lease (on success)
/// is held by the resolver — the interaction service never sees a
/// `CapabilityGrantId` and so cannot leak one to product surfaces.
#[async_trait]
pub trait ApprovalDecisionPort: Send + Sync {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ApprovalResolutionError>;

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ApprovalResolutionError>;
}

/// Adapter/UI-safe approval interaction surface.
///
/// Constructed with a scoped [`ApprovalRequestStore`] and a typed
/// [`ApprovalDecisionPort`]. Every operation is `ResourceScope`-validated;
/// approve/deny additionally re-checks the persisted record's pending
/// status before routing to the port, so an attacker-supplied stale
/// `request_id` cannot resurrect a closed record.
pub struct ApprovalInteractionService<'a, S, D>
where
    S: ApprovalRequestStore + ?Sized,
    D: ApprovalDecisionPort + ?Sized,
{
    approvals: &'a S,
    decisions: &'a D,
}

impl<'a, S, D> ApprovalInteractionService<'a, S, D>
where
    S: ApprovalRequestStore + ?Sized,
    D: ApprovalDecisionPort + ?Sized,
{
    pub fn new(approvals: &'a S, decisions: &'a D) -> Self {
        Self {
            approvals,
            decisions,
        }
    }

    /// List pending approvals visible to `scope`.
    ///
    /// Filters the durable record set down to `Pending` status and projects
    /// each record into a [`PendingApprovalSummary`]. Non-pending records
    /// (approved, denied, expired) are not exposed: the interaction layer
    /// is a live-gate surface, not a historical viewer. Records whose
    /// action is not capability-shaped (i.e. not `Dispatch` /
    /// `SpawnCapability`) are also excluded — the resolver cannot act on
    /// them via this surface, so surfacing them would mislead the caller.
    pub async fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<PendingApprovalSummary>, ApprovalInteractionError> {
        let records = self.approvals.records_for_scope(scope).await?;
        let summaries = records
            .into_iter()
            .filter(|record| record.status == ApprovalStatus::Pending)
            .filter_map(|record| {
                let capability = capability_id_for_action(record.request.action.as_ref())?;
                Some(PendingApprovalSummary {
                    request_id: record.request.id,
                    capability: capability.clone(),
                    requested_by: record.request.requested_by,
                })
            })
            .collect();
        Ok(summaries)
    }

    /// Approve a pending approval request.
    ///
    /// Validates `(scope, request_id)` against the durable record before
    /// routing to the decision port. If the record is missing, returns
    /// [`ApprovalInteractionError::Unknown`] — wrong-scope lookups produce
    /// the same error so the caller cannot probe other scopes.
    pub async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ApprovalInteractionError> {
        let record = self
            .approvals
            .get(scope, request_id)
            .await?
            .ok_or(ApprovalInteractionError::Unknown)?;
        if record.status != ApprovalStatus::Pending {
            return Err(ApprovalInteractionError::NotPending);
        }
        self.decisions
            .approve_dispatch(scope, request_id, approval)
            .await?;
        Ok(())
    }

    /// Deny a pending approval request.
    pub async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ApprovalInteractionError> {
        let record = self
            .approvals
            .get(scope, request_id)
            .await?
            .ok_or(ApprovalInteractionError::Unknown)?;
        if record.status != ApprovalStatus::Pending {
            return Err(ApprovalInteractionError::NotPending);
        }
        self.decisions.deny(scope, request_id, denial).await?;
        Ok(())
    }
}

/// Project an `Action` to its target capability when the action is
/// capability-shaped. Returns `None` for file/network/secret/lifecycle
/// actions — those are not actionable through `ApprovalResolver` and the
/// interaction service deliberately omits them from the pending list.
fn capability_id_for_action(action: &ironclaw_host_api::Action) -> Option<&CapabilityId> {
    use ironclaw_host_api::Action;
    match action {
        Action::Dispatch { capability, .. } | Action::SpawnCapability { capability, .. } => {
            Some(capability)
        }
        _ => None,
    }
}
