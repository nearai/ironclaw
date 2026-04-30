use ironclaw_authorization::{CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher, Decision,
    DenyReason, ExecutionContext, Obligation, ProcessId, ResourceEstimate,
};
use ironclaw_processes::{ProcessManager, ProcessStart};
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStart, RunStateError, RunStateStore, RunStatus,
};
use tracing::warn;

use crate::helpers::{
    CapabilityActionKind, approval_not_approved_error_kind, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, complete_run_after_side_effect, fail_run_if_configured,
    invocation_fingerprint_for_kind, matching_approval_lease, resume_context_mismatch_kind,
    run_state_error_kind, validate_approval_request_matches_invocation,
};
use crate::obligations::post_dispatch_obligations;
use crate::{
    CapabilityInvocationError, CapabilityInvocationRequest, CapabilityInvocationResult,
    CapabilityObligationAbortRequest, CapabilityObligationCompletionRequest,
    CapabilityObligationError, CapabilityObligationHandler, CapabilityObligationOutcome,
    CapabilityObligationPhase, CapabilityObligationRequest, CapabilityResumeRequest,
    CapabilitySpawnRequest, CapabilitySpawnResult,
};

pub struct CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    run_state: Option<&'a dyn RunStateStore>,
    approval_requests: Option<&'a dyn ApprovalRequestStore>,
    capability_leases: Option<&'a dyn CapabilityLeaseStore>,
    process_manager: Option<&'a dyn ProcessManager>,
    obligation_handler: Option<&'a dyn CapabilityObligationHandler>,
}

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        dispatcher: &'a D,
        authorizer: &'a dyn TrustAwareCapabilityDispatchAuthorizer,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            process_manager: None,
            obligation_handler: None,
        }
    }

    /// Attaches the run-state store used to record invocation lifecycle.
    ///
    /// Required for `resume_json`. Strongly recommended for `invoke_json` and
    /// `spawn_json` so denials, obligation rejections, and dispatch failures
    /// transition the run record to `Failed` instead of being silently
    /// dropped. Without it, error paths still return the right user-facing
    /// error but no run record is persisted.
    pub fn with_run_state(mut self, run_state: &'a dyn RunStateStore) -> Self {
        self.run_state = Some(run_state);
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    ///
    /// Required for `invoke_json` paths whose authorizer returns
    /// `Decision::RequireApproval` and for `resume_json`. Without it, an
    /// approval-required dispatch fails with `ApprovalStoreMissing` rather
    /// than blocking for human review.
    pub fn with_approval_requests(
        mut self,
        approval_requests: &'a dyn ApprovalRequestStore,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self
    }

    /// Attaches the capability-lease store used to consume approved leases.
    ///
    /// Required for `resume_json`; not consulted by `invoke_json` or
    /// `spawn_json`.
    pub fn with_capability_leases(
        mut self,
        capability_leases: &'a dyn CapabilityLeaseStore,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    /// Attaches the process manager used to spawn long-running invocations.
    ///
    /// Required for `spawn_json`; not consulted by `invoke_json` or
    /// `resume_json`. Without it, `spawn_json` fails with
    /// `ProcessManagerMissing`.
    pub fn with_process_manager(mut self, process_manager: &'a dyn ProcessManager) -> Self {
        self.process_manager = Some(process_manager);
        self
    }

    /// Attaches the obligation handler that satisfies allow-decision
    /// obligations before/after side effects. Without a handler, non-empty
    /// obligations fail closed.
    pub fn with_obligation_handler(mut self, handler: &'a dyn CapabilityObligationHandler) -> Self {
        self.obligation_handler = Some(handler);
        self
    }

    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
            });
        }

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

        if let Some(run_state) = self.run_state {
            run_state
                .start(RunStart {
                    invocation_id,
                    capability_id: capability_id.clone(),
                    scope: scope.clone(),
                })
                .await?;
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(self.run_state, &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let obligations;
        let obligation_outcome;
        match self
            .authorizer
            .authorize_dispatch_with_trust(
                &request.context,
                descriptor,
                &request.estimate,
                &request.trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => {
                let allowed_obligations = allowed_obligations.into_vec();
                match self
                    .prepare_obligations(
                        CapabilityObligationPhase::Invoke,
                        &request.context,
                        &request.capability_id,
                        &request.estimate,
                        allowed_obligations.clone(),
                    )
                    .await
                {
                    Ok(outcome) => {
                        obligations = allowed_obligations;
                        obligation_outcome = outcome;
                    }
                    Err(error) => {
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            obligation_invocation_error_kind(&error),
                        )
                        .await;
                        return Err(error);
                    }
                }
            }
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval {
                request: mut approval,
            } => {
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Dispatch,
                ) {
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "ApprovalRequestMismatch",
                    )
                    .await;
                    return Err(error);
                }

                if let Some(existing) = &approval.invocation_fingerprint {
                    if existing != &invocation_fingerprint {
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            "InvocationFingerprintMismatch",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                            capability: request.capability_id,
                        });
                    }
                } else {
                    approval.invocation_fingerprint = Some(invocation_fingerprint);
                }

                match (self.run_state, self.approval_requests) {
                    (Some(run_state), Some(approval_requests)) => {
                        let approval_id = approval.id;
                        if let Err(error) = approval_requests
                            .save_pending(scope.clone(), approval.clone())
                            .await
                        {
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalStore",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                        if let Err(error) = run_state
                            .block_approval(&scope, invocation_id, approval)
                            .await
                        {
                            if let Err(discard_error) =
                                approval_requests.discard_pending(&scope, approval_id).await
                            {
                                warn!(
                                    approval_request_id = %approval_id,
                                    invocation_id = %invocation_id,
                                    transition_error_kind = run_state_error_kind(&discard_error),
                                    "approval rollback failed after run-state block transition failed",
                                );
                            }
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalBlock",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                    }
                    (Some(run_state), None) => {
                        fail_run_if_configured(
                            Some(run_state),
                            &scope,
                            invocation_id,
                            "ApprovalStoreMissing",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "approval_requests",
                        });
                    }
                    (None, Some(_)) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "run_state",
                        });
                    }
                    (None, None) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "run_state and approval_requests",
                        });
                    }
                }
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id.clone(),
                scope: scope.clone(),
                estimate: request.estimate.clone(),
                mounts: obligation_outcome.mounts.clone(),
                resource_reservation: obligation_outcome.resource_reservation.clone(),
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(self.run_state, &scope, invocation_id, "Dispatch").await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Invoke,
                &request.context,
                &request.capability_id,
                &request.estimate,
                obligations.as_slice(),
                &dispatch,
            )
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                let cleanup_outcome = CapabilityObligationOutcome::default();
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &cleanup_outcome,
                )
                .await;
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
                )
                .await;
                return Err(error);
            }
        };

        if let Some(run_state) = self.run_state {
            complete_run_after_side_effect(
                run_state,
                &scope,
                invocation_id,
                &capability_id,
                "dispatch",
            )
            .await;
        }

        Ok(CapabilityInvocationResult { dispatch })
    }

    pub async fn resume_json(
        &self,
        request: CapabilityResumeRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
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
            });
        }

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

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.status != RunStatus::BlockedApproval {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        let capability_mismatch = run_record.capability_id != request.capability_id;
        let approval_request_mismatch =
            run_record.approval_request_id != Some(request.approval_request_id);
        if capability_mismatch || approval_request_mismatch {
            fail_run_if_configured(
                Some(run_state),
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
            .ok_or(RunStateError::UnknownApprovalRequest {
                request_id: request.approval_request_id,
            })?;
        if approval.status != ApprovalStatus::Approved {
            if approval.status != ApprovalStatus::Pending {
                fail_run_if_configured(
                    Some(run_state),
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
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalRequestMismatch",
            )
            .await;
            return Err(error);
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_run_if_configured(
                Some(run_state),
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
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
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
            fail_run_if_configured(
                Some(run_state),
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

        let obligations = match self
            .authorizer
            .authorize_dispatch_with_trust(
                &authorized_context,
                descriptor,
                &request.estimate,
                &request.trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => allowed_obligations.into_vec(),
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { .. } => {
                fail_run_if_configured(
                    Some(run_state),
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
                        "approval lease claim lost to a concurrent resume; leaving run state unchanged",
                    );
                } else {
                    fail_run_if_configured(
                        Some(run_state),
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
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &request.capability_id,
                &request.estimate,
                obligations.clone(),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
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
                        "capability lease revoke failed after obligation failure; lease may remain claimed",
                    );
                }
                return Err(error);
            }
        };

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id.clone(),
                scope: scope.clone(),
                estimate: request.estimate.clone(),
                mounts: obligation_outcome.mounts.clone(),
                resource_reservation: obligation_outcome.resource_reservation.clone(),
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(Some(run_state), &scope, invocation_id, "Dispatch").await;
                let invocation_error = CapabilityInvocationError::from(error);
                if let Err(revoke_error) = capability_leases
                    .revoke(&scope, claimed_lease.grant.id)
                    .await
                {
                    warn!(
                        lease_id = %claimed_lease.grant.id,
                        invocation_id = %invocation_id,
                        capability_id = %capability_id,
                        dispatch_error = %invocation_error,
                        revoke_error_kind = capability_lease_error_kind(&revoke_error),
                        "capability lease revoke failed after dispatch failure; lease may remain claimed",
                    );
                }
                return Err(invocation_error);
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &request.capability_id,
                &request.estimate,
                obligations.as_slice(),
                &dispatch,
            )
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                let cleanup_outcome = CapabilityObligationOutcome::default();
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &cleanup_outcome,
                )
                .await;
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
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
                        "capability lease revoke failed after completion obligation failure; lease may remain claimed",
                    );
                }
                return Err(error);
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
                "capability lease consume failed after successful dispatch; lease left in claimed state",
            );
        }

        complete_run_after_side_effect(
            run_state,
            &scope,
            invocation_id,
            &capability_id,
            "dispatch",
        )
        .await;
        Ok(CapabilityInvocationResult { dispatch })
    }

    pub async fn resume_spawn_json(
        &self,
        request: CapabilityResumeRequest,
    ) -> Result<CapabilitySpawnResult, CapabilityInvocationError> {
        let process_manager = self.process_manager.ok_or_else(|| {
            CapabilityInvocationError::ProcessManagerMissing {
                capability: request.capability_id.clone(),
            }
        })?;
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
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
            });
        }

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

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.status != RunStatus::BlockedApproval {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        let capability_mismatch = run_record.capability_id != request.capability_id;
        let approval_request_mismatch =
            run_record.approval_request_id != Some(request.approval_request_id);
        if capability_mismatch || approval_request_mismatch {
            fail_run_if_configured(
                Some(run_state),
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
            .ok_or(RunStateError::UnknownApprovalRequest {
                request_id: request.approval_request_id,
            })?;
        if approval.status != ApprovalStatus::Approved {
            if approval.status != ApprovalStatus::Pending {
                fail_run_if_configured(
                    Some(run_state),
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
            fail_run_if_configured(
                Some(run_state),
                &scope,
                invocation_id,
                "ApprovalRequestMismatch",
            )
            .await;
            return Err(error);
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_run_if_configured(
                Some(run_state),
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
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
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
            fail_run_if_configured(
                Some(run_state),
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

        let obligations = match self
            .authorizer
            .authorize_spawn_with_trust(
                &authorized_context,
                descriptor,
                &request.estimate,
                &request.trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => allowed_obligations.into_vec(),
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { .. } => {
                fail_run_if_configured(
                    Some(run_state),
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
                        "spawn approval lease claim lost to a concurrent resume; leaving run state unchanged",
                    );
                } else {
                    fail_run_if_configured(
                        Some(run_state),
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
                fail_run_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    obligation_invocation_error_kind(&error),
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

        let process = match process_manager
            .spawn(ProcessStart {
                process_id: ProcessId::new(),
                parent_process_id: authorized_context.process_id,
                invocation_id,
                scope: scope.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id.clone(),
                runtime: descriptor.runtime,
                grants: authorized_context.grants.clone(),
                mounts: effective_mounts,
                estimated_resources: request.estimate.clone(),
                resource_reservation_id,
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
                fail_run_if_configured(Some(run_state), &scope, invocation_id, "ProcessSpawn")
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

        complete_run_after_side_effect(run_state, &scope, invocation_id, &capability_id, "spawn")
            .await;
        Ok(CapabilitySpawnResult { process })
    }

    pub async fn spawn_json(
        &self,
        request: CapabilitySpawnRequest,
    ) -> Result<CapabilitySpawnResult, CapabilityInvocationError> {
        let process_manager = self.process_manager.ok_or_else(|| {
            CapabilityInvocationError::ProcessManagerMissing {
                capability: request.capability_id.clone(),
            }
        })?;
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
            });
        }

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

        if let Some(run_state) = self.run_state {
            run_state
                .start(RunStart {
                    invocation_id,
                    capability_id: capability_id.clone(),
                    scope: scope.clone(),
                })
                .await?;
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(self.run_state, &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

        let obligations;
        let obligation_outcome;
        match self
            .authorizer
            .authorize_spawn_with_trust(
                &request.context,
                descriptor,
                &request.estimate,
                &request.trust_decision,
            )
            .await
        {
            Decision::Allow {
                obligations: allowed_obligations,
            } => {
                let allowed_obligations = allowed_obligations.into_vec();
                match self
                    .prepare_obligations(
                        CapabilityObligationPhase::Spawn,
                        &request.context,
                        &request.capability_id,
                        &request.estimate,
                        allowed_obligations.clone(),
                    )
                    .await
                {
                    Ok(outcome) => {
                        obligations = allowed_obligations;
                        obligation_outcome = outcome;
                    }
                    Err(error) => {
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            obligation_invocation_error_kind(&error),
                        )
                        .await;
                        return Err(error);
                    }
                }
            }
            Decision::Deny { reason } => {
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationDenied",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval {
                request: mut approval,
            } => {
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Spawn,
                ) {
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "ApprovalRequestMismatch",
                    )
                    .await;
                    return Err(error);
                }

                if let Some(existing) = &approval.invocation_fingerprint {
                    if existing != &invocation_fingerprint {
                        fail_run_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            "InvocationFingerprintMismatch",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                            capability: request.capability_id,
                        });
                    }
                } else {
                    approval.invocation_fingerprint = Some(invocation_fingerprint);
                }

                match (self.run_state, self.approval_requests) {
                    (Some(run_state), Some(approval_requests)) => {
                        let approval_id = approval.id;
                        if let Err(error) = approval_requests
                            .save_pending(scope.clone(), approval.clone())
                            .await
                        {
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalStore",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                        if let Err(error) = run_state
                            .block_approval(&scope, invocation_id, approval)
                            .await
                        {
                            if let Err(discard_error) =
                                approval_requests.discard_pending(&scope, approval_id).await
                            {
                                warn!(
                                    approval_request_id = %approval_id,
                                    invocation_id = %invocation_id,
                                    transition_error_kind = run_state_error_kind(&discard_error),
                                    "approval rollback failed after spawn run-state block transition failed",
                                );
                            }
                            fail_run_if_configured(
                                Some(run_state),
                                &scope,
                                invocation_id,
                                "ApprovalBlock",
                            )
                            .await;
                            return Err(CapabilityInvocationError::from(error));
                        }
                    }
                    (Some(run_state), None) => {
                        fail_run_if_configured(
                            Some(run_state),
                            &scope,
                            invocation_id,
                            "ApprovalStoreMissing",
                        )
                        .await;
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "approval_requests",
                        });
                    }
                    (None, Some(_)) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "run_state",
                        });
                    }
                    (None, None) => {
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "run_state and approval_requests",
                        });
                    }
                }
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let effective_mounts = obligation_outcome
            .mounts
            .clone()
            .unwrap_or_else(|| request.context.mounts.clone());
        let resource_reservation_id = obligation_outcome
            .resource_reservation
            .as_ref()
            .map(|reservation| reservation.id);

        let process = match process_manager
            .spawn(ProcessStart {
                process_id: ProcessId::new(),
                parent_process_id: request.context.process_id,
                invocation_id,
                scope: scope.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id.clone(),
                runtime: descriptor.runtime,
                grants: request.context.grants.clone(),
                mounts: effective_mounts,
                estimated_resources: request.estimate.clone(),
                resource_reservation_id,
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Spawn,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                fail_run_if_configured(self.run_state, &scope, invocation_id, "ProcessSpawn").await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            complete_run_after_side_effect(
                run_state,
                &scope,
                invocation_id,
                &capability_id,
                "spawn",
            )
            .await;
        }

        Ok(CapabilitySpawnResult { process })
    }

    async fn prepare_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: Vec<Obligation>,
    ) -> Result<CapabilityObligationOutcome, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(CapabilityObligationOutcome::default());
        }
        if matches!(phase, CapabilityObligationPhase::Spawn) {
            let unsupported = post_dispatch_obligations(&obligations);
            if !unsupported.is_empty() {
                return Err(CapabilityInvocationError::UnsupportedObligations {
                    capability: capability_id.clone(),
                    obligations: unsupported,
                });
            }
        }
        let Some(handler) = self.obligation_handler else {
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            });
        };
        handler
            .prepare(CapabilityObligationRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations: obligations.as_slice(),
            })
            .await
            .map_err(|error| obligation_error_to_invocation(capability_id, error))
    }

    async fn complete_dispatch_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        dispatch: &CapabilityDispatchResult,
    ) -> Result<CapabilityDispatchResult, CapabilityInvocationError> {
        if obligations.is_empty() {
            return Ok(dispatch.clone());
        }
        let Some(handler) = self.obligation_handler else {
            let unsupported = post_dispatch_obligations(obligations);
            if unsupported.is_empty() {
                return Ok(dispatch.clone());
            }
            return Err(CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations: unsupported,
            });
        };
        handler
            .complete_dispatch(CapabilityObligationCompletionRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                dispatch,
            })
            .await
            .map_err(|error| obligation_error_to_invocation(capability_id, error))
    }

    async fn abort_obligations(
        &self,
        phase: CapabilityObligationPhase,
        context: &ExecutionContext,
        capability_id: &ironclaw_host_api::CapabilityId,
        estimate: &ResourceEstimate,
        obligations: &[Obligation],
        outcome: &CapabilityObligationOutcome,
    ) {
        if obligations.is_empty() {
            return;
        }
        let Some(handler) = self.obligation_handler else {
            return;
        };
        if let Err(error) = handler
            .abort(CapabilityObligationAbortRequest {
                phase,
                context,
                capability_id,
                estimate,
                obligations,
                outcome,
            })
            .await
        {
            warn!(
                capability_id = %capability_id,
                error = %error,
                "obligation abort failed after downstream side-effect failure",
            );
        }
    }
}

fn obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::Unsupported { obligations } => {
            CapabilityInvocationError::UnsupportedObligations {
                capability: capability_id.clone(),
                obligations,
            }
        }
        CapabilityObligationError::Failed { kind } => CapabilityInvocationError::ObligationFailed {
            capability: capability_id.clone(),
            kind,
        },
    }
}

fn obligation_invocation_error_kind(error: &CapabilityInvocationError) -> &'static str {
    match error {
        CapabilityInvocationError::UnsupportedObligations { .. } => "UnsupportedObligations",
        CapabilityInvocationError::ObligationFailed { .. } => "ObligationFailed",
        _ => "Obligation",
    }
}
