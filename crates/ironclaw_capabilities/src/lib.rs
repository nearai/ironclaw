//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization and runtime dispatch without making callers
//! understand grant evaluation and without making the dispatcher own auth.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::{
    CapabilityDispatchAuthorizer, CapabilityLease, CapabilityLeaseError, CapabilityLeaseStore,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatchError, CapabilityDispatchRequest,
    CapabilityDispatchResult, CapabilityDispatcher, CapabilityId, Decision, DenyReason, ErrorKind,
    ExecutionContext, HostApiError, InvocationFingerprint, InvocationId, ProcessId,
    ResourceEstimate, ResourceScope,
};
use ironclaw_processes::{
    ProcessError, ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessManager, ProcessRecord, ProcessResourceReservation, ProcessStart,
};
use ironclaw_run_state::{
    ApprovalRequestStore, ApprovalStatus, RunStart, RunStateError, RunStateStore, RunStatus,
};
use serde_json::Value;
use thiserror::Error;

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
    #[error("lease update failed: {0}")]
    Lease(Box<CapabilityLeaseError>),
    #[error("run-state update failed: {0}")]
    RunState(Box<RunStateError>),
    #[error("process update failed: {0}")]
    Process(Box<ProcessError>),
    #[error("dispatch failed: {0}")]
    Dispatch(Box<CapabilityDispatchError>),
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

impl From<CapabilityDispatchError> for CapabilityInvocationError {
    fn from(error: CapabilityDispatchError) -> Self {
        Self::Dispatch(Box::new(error))
    }
}

/// Process executor adapter that runs a spawned capability through the authorized dispatch path.
pub struct DispatchProcessExecutor<D>
where
    D: CapabilityDispatcher + ?Sized,
{
    dispatcher: Arc<D>,
}

