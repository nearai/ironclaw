use ironclaw_authorization::{
    CapabilityLease, CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityDispatchRequest, CapabilityDispatchResult,
    CapabilityDispatcher, CapabilityGrantId, CapabilityId, Decision, DenyReason, ExecutionContext,
    InvocationFingerprint, InvocationId, Obligation, ProcessId, ResourceEstimate, ResourceScope,
};
use ironclaw_processes::{ProcessManager, ProcessStart};
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStart, RunStateApprovalStore, RunStateError,
    RunStateStore, RunStatus,
};
use ironclaw_safety::shell_command_display_text;
use ironclaw_trust::TrustDecision;
use tracing::{debug, warn};

use crate::helpers::{
    CapabilityActionKind, CapabilityRunStateTransition, apply_run_state_transition_if_configured,
    approval_not_approved_error_kind, capability_lease_error_kind,
    claim_error_may_be_concurrent_resume, complete_run_after_side_effect, fail_run_if_configured,
    invocation_fingerprint_for_kind, matching_approval_lease,
    matching_claimed_approval_lease_for_auth_resume, resume_context_mismatch_kind,
    run_state_error_kind, validate_approval_request_matches_invocation,
};
use crate::obligations::post_dispatch_obligations;
use crate::{
    CapabilityAuthResumeRequest, CapabilityInvocationError, CapabilityInvocationRequest,
    CapabilityInvocationResult, CapabilityObligationAbortRequest,
    CapabilityObligationCompletionRequest, CapabilityObligationError,
    CapabilityObligationFailureKind, CapabilityObligationHandler, CapabilityObligationOutcome,
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
    run_state_approval_store: Option<&'a dyn RunStateApprovalStore>,
    capability_leases: Option<&'a dyn CapabilityLeaseStore>,
    process_manager: Option<&'a dyn ProcessManager>,
    obligation_handler: Option<&'a dyn CapabilityObligationHandler>,
}

/// Specification for a lease that must be claimed AFTER authorization succeeds.
///
/// Used by `resume_json` where the approval lease is claimed only after
/// `authorize_dispatch_with_trust` returns `Allow` — keeping the lease `Active`
/// if authorization is denied.
struct PendingClaimAfterAuth<'r> {
    leases: &'r dyn CapabilityLeaseStore,
    grant_id: CapabilityGrantId,
    fingerprint: InvocationFingerprint,
}

