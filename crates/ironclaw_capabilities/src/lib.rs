//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization, approval resume, run-state transitions, and
//! neutral runtime dispatch without depending on concrete runtime crates.

use ironclaw_authorization::{
    CapabilityDispatchAuthorizer, CapabilityLease, CapabilityLeaseError, CapabilityLeaseStore,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityId, Decision, DenyReason, DispatchError, ExecutionContext, HostApiError,
    InvocationFingerprint, InvocationId, Obligation, ProcessId, ResourceEstimate, ResourceScope,
};
use ironclaw_processes::{ProcessError, ProcessManager, ProcessRecord, ProcessStart};
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStart, RunStateError, RunStateStore, RunStatus,
};
use serde_json::Value;
use thiserror::Error;
use tracing::warn;

/// Caller-facing capability invocation request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityInvocationRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Caller-facing approved capability resume request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityResumeRequest {
    pub context: ExecutionContext,
    pub approval_request_id: ApprovalRequestId,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Caller-facing capability spawn request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilitySpawnRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Caller-facing capability invocation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocationResult {
    pub dispatch: CapabilityDispatchResult,
}

/// Caller-facing capability spawn result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySpawnResult {
    pub process: ProcessRecord,
}

/// Redacted reason a resume request did not match the blocked invocation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeContextMismatchKind {
    CapabilityId,
    ApprovalRequestId,
    CapabilityAndApprovalRequestId,
}

/// Capability invocation failures before or during dispatch.
#[derive(Debug, Error)]
pub enum CapabilityInvocationError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} invocation denied: {reason:?}")]
    AuthorizationDenied {
        capability: CapabilityId,
        reason: DenyReason,
    },
    #[error("capability {capability} returned unsupported authorization obligations")]
    UnsupportedObligations {
        capability: CapabilityId,
        obligations: Vec<Obligation>,
    },
    #[error("capability {capability} invocation requires approval")]
    AuthorizationRequiresApproval { capability: CapabilityId },
    #[error("capability {capability} invocation fingerprint failed: {source}")]
    InvocationFingerprint {
        capability: CapabilityId,
        source: HostApiError,
    },
    #[error("capability {capability} approval fingerprint mismatch")]
    ApprovalFingerprintMismatch { capability: CapabilityId },
    #[error("capability {capability} approval is not approved: {status:?}")]
    ApprovalNotApproved {
        capability: CapabilityId,
        status: ApprovalStatus,
    },
    #[error("capability {capability} approval path requires {store}")]
    ApprovalStoreMissing {
        capability: CapabilityId,
        store: &'static str,
    },
    #[error("capability {capability} approval lease is missing")]
    ApprovalLeaseMissing { capability: CapabilityId },
    #[error("capability {capability} resume requires {store}")]
    ResumeStoreMissing {
        capability: CapabilityId,
        store: &'static str,
    },
    #[error("capability {capability} spawn requires a process manager")]
    ProcessManagerMissing { capability: CapabilityId },
    #[error("capability {capability} cannot resume from run status {status:?}")]
    ResumeNotBlocked {
        capability: CapabilityId,
        status: RunStatus,
    },
    #[error("capability {capability} resume context mismatch: {kind:?}")]
    ResumeContextMismatch {
        capability: CapabilityId,
        kind: ResumeContextMismatchKind,
    },
    #[error("lease update failed: {0}")]
    Lease(Box<CapabilityLeaseError>),
    #[error("run-state update failed: {0}")]
    RunState(Box<RunStateError>),
    #[error("process update failed: {0}")]
    Process(Box<ProcessError>),
    /// Runtime dispatch failure surfaced through the neutral host API port.
    ///
    /// `kind` is a stable, redacted identifier produced by
    /// [`dispatch_error_kind`]. The mapping is part of the public contract:
    /// upstream callers may depend on these strings for routing, metrics, or
    /// audit grouping. The mapping is pinned by unit tests in this crate.
    #[error("dispatch failed: {kind}")]
    Dispatch { kind: String },
}

impl From<RunStateError> for CapabilityInvocationError {
    fn from(error: RunStateError) -> Self {
        Self::RunState(Box::new(error))
    }
}

impl From<ProcessError> for CapabilityInvocationError {
    fn from(error: ProcessError) -> Self {
        Self::Process(Box::new(error))
    }
}

impl From<DispatchError> for CapabilityInvocationError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch {
            kind: dispatch_error_kind(&error),
        }
    }
}