impl<D> DispatchProcessExecutor<D>
where
    D: CapabilityDispatcher + ?Sized,
{
    pub fn new(dispatcher: Arc<D>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl<D> ProcessExecutor for DispatchProcessExecutor<D>
where
    D: CapabilityDispatcher + ?Sized,
{
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        let expected_provider = request.extension_id.clone();
        let expected_runtime = request.runtime;
        let dispatch_estimate = if request.resource_reservation_id.is_some() {
            ResourceEstimate::default()
        } else {
            request.estimate
        };
        let dispatch = self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: request.scope,
                estimate: dispatch_estimate,
                input: request.input,
            })
            .await
            .map_err(|error| ProcessExecutionError::new(error.error_kind()))?;
        if dispatch.provider != expected_provider || dispatch.runtime != expected_runtime {
            return Err(ProcessExecutionError::new(ErrorKind::new(
                "ProcessDispatchInvariantViolation",
            )));
        }
        Ok(ProcessExecutionResult {
            output: dispatch.output,
        })
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

    pub fn with_run_state(mut self, run_state: &'a dyn RunStateStore) -> Self {
        self.run_state = Some(run_state);
        self
    }

    pub fn with_approval_requests(
        mut self,
        approval_requests: &'a dyn ApprovalRequestStore,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self
    }

    pub fn with_capability_leases(
        mut self,
        capability_leases: &'a dyn CapabilityLeaseStore,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_process_manager(mut self, process_manager: &'a dyn ProcessManager) -> Self {
        self.process_manager = Some(process_manager);
        self
    }

    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        if request.context.validate().is_err() {
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
            });
        }

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
                    capability_id,
                    scope: scope.clone(),
                })
                .await?;
        }

        let descriptor = match self.registry.get_capability(&request.capability_id) {
            Some(descriptor) => descriptor,
            None => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(&scope, invocation_id, "UnknownCapability".to_string())
                        .await?;
                }
                return Err(CapabilityInvocationError::UnknownCapability {
                    capability: request.capability_id,
                });
            }
        };

        match self
            .authorizer
            .authorize_dispatch(&request.context, descriptor, &request.estimate)
        {
            Decision::Allow { .. } => {}
            Decision::Deny { reason } => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .block_auth(&scope, invocation_id, "AuthorizationDenied".to_string())
                        .await?;
                }
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
                        if let Some(run_state) = self.run_state {
                            run_state
                                .fail(
                                    &scope,
                                    invocation_id,
                                    "InvocationFingerprintMismatch".to_string(),
                                )
                                .await?;
                        }
                        return Err(CapabilityInvocationError::AuthorizationDenied {
                            capability: request.capability_id,
                            reason: DenyReason::InternalInvariantViolation,
                        });
                    }
                } else {
                    approval.invocation_fingerprint = Some(invocation_fingerprint.clone());
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
                        run_state
                            .fail(&scope, invocation_id, "ApprovalStoreMissing".to_string())
                            .await?;
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
                    (None, None) => {}
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
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(&scope, invocation_id, "Dispatch".to_string())
                        .await?;
                }
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            run_state.complete(&scope, invocation_id).await?;
        }

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

        let descriptor = match self.registry.get_capability(&request.capability_id) {
            Some(descriptor) => descriptor,
            None => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(&scope, invocation_id, "UnknownCapability".to_string())
                        .await?;
                }
                return Err(CapabilityInvocationError::UnknownCapability {
                    capability: request.capability_id,
                });
            }
        };

        match self
            .authorizer
            .authorize_spawn(&request.context, descriptor, &request.estimate)
        {
            Decision::Allow { .. } => {}
            Decision::Deny { reason } => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(&scope, invocation_id, "AuthorizationDenied".to_string())
                        .await?;
                }
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { .. } => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(
                            &scope,
                            invocation_id,
                            "AuthorizationRequiresApproval".to_string(),
                        )
                        .await?;
                }
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
                resource_reservation: ProcessResourceReservation::none(),
                input: request.input,
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                if let Some(run_state) = self.run_state {
                    run_state
                        .fail(&scope, invocation_id, "ProcessSpawn".to_string())
                        .await?;
                }
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            run_state.complete(&scope, invocation_id).await?;
        }

        Ok(CapabilitySpawnResult { process })
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
        if run_record.capability_id != request.capability_id
            || run_record.approval_request_id != Some(request.approval_request_id)
        {
            fail_run(
                run_state,
                &scope,
                invocation_id,
                "ApprovalInvariantViolation",
            )
            .await?;
            return Err(CapabilityInvocationError::AuthorizationDenied {
                capability: request.capability_id,
                reason: DenyReason::InternalInvariantViolation,
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

        let descriptor = match self.registry.get_capability(&request.capability_id) {
            Some(descriptor) => descriptor,
            None => {
                fail_run(run_state, &scope, invocation_id, "UnknownCapability").await?;
                return Err(CapabilityInvocationError::UnknownCapability {
                    capability: request.capability_id,
                });
            }
        };

        let Some(lease) = matching_approval_lease(
            capability_leases,
            &request.context,
            &request.capability_id,
            &invocation_fingerprint,
        ) else {
            fail_run(run_state, &scope, invocation_id, "ApprovalLeaseMissing").await?;
            return Err(CapabilityInvocationError::ApprovalLeaseMissing {
                capability: request.capability_id,
            });
        };
        let claimed_lease =
            match capability_leases.claim(&scope, lease.grant.id, &invocation_fingerprint) {
                Ok(lease) => lease,
                Err(error) => {
                    fail_run(run_state, &scope, invocation_id, "ApprovalLeaseClaim").await?;
                    return Err(CapabilityInvocationError::Lease(Box::new(error)));
                }
            };
        let mut authorized_context = request.context.clone();
        authorized_context
            .grants
            .grants
            .push(claimed_lease.grant.clone());

        match self
            .authorizer
            .authorize_dispatch(&authorized_context, descriptor, &request.estimate)
        {
            Decision::Allow { .. } => {}
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

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id.clone(),
                scope: scope.clone(),
                estimate: request.estimate,
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

        if let Err(error) = capability_leases.consume(&scope, claimed_lease.grant.id) {
            fail_run(run_state, &scope, invocation_id, "LeaseConsumption").await?;
            return Err(CapabilityInvocationError::Lease(Box::new(error)));
        }
        run_state.complete(&scope, invocation_id).await?;

        Ok(CapabilityInvocationResult { dispatch })
    }
}

fn matching_approval_lease(
    capability_leases: &dyn CapabilityLeaseStore,
    context: &ExecutionContext,
    capability_id: &CapabilityId,
    invocation_fingerprint: &InvocationFingerprint,
) -> Option<CapabilityLease> {
    capability_leases
        .active_leases_for_context(context)
        .into_iter()
        .find(|lease| {
            lease.scope == context.resource_scope
                && lease.grant.capability == *capability_id
                && lease.invocation_fingerprint.as_ref() == Some(invocation_fingerprint)
        })
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
