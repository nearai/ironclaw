//! Workflows 3 and 4 — the two exits from an auth gate.
//!
//! `auth_resume_json` resumes a `BlockedAuth` run once the credential exists;
//! `decline_auth_json` is the terminal refusal that fails the blocked run and
//! never authorizes or dispatches. Both converge on
//! [`super::resume_support`]; neither owns policy.

use ironclaw_approvals::{ApprovalStatus, ApprovalStoreError};
use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::{CapabilityDispatchResult, CapabilityDispatcher},
    ids::{ApprovalRequestId, CapabilityId},
    resource::ResourceEstimate,
    scope::ExecutionContext,
};
use ironclaw_processes::{ProcessInvocationError, ProcessInvocationStatus};
use tracing::{debug, warn};

use super::{
    AuthResumeInput, BlockedResumeKind, CapabilityHost, ResumedDispatchParams, ResumedLeaseState,
};
use crate::CapabilityInvocationError;
use crate::helpers::{
    CapabilityActionKind, approval_not_approved_error_kind, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, fail_invocation_if_configured,
    invocation_fingerprint_for_kind, matching_approval_lease,
    matching_claimed_approval_lease_for_auth_resume, resume_context_mismatch_kind,
    validate_approval_request_matches_invocation,
};

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    /// Resume an invocation that was previously blocked at an auth gate.
    ///
    /// Validates that the invocation record is in `BlockedAuth` status.  When the
    /// invocation also passed an earlier approval gate (`approval_request_id`
    /// is `Some`), validates and claims the fingerprinted approval lease before
    /// dispatch so the prior approval is honoured without a second approval
    /// prompt.  When `approval_request_id` is `None` no lease step is needed
    /// and the path falls through to normal authorization + dispatch.
    pub async fn auth_resume_json(
        &self,
        context: ExecutionContext,
        capability_id: CapabilityId,
        estimate: ResourceEstimate,
        input: serde_json::Value,
        approval_request_id: Option<ApprovalRequestId>,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        let request = AuthResumeInput {
            context,
            capability_id,
            estimate,
            input,
            approval_request_id,
        };
        let invocation_state =
            self.invocation_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "invocation_state",
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
        // planning BEFORE the process-invocation lookup (see `resume_json`). On refusal only
        // the matching `BlockedAuth` run is failed — `approval_request_id` is NOT
        // compared, because `block_auth` clears it to `None` on the record.
        self.resume_preflight(
            &request.context,
            &request.capability_id,
            BlockedResumeKind::Auth,
        )
        .await?;

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
        if run_record.status != ProcessInvocationStatus::BlockedAuth {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        // Verify the capability_id on the request matches the one recorded in
        // the invocation state when the run was originally started.  A mismatch means
        // the caller is trying to resume a different capability than the one
        // that was blocked — treat it as a context mismatch and fail the run.
        if run_record.capability_id != request.capability_id {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: request.capability_id,
                kind: resume_context_mismatch_kind(true, false),
            });
        }

        // Check that the capability still exists before acquiring or mutating any
        // approval lease.  Moving this check above the lease-acquisition block
        // ensures an unknown capability returns `UnknownCapability` without
        // touching the lease at all — preventing a one-shot lease from being
        // permanently stranded in `Claimed`/`Dispatching` when the capability
        // was unregistered between the original invocation and this resume.
        let Some(base_descriptor) = self.registry.get_capability(&request.capability_id) else {
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
        let descriptor = match self
            .enrich_invocation_descriptor(base_descriptor, &request.capability_id, &request.input)
            .await
        {
            Ok(descriptor) => descriptor,
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
        if let Err(error) = self.enforce_runtime_policy(&descriptor) {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "RuntimePolicyDenied",
            )
            .await;
            return Err(error);
        }

        // When the invocation previously passed an approval gate, validate and
        // claim the fingerprinted approval lease so the existing approval
        // carries through without requiring a second human approval.
        //
        // `approval_lease_to_consume` tracks the lease that must be consumed
        // after a successful dispatch.  It is `Some` only when a lease was
        // found and used; the `None` branch (no prior approval) skips the
        // consume step entirely.
        let (authorized_context, approval_lease_to_consume) = if let Some(approval_request_id) =
            request.approval_request_id
        {
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

            let approval = approval_requests
                .get(&scope, approval_request_id)
                .await?
                .ok_or(ApprovalStoreError::UnknownApprovalRequest {
                    request_id: approval_request_id,
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

            // Try to find an Active lease (clean first-time path).
            let active_lease = matching_approval_lease(
                capability_leases,
                &request.context,
                &request.capability_id,
                &invocation_fingerprint,
            )
            .await;

            let claimed = if let Some(lease) = active_lease {
                // Fresh Active lease: claim it (Active→Claimed), then immediately
                // advance it to Dispatching via begin_dispatch_claimed.  This
                // ensures the in-flight single-winner fence covers the fresh path
                // just as it covers the reuse (already-Claimed) path below.
                // Without the second step a concurrent auth_resume_json that misses
                // the Active lease would find the Claimed lease in the reuse branch
                // and successfully call begin_dispatch_claimed itself — double-firing.
                let lease_id = lease.grant.id;
                let claimed = match capability_leases
                    .claim(&scope, lease_id, &invocation_fingerprint)
                    .await
                {
                    Ok(claimed) => claimed,
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %lease_id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease claim lost to a concurrent auth-resume; leaving invocation state unchanged",
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
                // Advance Claimed→Dispatching so the fence is set before dispatch.
                match capability_leases
                    .begin_dispatch_claimed(&scope, claimed.grant.id, &invocation_fingerprint)
                    .await
                {
                    Ok(dispatching_lease) => {
                        debug!(
                            lease_id = %dispatching_lease.grant.id,
                            invocation_id = %invocation_id,
                            capability_id = %capability_id,
                            "auth_resume fresh path advanced lease to Dispatching"
                        );
                        dispatching_lease
                    }
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %claimed.grant.id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease reuse lost to a concurrent auth-resume; leaving invocation state unchanged",
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
                }
            } else if let Some(claimed_lease) = matching_claimed_approval_lease_for_auth_resume(
                capability_leases,
                &scope,
                &request.capability_id,
                &invocation_fingerprint,
            )
            .await
            {
                // Claimed lease from a prior resume_json auth bounce: atomically
                // transition it to Dispatching so exactly one concurrent auth-resume
                // wins the reuse race. The loser sees InactiveLease{Dispatching} and
                // bails — matching the Active-lease claim() loser path.
                match capability_leases
                    .begin_dispatch_claimed(&scope, claimed_lease.grant.id, &invocation_fingerprint)
                    .await
                {
                    Ok(dispatching_lease) => {
                        debug!(
                            lease_id = %dispatching_lease.grant.id,
                            invocation_id = %invocation_id,
                            capability_id = %capability_id,
                            approval_request_id = %approval_request_id,
                            "auth_resume won dispatch race for claimed approval lease"
                        );
                        dispatching_lease
                    }
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %claimed_lease.grant.id,
                                invocation_id = %invocation_id,
                                capability_id = %capability_id,
                                error_kind = capability_lease_error_kind(&error),
                                "approval lease reuse lost to a concurrent auth-resume; leaving invocation state unchanged",
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
                }
            } else {
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

            let mut ctx = request.context.clone();
            ctx.grants.grants.push(claimed.grant.clone());
            (ctx, Some((capability_leases, claimed)))
        } else {
            (request.context.clone(), None)
        };

        self.dispatch_resumed_capability(ResumedDispatchParams {
            invocation_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            authorized_context,
            descriptor: &descriptor,
            lease_state: match approval_lease_to_consume {
                Some((leases, lease)) => ResumedLeaseState::AlreadyClaimed(leases, Box::new(lease)),
                None => ResumedLeaseState::NoPriorLease,
            },
        })
        .await
    }

    /// Terminalize an invocation whose auth gate was explicitly denied.
    ///
    /// This is the denial half of [`Self::auth_resume_json`]: it validates the
    /// same sealed invocation identity and actor scope, transitions only the
    /// matching `BlockedAuth` record to `Failed`, and never authorizes or
    /// dispatches the capability.
    pub async fn decline_auth_json(
        &self,
        context: ExecutionContext,
        capability_id: CapabilityId,
    ) -> Result<(), CapabilityInvocationError> {
        let invocation_state =
            self.invocation_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: capability_id.clone(),
                    store: "invocation_state",
                })?;
        let invocation_id = context.invocation_id;
        let scope = context.resource_scope.clone();
        if context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: capability_id,
                reason: DenyReason::InternalInvariantViolation,
                detail: None,
            });
        }
        let run_record = invocation_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        if run_record.authenticated_actor_user_id != context.authenticated_actor_user_id {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: capability_id,
                reason: DenyReason::PolicyDenied,
                detail: None,
            });
        }
        if run_record.status != ProcessInvocationStatus::BlockedAuth {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: capability_id,
                status: run_record.status,
            });
        }
        if run_record.capability_id != capability_id {
            fail_invocation_if_configured(
                Some(invocation_state),
                &scope,
                invocation_id,
                "ResumeContextMismatch",
            )
            .await;
            return Err(CapabilityInvocationError::ResumeContextMismatch {
                capability: capability_id,
                kind: resume_context_mismatch_kind(true, false),
            });
        }
        invocation_state
            .fail(&scope, invocation_id, "GateDeclined".to_string())
            .await?;
        Ok(())
    }
}
