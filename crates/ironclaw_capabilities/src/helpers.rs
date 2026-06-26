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

/// Finds a Claimed lease that was left in-flight by a prior approval-resume
/// auth bounce.
///
/// Called from `auth_resume_json` when `matching_approval_lease` (Active-only)
/// returns `None` after a `resume_json` → `AuthorizationRequiresAuth` bounce.
/// That bounce claims the lease but skips the revoke (Part A of the fix), so
/// the lease is Claimed rather than Active.  This helper locates it so the
/// same invocation can continue without a second approval prompt.
pub(crate) async fn matching_claimed_approval_lease_for_auth_resume(
    capability_leases: &dyn CapabilityLeaseStore,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
    invocation_fingerprint: &InvocationFingerprint,
) -> Option<CapabilityLease> {
    capability_leases
        .leases_for_scope(scope)
        .await
        .into_iter()
        .find(|lease| {
            lease.scope == *scope
                && lease.grant.capability == *capability_id
                && lease.invocation_fingerprint.as_ref() == Some(invocation_fingerprint)
                && lease.status == CapabilityLeaseStatus::Claimed
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilityRunStateTransition {
    Fail { error_kind: &'static str },
    BlockAuth { error_kind: &'static str },
}

impl CapabilityRunStateTransition {
    pub(crate) fn error_kind(self) -> &'static str {
        match self {
            Self::Fail { error_kind } | Self::BlockAuth { error_kind } => error_kind,
        }
    }
}

impl CapabilityInvocationError {
    /// Returns the run-state transition to apply for this error, or `None`
    /// when no transition is appropriate at the capability-host layer.
    ///
    /// Dispatch failures intentionally return `None`: per the
    /// `capability_failure_disposition` policy introduced in PR #4236, every
    /// `DispatchFailureKind` is either a `ModelVisibleToolError` (the model
    /// observes the failure as an ordinary tool error and can retry with
    /// corrected input) or a `RetrySameCall` (the caller retries
    /// transparently). Neither path wants the run marked failed at this
    /// layer; doing so would short-circuit the disposition policy and turn
    /// recoverable failures (notably `InputEncode`) into terminal run
    /// failures.
    pub(crate) fn run_state_transition(&self) -> Option<CapabilityRunStateTransition> {
        match self {
            Self::UnsupportedObligations { .. } => Some(CapabilityRunStateTransition::Fail {
                error_kind: "UnsupportedObligations",
            }),
            Self::ObligationFailed { .. } => Some(CapabilityRunStateTransition::Fail {
                error_kind: "ObligationFailed",
            }),
            Self::AuthorizationRequiresAuth { .. } => {
                Some(CapabilityRunStateTransition::BlockAuth {
                    error_kind: "AuthRequired",
                })
            }
            Self::Dispatch { .. } => None,
            Self::UnknownCapability { .. }
            | Self::AuthorizationDenied { .. }
            | Self::AuthorizationRequiresApproval { .. }
            | Self::InvocationFingerprint { .. }
            | Self::ApprovalRequestMismatch { .. }
            | Self::ApprovalFingerprintMismatch { .. }
            | Self::ApprovalNotApproved { .. }
            | Self::ApprovalStoreMissing { .. }
            | Self::ApprovalLeaseMissing { .. }
            | Self::ResumeStoreMissing { .. }
            | Self::ProcessManagerMissing { .. }
            | Self::ResumeNotBlocked { .. }
            | Self::ResumeContextMismatch { .. }
            | Self::Lease(_)
            | Self::RunState(_)
            | Self::Process(_) => Some(CapabilityRunStateTransition::Fail {
                error_kind: "Obligation",
            }),
        }
    }
}

pub(crate) async fn apply_run_state_transition_if_configured(
    run_state: Option<&dyn RunStateStore>,
    scope: &ResourceScope,
    invocation_id: InvocationId,
    error: &CapabilityInvocationError,
) {
    let Some(run_state) = run_state else {
        return;
    };
    let Some(transition) = error.run_state_transition() else {
        // No run-state transition at this layer; PR #4236 disposition policy
        // handles the failure on the outcome path.
        return;
    };
    match transition {
        CapabilityRunStateTransition::Fail { error_kind } => {
            fail_run_if_configured(Some(run_state), scope, invocation_id, error_kind).await;
        }
        CapabilityRunStateTransition::BlockAuth { error_kind } => {
            if let Err(error) = run_state
                .block_auth(scope, invocation_id, error_kind.to_string())
                .await
            {
                warn!(
                    invocation_id = %invocation_id,
                    error_kind,
                    transition_error_kind = run_state_error_kind(&error),
                    "run-state auth block transition failed; original business error is being returned to caller",
                );
            }
        }
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
        CapabilityLeaseError::VersionMismatch => "VersionMismatch",
        CapabilityLeaseError::CasExhausted => "CasExhausted",
    }
}

pub(crate) fn claim_error_may_be_concurrent_resume(error: &CapabilityLeaseError) -> bool {
    matches!(
        error,
        CapabilityLeaseError::InactiveLease {
            status: CapabilityLeaseStatus::Claimed
                | CapabilityLeaseStatus::Dispatching
                | CapabilityLeaseStatus::Consumed,
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
        RunStateError::Backend(_) => "Backend",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapabilityInvocationError;
    use ironclaw_host_api::{CapabilityId, DispatchFailureKind, RuntimeDispatchErrorKind};

    fn capability() -> CapabilityId {
        CapabilityId::new("test.capability").expect("capability id")
    }

    /// Regression for PR #4236: dispatch failures must not transition the run
    /// state at this layer. The `capability_failure_disposition` policy in
    /// host_runtime maps every `DispatchFailureKind` to either
    /// `ModelVisibleToolError` or `RetrySameCall`; both want the run to keep
    /// going. Calling `run_state.fail()` here would short-circuit that policy
    /// and turn recoverable input errors (notably `InputEncode`) into
    /// terminal run failures invisible to the model.
    #[test]
    fn dispatch_input_encode_returns_no_run_state_transition() {
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            safe_summary: None,
            detail: None,
        };
        assert!(error.run_state_transition().is_none());
    }

    #[test]
    fn dispatch_backend_returns_no_run_state_transition() {
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend),
            safe_summary: None,
            detail: None,
        };
        assert!(error.run_state_transition().is_none());
    }

    #[test]
    fn dispatch_unknown_capability_returns_no_run_state_transition() {
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::UnknownCapability,
            safe_summary: None,
            detail: None,
        };
        assert!(error.run_state_transition().is_none());
    }

    #[test]
    fn unknown_capability_still_fails_run_state() {
        let error = CapabilityInvocationError::UnknownCapability {
            capability: capability(),
        };
        let transition = error
            .run_state_transition()
            .expect("non-dispatch errors keep their fail transition");
        assert!(matches!(
            transition,
            CapabilityRunStateTransition::Fail { .. }
        ));
    }

    #[test]
    fn authorization_requires_auth_still_blocks_auth() {
        let error = CapabilityInvocationError::AuthorizationRequiresAuth {
            capability: capability(),
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        };
        let transition = error
            .run_state_transition()
            .expect("auth-required errors keep their block-auth transition");
        assert!(matches!(
            transition,
            CapabilityRunStateTransition::BlockAuth { .. }
        ));
    }
}