/// Host-facing capability invocation service.
pub struct CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    registry: &'a ExtensionRegistry,
    dispatcher: &'a D,
    authorizer: &'a dyn CapabilityDispatchAuthorizer,
    run_state: Option<&'a dyn RunStateStore>,
    approval_requests: Option<&'a dyn ApprovalRequestStore>,
    capability_leases: Option<&'a dyn CapabilityLeaseStore>,
    process_manager: Option<&'a dyn ProcessManager>,
}

impl<'a, D> CapabilityHost<'a, D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        dispatcher: &'a D,
        authorizer: &'a dyn CapabilityDispatchAuthorizer,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            process_manager: None,
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

        let invocation_fingerprint = InvocationFingerprint::for_dispatch(
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

        match self
            .authorizer
            .authorize_dispatch(&request.context, descriptor, &request.estimate)
            .await
        {
            Decision::Allow { obligations } => {
                if let Err(error) =
                    ensure_no_obligations(&request.capability_id, obligations.into_vec())
                {
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "UnsupportedObligations",
                    )
                    .await;
                    return Err(error);
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
                        approval_requests
                            .save_pending(scope.clone(), approval.clone())
                            .await?;
                        run_state
                            .block_approval(&scope, invocation_id, approval)
                            .await?;
                    }
                    (Some(run_state), None) => {
                        let _ = run_state
                            .fail(&scope, invocation_id, "ApprovalStoreMissing".to_string())
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
                capability_id: request.capability_id,
                scope: scope.clone(),
                estimate: request.estimate,
                mounts: None,
                resource_reservation: None,
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                fail_run_if_configured(self.run_state, &scope, invocation_id, "Dispatch").await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            run_state.complete(&scope, invocation_id).await?;
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

        let invocation_fingerprint = InvocationFingerprint::for_dispatch(
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
            fail_run(run_state, &scope, invocation_id, "ResumeContextMismatch").await?;
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
            fail_run(
                run_state,
                &scope,
                invocation_id,
                approval_not_approved_error_kind(approval.status),
            )
            .await?;
            return Err(CapabilityInvocationError::ApprovalNotApproved {
                capability: request.capability_id,
                status: approval.status,
            });
        }
        if approval.request.invocation_fingerprint.as_ref() != Some(&invocation_fingerprint) {
            fail_run(
                run_state,
                &scope,
                invocation_id,
                "InvocationFingerprintMismatch",
            )
            .await?;
            return Err(CapabilityInvocationError::ApprovalFingerprintMismatch {
                capability: request.capability_id,
            });
        }

        let Some(descriptor) = self.registry.get_capability(&request.capability_id) else {
            fail_run(run_state, &scope, invocation_id, "UnknownCapability").await?;
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
            fail_run(run_state, &scope, invocation_id, "ApprovalLeaseMissing").await?;
            return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                capability: request.capability_id,
            });
        };
        let mut authorized_context = request.context.clone();
        authorized_context.grants.grants.push(lease.grant.clone());

        match self
            .authorizer
            .authorize_dispatch(&authorized_context, descriptor, &request.estimate)
            .await
        {
            Decision::Allow { obligations } => {
                if let Err(error) = ensure_no_obligations(&capability_id, obligations.into_vec()) {
                    fail_run(run_state, &scope, invocation_id, "UnsupportedObligations").await?;
                    return Err(error);
                }
            }
            Decision::Deny { reason } => {
                fail_run(run_state, &scope, invocation_id, "AuthorizationDenied").await?;
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { .. } => {
                fail_run(
                    run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationRequiresApproval",
                )
                .await?;
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let claimed_lease = match capability_leases
            .claim(&scope, lease.grant.id, &invocation_fingerprint)
            .await
        {
            Ok(lease) => lease,
            Err(error) => {
                fail_run(run_state, &scope, invocation_id, "ApprovalLeaseClaim").await?;
                return Err(CapabilityInvocationError::Lease(Box::new(error)));
            }
        };

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: scope.clone(),
                estimate: request.estimate,
                mounts: None,
                resource_reservation: None,
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                fail_run(run_state, &scope, invocation_id, "Dispatch").await?;
                return Err(CapabilityInvocationError::from(error));
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

        run_state.complete(&scope, invocation_id).await?;
        Ok(CapabilityInvocationResult { dispatch })
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

        match self
            .authorizer
            .authorize_spawn(&request.context, descriptor, &request.estimate)
            .await
        {
            Decision::Allow { obligations } => {
                if let Err(error) =
                    ensure_no_obligations(&request.capability_id, obligations.into_vec())
                {
                    fail_run_if_configured(
                        self.run_state,
                        &scope,
                        invocation_id,
                        "UnsupportedObligations",
                    )
                    .await;
                    return Err(error);
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
            Decision::RequireApproval { .. } => {
                fail_run_if_configured(
                    self.run_state,
                    &scope,
                    invocation_id,
                    "AuthorizationRequiresApproval",
                )
                .await;
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let process = match process_manager
            .spawn(ProcessStart {
                process_id: ProcessId::new(),
                parent_process_id: request.context.process_id,
                invocation_id,
                scope: scope.clone(),
                extension_id: descriptor.provider.clone(),
                capability_id: request.capability_id,
                runtime: descriptor.runtime,
                grants: request.context.grants,
                mounts: request.context.mounts,
                estimated_resources: request.estimate,
                resource_reservation_id: None,
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                fail_run_if_configured(self.run_state, &scope, invocation_id, "ProcessSpawn").await;
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            run_state.complete(&scope, invocation_id).await?;
        }

        Ok(CapabilitySpawnResult { process })
    }
}

fn ensure_no_obligations(
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

async fn matching_approval_lease(
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

async fn fail_run_if_configured(
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

async fn fail_run(
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

fn approval_not_approved_error_kind(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "ApprovalPending",
        ApprovalStatus::Approved => "ApprovalApproved",
        ApprovalStatus::Denied => "ApprovalDenied",
        ApprovalStatus::Expired => "ApprovalExpired",
    }
}

fn resume_context_mismatch_kind(
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

fn capability_lease_error_kind(error: &CapabilityLeaseError) -> &'static str {
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

fn run_state_error_kind(error: &RunStateError) -> &'static str {
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

fn dispatch_error_kind(error: &DispatchError) -> String {
    match error {
        DispatchError::UnknownCapability { .. } => "UnknownCapability".to_string(),
        DispatchError::UnknownProvider { .. } => "UnknownProvider".to_string(),
        DispatchError::RuntimeMismatch { .. } => "RuntimeMismatch".to_string(),
        DispatchError::MissingRuntimeBackend { .. } => "MissingRuntimeBackend".to_string(),
        DispatchError::UnsupportedRuntime { .. } => "UnsupportedRuntime".to_string(),
        DispatchError::Mcp { kind }
        | DispatchError::Script { kind }
        | DispatchError::Wasm { kind } => kind.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{ExtensionId, RuntimeDispatchErrorKind, RuntimeKind};

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn ext() -> ExtensionId {
        ExtensionId::new("test").unwrap()
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_capability_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownCapability { capability: cap() });
        assert_eq!(kind, "UnknownCapability");
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_provider_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownProvider {
            capability: cap(),
            provider: ext(),
        });
        assert_eq!(kind, "UnknownProvider");
    }

    #[test]
    fn dispatch_error_kind_maps_runtime_mismatch_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::RuntimeMismatch {
            capability: cap(),
            descriptor_runtime: RuntimeKind::Wasm,
            package_runtime: RuntimeKind::Mcp,
        });
        assert_eq!(kind, "RuntimeMismatch");
    }

    #[test]
    fn dispatch_error_kind_maps_missing_runtime_backend_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind, "MissingRuntimeBackend");
    }

    #[test]
    fn dispatch_error_kind_maps_unsupported_runtime_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnsupportedRuntime {
            capability: cap(),
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind, "UnsupportedRuntime");
    }

    #[test]
    fn dispatch_error_kind_forwards_mcp_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::Backend,
        });
        assert_eq!(kind, "Backend");
    }

    #[test]
    fn dispatch_error_kind_forwards_script_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Script {
            kind: RuntimeDispatchErrorKind::OutputTooLarge,
        });
        assert_eq!(kind, "OutputTooLarge");
    }

    #[test]
    fn dispatch_error_kind_forwards_wasm_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Memory,
        });
        assert_eq!(kind, "Memory");
    }

    #[test]
    fn from_dispatch_error_flattens_via_dispatch_error_kind() {
        let err =
            CapabilityInvocationError::from(DispatchError::UnknownCapability { capability: cap() });
        match err {
            CapabilityInvocationError::Dispatch { kind } => assert_eq!(kind, "UnknownCapability"),
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_error_flattens_redacted_runtime_kind() {
        let err = CapabilityInvocationError::from(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        });
        match err {
            CapabilityInvocationError::Dispatch { kind } => assert_eq!(kind, "Guest"),
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }
}
