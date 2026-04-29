use ironclaw_authorization::{CapabilityLease, CapabilityLeaseError, CapabilityLeaseStore};
use ironclaw_host_api::{
    CapabilityId, ExecutionContext, InvocationFingerprint, InvocationId, Obligation, ResourceScope,
};
use ironclaw_run_state::{ApprovalStatus, RunStateError, RunStateStore};
use tracing::warn;

use crate::{CapabilityInvocationError, ResumeContextMismatchKind};

pub(crate) fn ensure_no_obligations(
    capability: &CapabilityId,
    obligations: Vec<Obligation>,
) -> Result<(), CapabilityInvocationError> {
    if obligations.is_empty() {
        Ok(())
    } else {
        Err(CapabilityInvocationError::UnsupportedObligations {
            capability: capability.clone(),
            obligations,
        })
    }
}

pub(crate) async fn matching_approval_lease(
    capability_leases: &dyn CapabilityLeaseStore,
    context: &ExecutionContext,
    capability_id: &CapabilityId,
    invocation_fingerprint: &InvocationFingerprint,
) -> Option<CapabilityLease> {
    capability_leases
        .active_leases_for_context(context)
        .await
        .into_iter()
        .find(|lease| {
            lease.scope == context.resource_scope
                && lease.grant.capability == *capability_id
                && lease.invocation_fingerprint.as_ref() == Some(invocation_fingerprint)
        })
}

pub(crate) async fn fail_run_if_configured(
    run_state: Option<&dyn RunStateStore>,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    error_kind: &'static str,
) {
    if let Some(run_state) = run_state
        && let Err(error) = fail_run(run_state, scope, invocation_id, error_kind).await
    {
        warn!(
            invocation_id = %invocation_id,
            error_kind,
            transition_error_kind = run_state_error_kind(&error),
            "run-state fail transition failed; original business error is being returned to caller",
        );
    }
}

pub(crate) async fn fail_run(
    run_state: &dyn RunStateStore,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    error_kind: &'static str,
) -> Result<(), RunStateError> {
    run_state
        .fail(scope, invocation_id, error_kind.to_string())
        .await?;
    Ok(())
}

pub(crate) fn approval_not_approved_error_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "ApprovalPending",
        ApprovalStatus::Approved => "ApprovalApproved",
        ApprovalStatus::Denied => "ApprovalDenied",
        ApprovalStatus::Expired => "ApprovalExpired",
    }
}

pub(crate) fn resume_context_mismatch_kind(
    capability_mismatch: bool,
    approval_request_mismatch: bool,
) -> ResumeContextMismatchKind {
    debug_assert!(capability_mismatch || approval_request_mismatch);
    match (capability_mismatch, approval_request_mismatch) {
        (true, true) => ResumeContextMismatchKind::CapabilityAndApprovalRequestId,
        (true, false) => ResumeContextMismatchKind::CapabilityId,
        (false, true) => ResumeContextMismatchKind::ApprovalRequestId,
        (false, false) => ResumeContextMismatchKind::ApprovalRequestId,
    }
}

pub(crate) fn capability_lease_error_kind(error: &CapabilityLeaseError) -> &'static str {
    match error {
        CapabilityLeaseError::UnknownLease { .. } => "UnknownLease",
        CapabilityLeaseError::ExpiredLease { .. } => "ExpiredLease",
        CapabilityLeaseError::ExhaustedLease { .. } => "ExhaustedLease",
        CapabilityLeaseError::UnclaimedFingerprintLease { .. } => "UnclaimedFingerprintLease",
        CapabilityLeaseError::FingerprintMismatch { .. } => "FingerprintMismatch",
        CapabilityLeaseError::InactiveLease { .. } => "InactiveLease",
        CapabilityLeaseError::Persistence { .. } => "Persistence",
    }
}

pub(crate) fn run_state_error_kind(error: &RunStateError) -> &'static str {
    match error {
        RunStateError::UnknownInvocation { .. } => "UnknownInvocation",
        RunStateError::InvocationAlreadyExists { .. } => "InvocationAlreadyExists",
        RunStateError::UnknownApprovalRequest { .. } => "UnknownApprovalRequest",
        RunStateError::ApprovalRequestAlreadyExists { .. } => "ApprovalRequestAlreadyExists",
        RunStateError::ApprovalNotPending { .. } => "ApprovalNotPending",
        RunStateError::InvalidPath(_) => "InvalidPath",
        RunStateError::Filesystem(_) => "Filesystem",
        RunStateError::Serialization(_) => "Serialization",
        RunStateError::Deserialization(_) => "Deserialization",
    }
}
