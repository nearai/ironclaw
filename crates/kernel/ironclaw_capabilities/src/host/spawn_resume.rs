//! Workflow 5 — `resume_spawn_json`: resuming a background spawn blocked on approval.
//!
//! The spawn twin of [`super::approval_resume`]: same preflight and approval
//! validation, but the tail starts a process instead of dispatching inline, so
//! it seals a [`ProcessAuthorizedContinuation`] rather than an inline witness.

use ironclaw_approvals::{ApprovalStatus, ApprovalStoreError};
use ironclaw_host_api::{
    decision::{Decision, DenyReason},
    dispatch::CapabilityDispatcher,
    ids::{ApprovalRequestId, CapabilityId, ProcessId},
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use ironclaw_processes::{ProcessInvocationError, ProcessInvocationStatus, ProcessStart};
use tracing::warn;

use super::{
    ApprovalResumeInput, BlockedResumeKind, CapabilityHost, process_authorized_continuation,
};
use crate::helpers::{
    CapabilityActionKind, apply_invocation_state_transition_if_configured,
    approval_not_approved_error_kind, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, complete_invocation_after_side_effect,
    fail_invocation_if_configured, invocation_fingerprint_for_kind, matching_approval_lease,
    resume_context_mismatch_kind, validate_approval_request_matches_invocation,
};
use crate::{CapabilityInvocationError, CapabilityObligationPhase, CapabilitySpawnResult};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub async fn resume_spawn_json(
        &self,
        context: ExecutionContext,
        approval_request_id: ApprovalRequestId,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: serde_json::Value,
    ) -> Result<CapabilitySpawnResult, CapabilityInvocationError> {
        let request = ApprovalResumeInput {
            context,
            approval_request_id,
            capability_id,
            estimate,
            input,
        };
        let process_manager = self.process_manager.ok_or_else(|| {
            CapabilityInvocationError::ProcessManagerMissing {
                capability: request.capability_id.clone(),
            }
        })?;
        let invocation_state =
            self.invocation_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "invocation_state",
                })?;
        let approval_requests = self.approval_requests.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "approval_requests",
            }
        })?;
        let capability_leases = self.capability_leases.ok_or_else(|| {
            CapabilityInvocationError::ResumeStoreMissing {
                capability: request.capability_id.clone(),
                store: "capability_leases",
            }
        })?;

        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }

        // Resume-path pre-authorization (§5.3.2/§9, R-A): descriptor + runtime-policy
        // planning BEFORE the process-invocation lookup (see `resume_json`), so an unknown
        // capability short-circuits to `MissingRuntime` and a tightened policy fails
        // closed. On refusal only the matching `BlockedApproval` run is failed.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Approval {
                approval_request_id: request.approval_request_id,
            },
        )
        .await?;

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Spawn,
            &scope,
            &request.capability_id,
            &request.estimate,
            &request.input,
        )
        .map_err(|source| CapabilityInvocationError::InvocationFingerprint {
            capability: request.capability_id.clone(),
            source,
        })?;

        let run_record = invocation_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        if run_record.authenticated_actor_user_id != request.context.authenticated_actor_user_id {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::PolicyDenied,
                detail: None,
            });
        }
        if run_record.status != ProcessInvocationStatus::BlockedApproval {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        let capability_mismatch = run_record.capability_id != request.capability_id;
        let approval_request_mismatch =
            run_record.approval_request_id != Some(request.approval_request_id);
        if capability_mismatch || approval_request_mismatch {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: request.capability_id,
                kind: resume_context_mismatch_kind(capability_mismatch, approval_request_mismatch),
            });
        }

        let approval = approval_requests
            .get(&scope, request.approval_request_id)
            .await?
            .ok_or(ApprovalStoreError::UnknownApprovalRequest {
                request_id: request.approval_request_id,
            })?;
        if approval.status != ApprovalStatus::Approved {
            if approval.status != ApprovalStatus::Pending {
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    approval_not_approved_error_kind(approval.status),
                )
                .await;
            }
            return Err(CapabilityInvocationError::ApprovalNotApproved {
                capability: request.capability_id,
                status: approval.status,
            });
        }
        if let Err(error) = validate_approval_request_matches_invocation(
            &approval.request,
            &request.context,
            &request.capability_id,
            &request.estimate,
            CapabilityActionKind::Spawn,
        ) {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "ApprovalRequestMismatch",
            )
            .await;
            return Err(error);
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "InvocationFingerprintMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                capability: request.capability_id,
            });
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "UnknownCapability",
            )
            .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let Some(lease) = matching_approval_lease(
            capability_leases,
            &request.context,
            &request.capability_id,
            &invocation_fingerprint,
        )
        .await
        else {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "ApprovalLeaseMissing",
            )
            .await;
            return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                capability: request.capability_id,
            });
        };
        let mut authorized_context = request.context.clone();
        authorized_context.grants.grants.push(lease.grant.clone());

        // Kernel-computed trust on the spawn-resume path (§5.3.2/§9). Runtime-policy
        // planning already ran in `resume_preflight` above (fail-closed before the
        // lease was claimed), so it is not repeated here.
        let trust_decision = match self.evaluate_trust(&capability_id) {
            Ok(d) => d,
            Err(error) => {
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(error);
            }
        };
        authorized_context.trust = trust_decision.effective_trust.class();

        let obligations = match self
            .authorizer
            .authorize_spawn_with_trust(
                &authorized_context,
                descriptor,
                &request.estimate,
                &trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => allowed_obligations.into_vec(),
            Decision::Deny { reason } => {
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                    detail: None,
                });
            }
            Decision::RequireApproval { .. } => {
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    "AuthorizationRequiresApproval",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        };

        let claimed_lease = match capability_leases
            .claim(&scope, lease.grant.id, &invocation_fingerprint)
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                if claim_error_may_be_concurrent_resume(&error) {
                    warn!(
                        lease_id = %lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        error_kind = capability_lease_error_kind(&error),
                        "spawn approval lease claim lost to a concurrent resume; leaving invocation state unchanged",
                    );
                } else {
                    fail_invocation_if_configured(
                        Some(invocation_state),
                        &scope,
                        invocation_id,
                        "ApprovalLeaseClaim",
                    )
                    .await;
                }
                return Err(CapabilityInvocationError::Lease(Box::new(error)));
            }
        };

        let obligation_outcome = match self
            .prepare_obligations(
                CapabilityObligationPhase::Spawn,
                &authorized_context,
                &request.capability_id,
                &request.estimate,
                obligations.clone(),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                apply_invocation_state_transition_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        obligation_error = %error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after spawn obligation failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };
        let effective_mounts = obligation_outcome
            .mounts
            .clone()
            .unwrap_or_else(|| authorized_context.mounts.clone());
        let resource_reservation_id = obligation_outcome
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);
        let process_id = ProcessId::new();
        let result = self.seal_authorization(
            &authorized_context,
            &request.capability_id,
            &request.estimate,
            &request.input,
            descriptor,
            &obligation_outcome,
            claimed_lease.grant.constraints.expires_at,
        );
        let authorized_continuation = match process_authorized_continuation(
            result,
            &request.capability_id,
            descriptor.runtime,
            process_id,
        ) {
            Ok(continuation) => continuation,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    "ProcessSpawn",
                )
                .await;
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after spawn authorization failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };

        let process = match process_manager
            .spawn(ProcessStart {
                process_id,
                parent_process_id: authorized_context.process_id,
                invocation_id,
                scope: scope.clone(),
                authenticated_actor_user_id: authorized_context.authenticated_actor_user_id.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id.clone(),
                runtime: descriptor.runtime,
                grants: authorized_context.grants.clone(),
                mounts: effective_mounts,
                estimated_resources: request.estimate.clone(),
                resource_reservation_id,
                authorized_continuation,
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_invocation_if_configured(
                    Some(invocation_state),
                    &scope,
                    invocation_id,
                    "ProcessSpawn",
                )
                .await;
                let invocation_error = CapabilityInvocationError::from(error);
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        process_error = %invocation_error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after process spawn failure; lease may remain claimed",
                    );
                }
                return Err(invocation_error);
            }
        };

        if let Err(error) = capability_leases
            .consume(&scope, claimed_lease.grant.id)
            .await
        {
            warn!(
                lease_id = %claimed_lease.grant.id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                error_kind = capability_lease_error_kind(&error),
                "capability lease consume failed after successful process spawn; lease left in claimed state",
            );
        }

        complete_invocation_after_side_effect(
            invocation_state,
            &scope,
            invocation_id,
            &capability_id,
            "spawn",
        )
        .await;
        Ok(CapabilitySpawnResult { process })
    }
}
