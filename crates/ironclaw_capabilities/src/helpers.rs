use ironclaw_authorization::{
    CapabilityLease, CapabilityLeaseError, CapabilityLeaseStatus, CapabilityLeaseStore,
};
use ironclaw_host_api::{
    Action, ApprovalRequest, CapabilityId, ExecutionContext, InvocationFingerprint, InvocationId,
    Principal, ResourceEstimate, ResourceScope,
};
use ironclaw_run_state::{ApprovalStatus, RunStateError, RunStateStore};
use tracing::warn;

use crate::{CapabilityInvocationError, ResumeContextMismatchKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityActionKind {
    Dispatch,
    Spawn,
}

pub(crate) fn invocation_fingerprint_for_kind(
    kind: CapabilityActionKind,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    estimate: &ResourceEstimate,
    input: &serde_json::Value,
) -> Result<InvocationFingerprint, ironclaw_host_api::HostApiError> {
    match kind {
        CapabilityActionKind::Dispatch => {
            InvocationFingerprint::for_dispatch(scope, capability_id, estimate, input)
        }
        CapabilityActionKind::Spawn => {
            InvocationFingerprint::for_spawn(scope, capability_id, estimate, input)
        }
    }
}

pub(crate) fn validate_approval_request_matches_invocation(
    approval: &ApprovalRequest,
    context: &ExecutionContext,
    capability_id: &CapabilityId,
    estimate: &ResourceEstimate,
    expected_action: CapabilityActionKind,
) -> Result<(), CapabilityInvocationError> {
    let action_matches = match (expected_action, approval.action.as_ref()) {
        (
            CapabilityActionKind::Dispatch,
            Action::Dispatch {
                capability,
                estimated_resources,
            },
        )
        | (
            CapabilityActionKind::Spawn,
            Action::SpawnCapability {
                capability,
                estimated_resources,
            },
        ) => capability == capability_id && estimated_resources == estimate,
        _ => false,
    };
    if !action_matches {
        return Err(CapabilityInvocationError::ApprovalRequestMismatch {
            capability: capability_id.clone(),
            field: "action",
        });
    }

    if approval.correlation_id != context.correlation_id {
        return Err(CapabilityInvocationError::ApprovalRequestMismatch {
            capability: capability_id.clone(),
            field: "correlation_id",
        });
    }

    let expected_requester = Principal::Extension(context.extension_id.clone());
    if approval.requested_by != expected_requester {
        return Err(CapabilityInvocationError::ApprovalRequestMismatch {
            capability: capability_id.clone(),
            field: "requested_by",
        });
    }

    Ok(())
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

pub(crate) async fn complete_run_after_side_effect(
    run_state: &dyn RunStateStore,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    capability_id: &CapabilityId,
    side_effect: &'static str,
) {
    if let Err(error) = run_state.complete(scope, invocation_id).await {
        warn!(
            invocation_id = %invocation_id,
            capability_id = %capability_id,
            side_effect,
            transition_error_kind = run_state_error_kind(&error),
            "run-state completion failed after successful side effect; returning successful capability result",
        );
    }
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
        (false, false) => unreachable!("resume context mismatch kind called without mismatch"),
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

pub(crate) fn claim_error_may_be_concurrent_resume(error: &CapabilityLeaseError) -> bool {
    matches!(
        error,
        CapabilityLeaseError::InactiveLease {
            status: CapabilityLeaseStatus::Claimed | CapabilityLeaseStatus::Consumed,
            ..
        }
    )
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
