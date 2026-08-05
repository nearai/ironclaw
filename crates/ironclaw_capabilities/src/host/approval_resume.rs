//! Workflow 2 — `resume_json`: resuming an invocation blocked on an approval gate.
//!
//! Owns the approval-resume preamble (approval record validation, one-shot
//! lease selection) and then converges on the shared tail in
//! [`super::resume_support`].

use ironclaw_approvals::{ApprovalStatus, ApprovalStoreError};
use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::CapabilityDispatcher,
    ids::{ApprovalRequestId, CapabilityId},
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use ironclaw_processes::{ProcessInvocationError, ProcessInvocationStatus};

use super::{
    ApprovalResumeInput, BlockedResumeKind, CapabilityHost, PendingClaimAfterAuth,
    ResumedDispatchParams, ResumedLeaseState,
};
use crate::helpers::{
    CapabilityActionKind, approval_not_approved_error_kind, fail_invocation_if_configured,
    invocation_fingerprint_for_kind, matching_approval_lease, resume_context_mismatch_kind,
    validate_approval_request_matches_invocation,
};
use crate::{CapabilityInvocationError, CapabilityInvocationResult};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub async fn resume_json(
        &self,
        context: ExecutionContext,
        approval_request_id: ApprovalRequestId,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: serde_json::Value,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let request = ApprovalResumeInput {
            context,
            approval_request_id,
            capability_id,
            estimate,
            input,
        };
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

        // Resume-path pre-authorization (§5.3.2/§9, R-A): resolve the descriptor
        // and enforce runtime-policy planning BEFORE the process-invocation lookup so an
        // unknown capability short-circuits to `UnknownCapability`
        // (→ `MissingRuntime`) instead of the process-invocation-not-found `Backend` path,
        // and a policy tightened between invoke and resume fails closed. On
        // refusal only the matching `BlockedApproval` run is failed.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Approval {
                approval_request_id: request.approval_request_id,
            },
        )
        .await?;

        let invocation_fingerprint = invocation_fingerprint_for_kind(
            CapabilityActionKind::Dispatch,
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
            CapabilityActionKind::Dispatch,
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
        // The lease is claimed INSIDE `dispatch_resumed_capability`, after
        // `authorize_dispatch_with_trust` returns Allow.  Deferring the claim
        // preserves the original contract: a Deny leaves the lease Active.
        let grant_id = lease.grant.id;
        // Carry the lease expiry onto the pending-claim spec so the sealed
        // witness minted in `authorize_resumed` is bounded by the approval that
        // authorized it (the claim, and thus a readable claimed lease, happens
        // only after the seal).
        let grant_expiry = lease.grant.constraints.expires_at;

        self.dispatch_resumed_capability(ResumedDispatchParams {
            invocation_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            authorized_context,
            descriptor,
            lease_state: ResumedLeaseState::PendingClaim(PendingClaimAfterAuth {
                leases: capability_leases,
                grant_id,
                fingerprint: invocation_fingerprint,
                grant_expiry,
            }),
        })
        .await
    }
}