/// Encodes the three mutually-exclusive approval-lease states that
/// `dispatch_resumed_capability` must handle.
enum ResumedLeaseState<'r> {
    /// A one-shot `Active` lease to claim *after* `authorize_dispatch_with_trust`
    /// returns `Allow`.  Used by `resume_json` so that a `Deny` leaves the
    /// lease `Active` (the claim is deferred past the authorize call).
    PendingClaim(PendingClaimAfterAuth<'r>),
    /// A lease already transitioned to `Claimed` by a prior `resume_json` auth
    /// bounce.  Used by `auth_resume_json` when the invocation previously passed
    /// an approval gate; reuses the existing `Claimed` lease without a second
    /// approval prompt.
    AlreadyClaimed(&'r dyn CapabilityLeaseStore, Box<CapabilityLease>),
    /// No prior approval lease is in play.  Used by `auth_resume_json` when
    /// `approval_request_id` is `None` (the invocation never passed an approval
    /// gate before hitting the auth gate).
    NoPriorLease,
}

/// Parameters for the converging dispatch tail shared between `resume_json`
/// and `auth_resume_json`.  All fields are resolved by the respective
/// method preamble before the shared tail begins.
struct ResumedDispatchParams<'r> {
    run_state: &'r dyn RunStateStore,
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    estimate: ResourceEstimate,
    input: serde_json::Value,
    trust_decision: TrustDecision,
    authorized_context: ExecutionContext,
    descriptor: &'r CapabilityDescriptor,
    /// Approval-lease state for this resume.  See [`ResumedLeaseState`].
    lease_state: ResumedLeaseState<'r>,
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
            run_state_approval_store: None,
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
        self.run_state_approval_store = None;
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
        self.run_state_approval_store = None;
        self
    }

    /// Attaches a combined durable run-state/approval store that can persist a
    /// pending approval and transition the invocation to `BlockedApproval` in one
    /// transaction. Production composition should prefer this over separate
    /// stores when both records live in the same backend.
    pub fn with_run_state_approval_store(mut self, store: &'a dyn RunStateApprovalStore) -> Self {
        self.run_state = Some(store);
        self.approval_requests = Some(store);
        self.run_state_approval_store = Some(store);
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

    #[tracing::instrument(
        level = "debug",
        skip(self, request),
        fields(
            invocation_id = %request.context.invocation_id,
            capability_id = %request.capability_id,
            scope = ?request.context.resource_scope,
        )
    )]
    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        let scope = request.context.resource_scope.clone();
        if request.context.validate().is_err() {
            debug!("capability invocation rejected invalid execution context");
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
            });
        }
        debug!("capability invocation started");

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
            debug!("capability run state started");
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            debug!("capability invocation failed before authorization: unknown capability");
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
                debug!(
                    obligation_count = allowed_obligations.len(),
                    "capability authorization allowed dispatch"
                );
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
                        debug!("capability invoke obligations prepared");
                    }
                    Err(error) => {
                        debug!(
                            error_kind = obligation_invocation_error_kind(&error),
                            "capability invoke obligation preparation failed"
                        );
                        apply_run_state_transition_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            &error,
                        )
                        .await;
                        return Err(error);
                    }
                }
            }
            Decision::Deny { reason } => {
                debug!(
                    reason = ?reason,
                    "capability authorization denied dispatch"
                );
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
                let approval_request_id = approval.id;
                add_capability_input_display_hint(
                    &mut approval.reason,
                    &request.capability_id,
                    &request.input,
                );
                debug!(
                    approval_request_id = %approval_request_id,
                    "capability authorization requires approval"
                );
                if let Err(error) = validate_approval_request_matches_invocation(
                    &approval,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    CapabilityActionKind::Dispatch,
                ) {
                    debug!(
                        approval_request_id = %approval_request_id,
                        "capability approval request did not match invocation"
                    );
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
                        debug!(
                            approval_request_id = %approval_request_id,
                            "capability approval fingerprint mismatch"
                        );
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
                        if let Some(combined_store) = self.run_state_approval_store {
                            if let Err(error) = combined_store
                                .save_pending_and_block_approval(
                                    scope.clone(),
                                    invocation_id,
                                    approval,
                                )
                                .await
                            {
                                debug!(
                                    approval_request_id = %approval_request_id,
                                    "capability approval block failed in combined store"
                                );
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                            debug!(
                                approval_request_id = %approval_request_id,
                                "capability approval persisted and run state blocked"
                            );
                        } else {
                            let approval_id = approval.id;
                            if let Err(error) = approval_requests
                                .save_pending(scope.clone(), approval.clone())
                                .await
                            {
                                debug!(
                                    approval_request_id = %approval_id,
                                    "capability approval request persistence failed"
                                );
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
                                debug!(
                                    approval_request_id = %approval_id,
                                    "capability run state approval block failed"
                                );
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
                            debug!(
                                approval_request_id = %approval_id,
                                "capability approval persisted and run state blocked"
                            );
                        }
                    }
                    (Some(run_state), None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "approval_requests",
                            "capability approval cannot block because store is missing"
                        );
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
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "run_state",
                            "capability approval cannot block because store is missing"
                        );
                        return Err(CapabilityInvocationError::ApprovalStoreMissing {
                            capability: request.capability_id,
                            store: "run_state",
                        });
                    }
                    (None, None) => {
                        debug!(
                            approval_request_id = %approval_request_id,
                            store = "run_state and approval_requests",
                            "capability approval cannot block because stores are missing"
                        );
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

        debug!("capability dispatch starting");
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
            Ok(dispatch) => {
                debug!(
                    provider = %dispatch.provider,
                    runtime = ?dispatch.runtime,
                    "capability dispatch completed"
                );
                dispatch
            }
            Err(error) => {
                debug!(
                    dispatch_failure_kind = %error.failure_kind(),
                    "capability dispatch failed"
                );
                self.abort_obligations(
                    CapabilityObligationPhase::Invoke,
                    &request.context,
                    &request.capability_id,
                    &request.estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                let invocation_error = CapabilityInvocationError::from(error);
                apply_run_state_transition_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    &invocation_error,
                )
                .await;
                return Err(invocation_error);
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
                debug!(
                    error_kind = obligation_invocation_error_kind(&error),
                    "capability invoke obligation completion failed"
                );
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
            debug!("capability run state completed");
        }

        debug!("capability invocation completed");
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
        // The lease is claimed INSIDE `dispatch_resumed_capability`, after
        // `authorize_dispatch_with_trust` returns Allow.  Deferring the claim
        // preserves the original contract: a Deny leaves the lease Active.
        let grant_id = lease.grant.id;

        self.dispatch_resumed_capability(ResumedDispatchParams {
            run_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            trust_decision: request.trust_decision,
            authorized_context,
            descriptor,
            lease_state: ResumedLeaseState::PendingClaim(PendingClaimAfterAuth {
                leases: capability_leases,
                grant_id,
                fingerprint: invocation_fingerprint,
            }),
        })
        .await
    }

    /// Resume an invocation that was previously blocked at an auth gate.
    ///
    /// Validates that the run record is in `BlockedAuth` status.  When the
    /// invocation also passed an earlier approval gate (`approval_request_id`
    /// is `Some`), validates and claims the fingerprinted approval lease before
    /// dispatch so the prior approval is honoured without a second approval
    /// prompt.  When `approval_request_id` is `None` no lease step is needed
    /// and the path falls through to normal authorization + dispatch.
    pub async fn auth_resume_json(
        &self,
        request: CapabilityAuthResumeRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let run_state =
            self.run_state
                .ok_or_else(|| CapabilityInvocationError::ResumeStoreMissing {
                    capability: request.capability_id.clone(),
                    store: "run_state",
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

        let run_record = run_state
            .get(&scope, invocation_id)
            .await?
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        if run_record.status != RunStatus::BlockedAuth {
            return Err(CapabilityInvocationError::ResumeNotBlocked {
                capability: request.capability_id,
                status: run_record.status,
            });
        }
        // Verify the capability_id on the request matches the one recorded in
        // the run state when the run was originally started.  A mismatch means
        // the caller is trying to resume a different capability than the one
        // that was blocked — treat it as a context mismatch and fail the run.
        if run_record.capability_id != request.capability_id {
            fail_run_if_configured(
                Some(run_state),
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
        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run_if_configured(Some(run_state), &scope, invocation_id, "UnknownCapability")
                .await;
            return Err(CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id,
            });
        };

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
                .ok_or(RunStateError::UnknownApprovalRequest {
                    request_id: approval_request_id,
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
                                "approval lease claim lost to a concurrent auth-resume; leaving run state unchanged",
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
                                "approval lease reuse lost to a concurrent auth-resume; leaving run state unchanged",
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
                                "approval lease reuse lost to a concurrent auth-resume; leaving run state unchanged",
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
                }
            } else {
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

            let mut ctx = request.context.clone();
            ctx.grants.grants.push(claimed.grant.clone());
            (ctx, Some((capability_leases, claimed)))
        } else {
            (request.context.clone(), None)
        };

        self.dispatch_resumed_capability(ResumedDispatchParams {
            run_state,
            scope,
            invocation_id,
            capability_id,
            estimate: request.estimate,
            input: request.input,
            trust_decision: request.trust_decision,
            authorized_context,
            descriptor,
            lease_state: match approval_lease_to_consume {
                Some((leases, lease)) => ResumedLeaseState::AlreadyClaimed(leases, Box::new(lease)),
                None => ResumedLeaseState::NoPriorLease,
            },
        })
        .await
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
                apply_run_state_transition_if_configured(
                    Some(run_state),
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
                        apply_run_state_transition_if_configured(
                            self.run_state,
                            &scope,
                            invocation_id,
                            &error,
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
                add_capability_input_display_hint(
                    &mut approval.reason,
                    &request.capability_id,
                    &request.input,
                );
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
                        if let Some(combined_store) = self.run_state_approval_store {
                            if let Err(error) = combined_store
                                .save_pending_and_block_approval(
                                    scope.clone(),
                                    invocation_id,
                                    approval,
                                )
                                .await
                            {
                                fail_run_if_configured(
                                    Some(run_state),
                                    &scope,
                                    invocation_id,
                                    "ApprovalBlock",
                                )
                                .await;
                                return Err(CapabilityInvocationError::from(error));
                            }
                        } else {
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

    /// Converging tail shared by `resume_json` and `auth_resume_json`.
    ///
    /// Runs: trust-aware authorization → prepare obligations (Resume phase) →
    /// `dispatcher.dispatch_json` → complete dispatch obligations → optional
    /// lease consume → `complete_run_after_side_effect` → Ok.
    ///
    /// On any failure: aborts applicable obligations, transitions run state,
    /// and revokes the claimed lease unless the error is a non-terminal
    /// `BlockAuth` transition (in which case the lease stays Claimed so a
    /// subsequent `auth_resume_json` can reuse it without a second approval).
    async fn dispatch_resumed_capability(
        &self,
        params: ResumedDispatchParams<'_>,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let ResumedDispatchParams {
            run_state,
            scope,
            invocation_id,
            capability_id,
            estimate,
            input,
            trust_decision,
            authorized_context,
            descriptor,
            lease_state,
        } = params;

        let obligations = match self
            .authorizer
            .authorize_dispatch_with_trust(
                &authorized_context,
                descriptor,
                &estimate,
                &trust_decision,
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
                // The AlreadyClaimed lease was transitioned to Dispatching in the
                // auth_resume_json preamble, before this authorization check ran.
                // A Deny is terminal — revoke the lease so it does not stay stuck
                // in Dispatching.  PendingClaim and NoPriorLease have no pre-authz
                // state mutation here.
                if let ResumedLeaseState::AlreadyClaimed(store, lease) = &lease_state
                    && let Err(error) = store.revoke(&scope, lease.grant.id).await
                {
                    warn!(
                        lease_id = %lease.grant.id,
                        revoke_error_kind = capability_lease_error_kind(&error),
                        "failed to revoke reused approval lease after authorization refused auth-resume; lease may remain Dispatching",
                    );
                }
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: capability_id,
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
                // Same as the Deny arm: the AlreadyClaimed lease was transitioned to
                // Dispatching before authorization ran; a RequireApproval refusal is
                // also terminal — revoke so it does not remain stuck in Dispatching.
                if let ResumedLeaseState::AlreadyClaimed(store, lease) = &lease_state
                    && let Err(error) = store.revoke(&scope, lease.grant.id).await
                {
                    warn!(
                        lease_id = %lease.grant.id,
                        revoke_error_kind = capability_lease_error_kind(&error),
                        "failed to revoke reused approval lease after authorization refused auth-resume; lease may remain Dispatching",
                    );
                }
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: capability_id,
                });
            }
        };

        // For `resume_json` (`PendingClaim`), the approval lease is claimed AFTER
        // authorization so that a `Deny` leaves the lease `Active` (the preamble
        // only injects the grant for the authorize call; the actual `Claimed`
        // transition is deferred to this point).
        //
        // For `auth_resume_json` with a prior approval (`AlreadyClaimed`), the
        // lease was already transitioned to `Claimed` in the preamble; reuse it
        // directly.
        //
        // For `auth_resume_json` with no prior approval (`NoPriorLease`), there
        // is no lease to claim or consume.
        let claimed_lease: Option<(&dyn CapabilityLeaseStore, CapabilityLease)> = match lease_state
        {
            ResumedLeaseState::PendingClaim(pc) => {
                let grant_id = pc.grant_id;
                match pc.leases.claim(&scope, grant_id, &pc.fingerprint).await {
                    Ok(claimed) => Some((pc.leases, claimed)),
                    Err(error) => {
                        if claim_error_may_be_concurrent_resume(&error) {
                            warn!(
                                lease_id = %grant_id,
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
                }
            }
            ResumedLeaseState::AlreadyClaimed(leases, lease) => Some((leases, *lease)),
            ResumedLeaseState::NoPriorLease => None,
        };

        let obligation_outcome = match self
            .prepare_obligations(
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &capability_id,
                &estimate,
                obligations.clone(),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                apply_run_state_transition_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    &error,
                )
                .await;
                // Non-terminal auth bounce: revert Dispatching → Claimed so the next
                // auth_resume_json call can find and reuse the lease.
                if let Some((capability_leases, ref claimed)) = claimed_lease {
                    cleanup_claimed_lease_after_resume_error(
                        capability_leases,
                        &scope,
                        claimed.grant.id,
                        invocation_id,
                        &capability_id,
                        &error,
                        "obligation failure",
                    )
                    .await;
                }
                return Err(error);
            }
        };

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: capability_id.clone(),
                scope: scope.clone(),
                estimate: estimate.clone(),
                mounts: obligation_outcome.mounts.clone(),
                resource_reservation: obligation_outcome.resource_reservation.clone(),
                input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.abort_obligations(
                    CapabilityObligationPhase::Resume,
                    &authorized_context,
                    &capability_id,
                    &estimate,
                    obligations.as_slice(),
                    &obligation_outcome,
                )
                .await;
                let invocation_error = CapabilityInvocationError::from(error);
                apply_run_state_transition_if_configured(
                    Some(run_state),
                    &scope,
                    invocation_id,
                    &invocation_error,
                )
                .await;
                // Non-terminal auth bounce: revert Dispatching → Claimed so the next
                // auth_resume_json call can find and reuse the lease.
                if let Some((capability_leases, ref claimed)) = claimed_lease {
                    cleanup_claimed_lease_after_resume_error(
                        capability_leases,
                        &scope,
                        claimed.grant.id,
                        invocation_id,
                        &capability_id,
                        &invocation_error,
                        "dispatch failure",
                    )
                    .await;
                }
                return Err(invocation_error);
            }
        };

        let dispatch = match self
            .complete_dispatch_obligations(
                CapabilityObligationPhase::Resume,
                &authorized_context,
                &capability_id,
                &estimate,
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
                    &capability_id,
                    &estimate,
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
                if let Some((capability_leases, ref claimed)) = claimed_lease
                    && let Err(revoke_error) =
                        capability_leases.revoke(&scope, claimed.grant.id).await
                {
                    warn!(
                        lease_id = %claimed.grant.id,
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

        if let Some((capability_leases, claimed)) = claimed_lease
            && let Err(error) = capability_leases.consume(&scope, claimed.grant.id).await
        {
            warn!(
                lease_id = %claimed.grant.id,
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
            .map_err(|error| prepare_obligation_error_to_invocation(capability_id, error))
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
            .map_err(|error| completion_obligation_error_to_invocation(capability_id, error))
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

fn add_capability_input_display_hint(
    reason: &mut String,
    capability_id: &CapabilityId,
    input: &serde_json::Value,
) {
    let capability_id = capability_id.as_str();
    if capability_id != "shell"
        && capability_id != "builtin.shell"
        && !capability_id.ends_with(".shell")
    {
        return;
    }
    let Some(command) = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(shell_command_display_text)
    else {
        return;
    };
    if command.text.is_empty() {
        return;
    }
    reason.push_str("\n\nCommand:\n");
    reason.push_str(&command.text);
    if command.truncated {
        reason.push_str("\n[truncated]");
    }
}

/// Cleans up a claimed lease after a resume-path error using best-effort
/// abort-or-revoke semantics.
///
/// - If `error` is a `BlockAuth` (non-terminal auth gate), aborts the
///   `Dispatching` lease back to `Claimed` so the next `auth_resume_json`
///   call can reuse it without a new human approval.
/// - Otherwise revokes the lease terminally.
///
/// Both operations are best-effort: failures are logged as warnings and do
/// not propagate — the caller should already be returning an error.
///
/// `revoke_context` names the failure site ("obligation failure" or
/// "dispatch failure") and is included in the revoke warn message.
async fn cleanup_claimed_lease_after_resume_error(
    capability_leases: &dyn CapabilityLeaseStore,
    scope: &ResourceScope,
    claimed_grant_id: CapabilityGrantId,
    invocation_id: InvocationId,
    capability_id: &CapabilityId,
    error: &CapabilityInvocationError,
    revoke_context: &str,
) {
    if is_block_auth_transition(error) {
        if let Err(abort_error) = capability_leases
            .abort_dispatch_claimed(scope, claimed_grant_id)
            .await
        {
            warn!(
                lease_id = %claimed_grant_id,
                invocation_id = %invocation_id,
                capability_id = %capability_id,
                abort_error_kind = capability_lease_error_kind(&abort_error),
                "capability lease abort-dispatch failed after non-terminal auth bounce; lease may remain Dispatching",
            );
        }
    } else if let Err(revoke_error) = capability_leases.revoke(scope, claimed_grant_id).await {
        warn!(
            lease_id = %claimed_grant_id,
            invocation_id = %invocation_id,
            capability_id = %capability_id,
            revoke_error_kind = capability_lease_error_kind(&revoke_error),
            "capability lease revoke failed after {revoke_context}; lease may remain claimed",
        );
    }
}

/// Returns `true` when the error will transition the run to `BlockedAuth`
/// (a non-terminal, retriable auth gate).  Used to decide whether to skip
/// the post-claim lease revoke so `auth_resume_json` can reuse the same
/// Claimed lease without requiring a new human approval.
fn is_block_auth_transition(error: &CapabilityInvocationError) -> bool {
    matches!(
        error.run_state_transition(),
        Some(CapabilityRunStateTransition::BlockAuth { .. })
    )
}

fn prepare_obligation_error_to_invocation(
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
        CapabilityObligationError::AuthRequired {
            credential_requirements,
        } => CapabilityInvocationError::AuthorizationRequiresAuth {
            capability: capability_id.clone(),
            required_secrets: Vec::new(),
            credential_requirements,
        },
        CapabilityObligationError::Failed { kind } => CapabilityInvocationError::ObligationFailed {
            capability: capability_id.clone(),
            kind,
        },
    }
}

fn completion_obligation_error_to_invocation(
    capability_id: &ironclaw_host_api::CapabilityId,
    error: CapabilityObligationError,
) -> CapabilityInvocationError {
    match error {
        CapabilityObligationError::AuthRequired { .. } => {
            CapabilityInvocationError::ObligationFailed {
                capability: capability_id.clone(),
                kind: CapabilityObligationFailureKind::Secret,
            }
        }
        other => prepare_obligation_error_to_invocation(capability_id, other),
    }
}

fn obligation_invocation_error_kind(error: &CapabilityInvocationError) -> &'static str {
    // `run_state_transition` returns `None` for `CapabilityInvocationError::Dispatch`
    // because PR #4236 handles those failures via the disposition policy on the
    // outcome path. The obligation call sites only see this function for
    // diagnostic logging; fall back to a stable "Dispatch" label in that case.
    error
        .run_state_transition()
        .map(CapabilityRunStateTransition::error_kind)
        .unwrap_or("Dispatch")
}
