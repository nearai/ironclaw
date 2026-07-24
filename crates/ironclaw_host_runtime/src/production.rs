//! Production composition of the [`HostRuntime`] contract.
//!
//! [`DefaultHostRuntime`] is the contract-level facade that upper turn/loop
//! services should depend on. Internally it composes
//! [`ironclaw_capabilities::CapabilityHost`] with neutral kernel services —
//! extension registry, capability dispatcher, trust-aware authorizer,
//! run-state and approval stores, capability-lease store, and process
//! manager.
//!
//! Trust classification and runtime-policy planning are computed inside the
//! capability kernel's `authorize()` fold ([`CapabilityHost`]); this layer
//! composes that kernel with the neutral services and maps its results back to
//! the [`HostRuntime`] contract. The default fail-closed trust policy denies
//! authority until composition supplies a concrete host policy.

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_approvals::{
    PersistentApprovalAction, PersistentApprovalPolicyKey, PersistentApprovalPolicyStore,
    PersistentApprovalScope,
};
use ironclaw_authorization::{CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityInvocationError, CapabilityInvocationResult,
    CapabilityObligationHandler, CapabilitySpawnRequest, CapabilitySpawnResult,
};
use ironclaw_extensions::{ExtensionRegistry, SharedExtensionRegistry};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatcher, CapabilityId, DenyReason, DispatchFailureKind,
    InvocationId, Principal, ResourceScope, RuntimeCredentialAuthRequirement,
    RuntimeDispatchErrorKind, RuntimeKind, SecretHandle, runtime_policy::EffectiveRuntimePolicy,
    sha256_digest_token,
};
use ironclaw_observability::live_latency_started_at;
use ironclaw_process_sandbox::{
    PROCESS_SANDBOX_CAPABILITY_ID, SandboxProcessPlan, ValidatedSandboxProcessPlan,
};
use ironclaw_processes::{
    ProcessCancellationRegistry, ProcessError, ProcessHost, ProcessManager, ProcessResultStore,
    ProcessStart, ProcessStatus, ProcessStore,
};
use ironclaw_run_state::{
    ApprovalRequestStore, RunStateApprovalStore, RunStateError, RunStateStore, RunStatus,
};
use ironclaw_secrets::SecretStore;
use ironclaw_trust::{HostTrustPolicy, TrustPolicy};
use ironclaw_turns::run_profile::LoopSafeSummary;

fn trace_capability_latency_ok(
    operation: &'static str,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    started_at: Option<Instant>,
) {
    ironclaw_observability::live_latency_trace_ok!(
        "host_runtime",
        operation,
        started_at,
        capability_id = %capability_id,
        tenant_id = %scope.tenant_id,
        user_id = %scope.user_id,
        agent_id = scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        mission_id = scope.mission_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = scope.thread_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        invocation_id = %scope.invocation_id,
        "host runtime capability operation completed",
    );
}

fn trace_capability_latency_error<E: ?Sized>(
    operation: &'static str,
    capability_id: &CapabilityId,
    scope: &ResourceScope,
    started_at: Option<Instant>,
    _error: &E,
) {
    ironclaw_observability::live_latency_trace_error!(
        "host_runtime",
        operation,
        started_at,
        "error",
        capability_id = %capability_id,
        tenant_id = %scope.tenant_id,
        user_id = %scope.user_id,
        agent_id = scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        mission_id = scope.mission_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = scope.thread_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        invocation_id = %scope.invocation_id,
        "host runtime capability operation failed",
    );
}

use crate::{
    BuiltinObligationHandler, BuiltinObligationServices, CancelRuntimeWorkOutcome,
    CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate, RuntimeApprovalResume,
    RuntimeAuthGate, RuntimeAuthResume, RuntimeBackendHealth, RuntimeBlockedReason,
    RuntimeCapabilityCompleted, RuntimeCapabilityFailure, RuntimeCapabilityOutcome,
    RuntimeFailureKind, RuntimeGateId, RuntimeInvocation, RuntimeStatusRequest, RuntimeWorkId,
    RuntimeWorkSummary, VisibleCapabilityRequest, VisibleCapabilitySurface,
    obligations::secret_owner_scope, surface::CapabilityCatalog,
};

/// Default production wiring for [`HostRuntime`].
pub struct DefaultHostRuntime {
    registry: Arc<SharedExtensionRegistry>,
    dispatcher: Arc<dyn CapabilityDispatcher>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    trust_policy: Arc<dyn TrustPolicy>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    run_state_approval_store: Option<Arc<dyn RunStateApprovalStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    // arch-exempt: optional_arc, minimal/test compositions intentionally disable
    // persistent approval replay until the product revoke control plane is split out,
    // plan #4539
    persistent_approval_policies: Option<Arc<dyn PersistentApprovalPolicyStore>>,
    process_manager: Option<Arc<dyn ProcessManager>>,
    process_store: Option<Arc<dyn ProcessStore>>,
    process_result_store: Option<Arc<dyn ProcessResultStore>>,
    process_cancellation_registry: Option<Arc<ProcessCancellationRegistry>>,
    surface_filesystem: Option<Arc<dyn RootFilesystem>>,
    runtime_health: Option<Arc<dyn RuntimeBackendHealth>>,
    obligation_handler: Option<Arc<dyn CapabilityObligationHandler>>,
    /// Optional secret store used for pre-flight credential presence checks.
    ///
    /// When present, capability dispatch (both `invoke_capability` and
    /// `spawn_capability`) checks whether all required credentials declared in the
    /// capability manifest are present before the authorization step. This surfaces
    /// `AuthRequired` ahead of the approval gate so users are never asked to
    /// approve an action that cannot yet execute.
    ///
    /// When absent the pre-flight is skipped; the dispatch-time obligation check
    /// remains the enforcement backstop regardless.
    // arch-exempt: optional_arc, credential pre-flight is disabled in minimal/test
    // host-runtime graphs that do not wire a secret store, plan #4539 (Fix B)
    credential_preflight_store: Option<Arc<dyn SecretStore>>,
    surface_version: CapabilitySurfaceVersion,
    runtime_policy: EffectiveRuntimePolicy,
}

impl DefaultHostRuntime {
    /// Constructs a default host runtime over the supplied kernel services.
    ///
    /// This constructor snapshots the supplied registry into an internal
    /// [`SharedExtensionRegistry`]. Use [`Self::from_shared_registry`] when
    /// callers need subsequent registry mutations to be shared with the runtime.
    ///
    /// The runtime starts with an explicit fail-closed host trust policy, so
    /// capability dispatch is denied until composition attaches a concrete
    /// policy with [`Self::with_trust_policy`] or [`Self::with_trust_policy_dyn`].
    ///
    /// Callers must additionally attach either a combined
    /// [`RunStateApprovalStore`] via
    /// [`with_run_state_approval_store`](Self::with_run_state_approval_store),
    /// or separate stores via [`with_run_state`](Self::with_run_state) and
    /// [`with_approval_requests`](Self::with_approval_requests), before
    /// invoking any capability whose authorizer may return
    /// `RequireApproval`. Without those stores the capability host fails
    /// closed with `ApprovalStoreMissing`, which surfaces here as a
    /// [`RuntimeCapabilityOutcome::Failed`] rather than blocking for human
    /// review.
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        dispatcher: Arc<dyn CapabilityDispatcher>,
        authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
        surface_version: CapabilitySurfaceVersion,
        runtime_policy: EffectiveRuntimePolicy,
    ) -> Self {
        Self::from_shared_registry(
            Arc::new(SharedExtensionRegistry::new((*registry).clone())),
            dispatcher,
            authorizer,
            surface_version,
            runtime_policy,
        )
    }

    pub fn from_shared_registry(
        registry: Arc<SharedExtensionRegistry>,
        dispatcher: Arc<dyn CapabilityDispatcher>,
        authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
        surface_version: CapabilitySurfaceVersion,
        runtime_policy: EffectiveRuntimePolicy,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            trust_policy: Arc::new(HostTrustPolicy::fail_closed()),
            run_state: None,
            approval_requests: None,
            run_state_approval_store: None,
            capability_leases: None,
            persistent_approval_policies: None,
            process_manager: None,
            process_store: None,
            process_result_store: None,
            process_cancellation_registry: None,
            surface_filesystem: None,
            runtime_health: None,
            obligation_handler: None,
            credential_preflight_store: None,
            surface_version,
            runtime_policy,
        }
    }

    /// Attaches the host-owned trust policy used to evaluate each provider's
    /// manifest-derived trust input immediately before capability dispatch.
    pub fn with_trust_policy<T>(mut self, trust_policy: Arc<T>) -> Self
    where
        T: TrustPolicy + 'static,
    {
        self.trust_policy = trust_policy;
        self
    }

    /// Attaches an already-erased host-owned trust policy.
    pub fn with_trust_policy_dyn(mut self, trust_policy: Arc<dyn TrustPolicy>) -> Self {
        self.trust_policy = trust_policy;
        self
    }

    /// Attaches the resolved runtime policy that structurally gates each
    /// capability invocation and visible-capability projection.
    pub fn with_runtime_policy(mut self, policy: EffectiveRuntimePolicy) -> Self {
        self.runtime_policy = policy;
        self
    }

    pub fn with_surface_filesystem(mut self, filesystem: Arc<dyn RootFilesystem>) -> Self {
        self.surface_filesystem = Some(filesystem);
        self
    }

    /// Attaches the run-state store used to record invocation lifecycle.
    pub fn with_run_state(mut self, run_state: Arc<dyn RunStateStore>) -> Self {
        self.run_state = Some(run_state);
        self.run_state_approval_store = None;
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    pub fn with_approval_requests(
        mut self,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
        self.run_state_approval_store = None;
        self
    }

    /// Attaches a combined durable run-state/approval-request store with an
    /// atomic approval-block transition.
    pub fn with_run_state_approval_store(mut self, store: Arc<dyn RunStateApprovalStore>) -> Self {
        self.run_state = Some(store.clone());
        self.approval_requests = Some(store.clone());
        self.run_state_approval_store = Some(store);
        self
    }

    /// Attaches the capability-lease store used by approval resume paths.
    pub fn with_capability_leases(
        mut self,
        capability_leases: Arc<dyn CapabilityLeaseStore>,
    ) -> Self {
        self.capability_leases = Some(capability_leases);
        self
    }

    /// Attaches reusable approval policy overrides used to inject scoped,
    /// manifest-bounded grants before ordinary authorization.
    pub fn with_persistent_approval_policies(
        mut self,
        policies: Arc<dyn PersistentApprovalPolicyStore>,
    ) -> Self {
        self.persistent_approval_policies = Some(policies);
        self
    }

    /// Attaches the process manager used by future spawn paths.
    pub fn with_process_manager(mut self, process_manager: Arc<dyn ProcessManager>) -> Self {
        self.process_manager = Some(process_manager);
        self
    }

    /// Attaches the process store used for status and cancellation fanout.
    pub fn with_process_store(mut self, process_store: Arc<dyn ProcessStore>) -> Self {
        self.process_store = Some(process_store);
        self
    }

    /// Attaches the process result store used to persist cancellation results.
    pub fn with_process_result_store(
        mut self,
        process_result_store: Arc<dyn ProcessResultStore>,
    ) -> Self {
        self.process_result_store = Some(process_result_store);
        self
    }

    /// Attaches the process cancellation registry used to notify running
    /// background executors when `cancel_work` kills a process record.
    pub fn with_process_cancellation_registry(
        mut self,
        registry: Arc<ProcessCancellationRegistry>,
    ) -> Self {
        self.process_cancellation_registry = Some(registry);
        self
    }

    /// Attaches the backend health probe for concrete runtime implementations.
    pub fn with_runtime_health(mut self, health: Arc<dyn RuntimeBackendHealth>) -> Self {
        self.runtime_health = Some(health);
        self
    }

    /// Attaches a host-provided obligation handler.
    pub fn with_obligation_handler<T>(mut self, handler: Arc<T>) -> Self
    where
        T: CapabilityObligationHandler + 'static,
    {
        let handler: Arc<dyn CapabilityObligationHandler> = handler;
        self.obligation_handler = Some(handler);
        self
    }

    /// Attaches an already-erased host-provided obligation handler.
    pub fn with_obligation_handler_dyn(
        mut self,
        handler: Arc<dyn CapabilityObligationHandler>,
    ) -> Self {
        self.obligation_handler = Some(handler);
        self
    }

    /// Installs a fully configured built-in obligation handler using the shared
    /// service graph supplied by host-runtime composition.
    ///
    /// The `services` value owns the handoff stores that runtime adapters and
    /// HTTP egress wiring will consume, while the installed handler receives
    /// clones of the same stores for staging obligations before dispatch.
    pub fn with_builtin_obligation_services(self, services: &BuiltinObligationServices) -> Self {
        self.with_obligation_handler(Arc::new(services.obligation_handler()))
    }

    /// Installs the default built-in obligation handler with no optional backing
    /// stores. Obligations requiring audit/network/secret/resource backing still
    /// fail closed until the caller supplies a fully configured handler through
    /// [`Self::with_builtin_obligation_services`], [`Self::with_obligation_handler`],
    /// or [`Self::with_obligation_handler_dyn`].
    pub fn with_builtin_obligation_handler(self) -> Self {
        self.with_obligation_handler(Arc::new(BuiltinObligationHandler::new()))
    }

    /// Attaches the secret store used for credential pre-flight checks.
    ///
    /// When set, `invoke_capability` and `spawn_capability` query secret presence
    /// for all required credentials declared in the capability manifest *before*
    /// the approval gate fires. This prevents burning a human approval on an
    /// invocation that cannot yet succeed because a credential is missing.
    ///
    /// The dispatch-time obligation check remains the enforcement backstop
    /// regardless of whether this store is set.
    ///
    /// Production code must use `HostRuntimeServices::build_host_runtime()` which
    /// wires the secret store automatically. This setter is `pub(crate)` to prevent
    /// a second public seam for secret-store configuration on the production facade.
    // arch-exempt: optional_arc, genuinely optional — minimal/test graphs that
    // never need pre-flight skip this; production wires it from HostRuntimeServices,
    // plan #4539 (Fix B)
    pub(crate) fn with_credential_preflight_store(
        mut self,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        self.credential_preflight_store = Some(secret_store);
        self
    }

    /// Spawns an already-authorized process request through the configured
    /// process manager.
    pub async fn spawn_process(
        &self,
        start: ProcessStart,
    ) -> Result<crate::RuntimeProcessHandle, HostRuntimeError> {
        let Some(process_manager) = &self.process_manager else {
            return Err(HostRuntimeError::Unavailable {
                reason: "process manager unavailable".to_string(),
            });
        };
        let capability_id = start.capability_id.clone();
        let record = process_manager
            .spawn(start)
            .await
            .map_err(unavailable_from_process_error)?;
        Ok(crate::RuntimeProcessHandle {
            process_id: record.process_id,
            capability_id,
        })
    }
}

#[async_trait]
impl HostRuntime for DefaultHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, capability_id, estimate, input) = request;
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;
        let total_started_at = live_latency_started_at();

        let registry = self.registry.snapshot();

        // Validate the execution context before the kernel's credential pre-flight
        // queries the secret store. Without this guard a malformed
        // HostRuntime::invoke_capability could probe secret-store presence under a forged
        // resource_scope that does not match the top-level
        // tenant/user/agent/project fields. Trust classification and runtime-policy
        // planning now run inside the kernel's `authorize()` fold — no host_runtime
        // pre-authorization stamps `context.trust` before it.
        if let Err(error) = context.validate() {
            return Err(HostRuntimeError::invalid_request(error.to_string()));
        }

        // Credential pre-flight and the persistent-approval re-authorize fold now
        // run inside the capability kernel's `authorize()` fold (§5.2.7/§5.3.2),
        // reading `HostPolicyFacts` (impl'd by this runtime). A missing credential
        // surfaces as `CapabilityInvocationError::AuthorizationRequiresAuth`, which
        // `translate_invocation_error` maps back to `auth_required_outcome` (same
        // gate id, same fields). The kernel orders credential-before-approval and
        // adopts the first persistent grant that flips the decision to Allow.

        let host = self.capability_host(&registry);

        let dispatch_started_at = live_latency_started_at();
        match host
            .invoke_json(context, capability_id.clone(), estimate, input)
            .await
        {
            Ok(result) => {
                trace_capability_latency_ok(
                    "capability_host_invoke_json",
                    &capability_id,
                    &scope,
                    dispatch_started_at,
                );
                trace_capability_latency_ok(
                    "invoke_capability",
                    &capability_id,
                    &scope,
                    total_started_at,
                );
                Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                    completed_outcome_from(result, capability_id),
                )))
            }
            Err(error) => {
                trace_capability_latency_error(
                    "capability_host_invoke_json",
                    &capability_id,
                    &scope,
                    dispatch_started_at,
                    &error,
                );
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    "capability invocation failed"
                );
                let translated = self
                    .translate_invocation_error(
                        error,
                        capability_id.clone(),
                        scope.clone(),
                        invocation_id,
                    )
                    .await;
                match &translated {
                    Ok(_) => trace_capability_latency_ok(
                        "invoke_capability",
                        &capability_id,
                        &scope,
                        total_started_at,
                    ),
                    Err(error) => trace_capability_latency_error(
                        "invoke_capability",
                        &capability_id,
                        &scope,
                        total_started_at,
                        error,
                    ),
                }
                translated
            }
        }
    }

    async fn spawn_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, capability_id, estimate, input) = request;
        let input = match host_runtime_spawn_input_for_capability(&capability_id, input)? {
            SpawnInputPreparation::Ready(input) => input,
            SpawnInputPreparation::ModelInputRejected(failure) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    "process sandbox spawn rejected malformed model plan as recoverable tool error"
                );
                return Ok(RuntimeCapabilityOutcome::Failed(failure));
            }
        };
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;

        let registry = self.registry.snapshot();

        // Validate the execution context before the kernel's credential pre-flight
        // queries the secret store. Without this guard a malformed
        // HostRuntime::spawn_capability could probe secret-store presence under a forged
        // resource_scope that does not match the top-level
        // tenant/user/agent/project fields. Trust classification and runtime-policy
        // planning now run inside the kernel's spawn authorize fold — no
        // host_runtime pre-authorization stamps `context.trust` before it.
        if let Err(error) = context.validate() {
            return Err(HostRuntimeError::invalid_request(error.to_string()));
        }

        // Credential pre-flight and the persistent-approval re-authorize fold now
        // run inside the kernel's spawn authorize fold (§5.2.7/§5.3.2) via
        // `HostPolicyFacts`: a missing credential surfaces as
        // `AuthorizationRequiresAuth` before the spawn-approval decision, and the
        // first persistent grant that flips the decision to Allow is adopted.

        let host = self.capability_host(&registry);
        let spawn = CapabilitySpawnRequest {
            context,
            capability_id: capability_id.clone(),
            estimate,
            input,
        };

        match host.spawn_json(spawn).await {
            Ok(result) => Ok(RuntimeCapabilityOutcome::SpawnedProcess(
                spawned_process_outcome_from(result, capability_id),
            )),
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    "capability spawn failed"
                );
                self.translate_invocation_error(error, capability_id, scope, invocation_id)
                    .await
            }
        }
    }

    async fn resume_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, approval_request_id, capability_id, estimate, input) = request;
        if let Some(outcome) = self
            .resume_actor_preflight_guard(&context, &capability_id)
            .await?
        {
            return Ok(outcome);
        }

        // Trust classification runs inside the kernel's `authorize_resumed` fold,
        // which fails the blocked run on a trust rejection (replacing the former
        // host_runtime pre-authorization + `context.trust` stamp).
        let registry = self.registry.snapshot();
        let host = self.capability_host(&registry);
        match host
            .resume_json(
                context,
                approval_request_id,
                capability_id.clone(),
                estimate,
                input,
            )
            .await
        {
            Ok(result) => Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                completed_outcome_from(result, capability_id),
            ))),
            // Resume must not start a second approval loop: if the lower layer ever returns
            // AuthorizationRequiresApproval here, surface it as a failed resume instead of
            // translating it back into RuntimeCapabilityOutcome::ApprovalRequired.
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    "capability resume failed"
                );
                match error {
                    CapabilityInvocationError::AuthorizationRequiresAuth {
                        capability,
                        required_secrets,
                        credential_requirements,
                    } => Ok(auth_required_outcome(
                        capability,
                        required_secrets,
                        credential_requirements,
                    )),
                    other => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                        other,
                        capability_id,
                    ))),
                }
            }
        }
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeAuthResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, capability_id, estimate, input, approval_request_id) = request;
        if let Some(outcome) = self
            .resume_actor_preflight_guard(&context, &capability_id)
            .await?
        {
            return Ok(outcome);
        }

        // Trust classification and the persistent-approval re-application on
        // auth-resume now live in the kernel's `authorize_resumed` fold
        // (§5.2.7/§5.3.2): a capability authorized only by a persistent grant
        // (e.g. `extension_install` under admin-config FirstParty trust) is
        // re-authorized by the kernel injecting the candidate grant after the
        // credential gate, and a trust rejection fails the blocked run there —
        // replacing the former host_runtime pre-authorization + `context.trust`
        // stamp.
        let registry = self.registry.snapshot();
        let host = self.capability_host(&registry);
        match host
            .auth_resume_json(
                context,
                capability_id.clone(),
                estimate,
                input,
                approval_request_id,
            )
            .await
        {
            Ok(result) => Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                completed_outcome_from(result, capability_id),
            ))),
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    "capability auth-resume failed"
                );
                match error {
                    CapabilityInvocationError::AuthorizationRequiresAuth {
                        capability,
                        required_secrets,
                        credential_requirements,
                    } => Ok(auth_required_outcome(
                        capability,
                        required_secrets,
                        credential_requirements,
                    )),
                    other => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                        other,
                        capability_id,
                    ))),
                }
            }
        }
    }

    async fn decline_auth_capability(
        &self,
        request: crate::RuntimeAuthDecline,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, capability_id) = request;
        let registry = self.registry.snapshot();
        let host = self.capability_host(&registry);
        match host.decline_auth_json(context, capability_id.clone()).await {
            Ok(()) => Ok(RuntimeCapabilityOutcome::Failed(
                RuntimeCapabilityFailure::new(
                    capability_id,
                    RuntimeFailureKind::GateDeclined,
                    Some("auth gate denied by user".to_string()),
                ),
            )),
            Err(CapabilityInvocationError::RunState(error)) => {
                Err(unavailable_from_run_state(*error))
            }
            Err(CapabilityInvocationError::ResumeStoreMissing { .. }) => {
                Err(HostRuntimeError::unavailable("run-state store unavailable"))
            }
            Err(error) => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                error,
                capability_id,
            ))),
        }
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let (context, approval_request_id, capability_id, estimate, input) = request;
        if let Some(outcome) = self
            .resume_actor_preflight_guard(&context, &capability_id)
            .await?
        {
            return Ok(outcome);
        }
        let input = match host_runtime_spawn_input_for_capability(&capability_id, input)? {
            SpawnInputPreparation::Ready(input) => input,
            SpawnInputPreparation::ModelInputRejected(failure) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    "process sandbox spawn resume rejected malformed model plan as recoverable tool error"
                );
                return Ok(RuntimeCapabilityOutcome::Failed(failure));
            }
        };

        // Runtime-policy planning and trust classification run inside the kernel's
        // `resume_spawn_json` fold, which fails the blocked run on rejection —
        // replacing the former host_runtime pre-authorization + `context.trust`
        // stamp.
        let registry = self.registry.snapshot();
        let host = self.capability_host(&registry);
        match host
            .resume_spawn_json(
                context,
                approval_request_id,
                capability_id.clone(),
                estimate,
                input,
            )
            .await
        {
            Ok(result) => Ok(RuntimeCapabilityOutcome::SpawnedProcess(
                spawned_process_outcome_from(result, capability_id),
            )),
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    "capability spawn resume failed"
                );
                // Mirror resume_capability: AuthorizationRequiresAuth must return
                // AuthRequired, not Failed. Without this arm a spawned capability
                // that needs re-auth after an approval resume silently fails.
                match error {
                    CapabilityInvocationError::AuthorizationRequiresAuth {
                        capability,
                        required_secrets,
                        credential_requirements,
                    } => Ok(auth_required_outcome(
                        capability,
                        required_secrets,
                        credential_requirements,
                    )),
                    other => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                        other,
                        capability_id,
                    ))),
                }
            }
        }
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        let registry = self.registry.snapshot();
        let catalog = CapabilityCatalog::new(
            &registry,
            self.authorizer.as_ref(),
            &self.surface_version,
            &self.runtime_policy,
        );
        let catalog = match self.surface_filesystem.as_deref() {
            Some(filesystem) => catalog.with_filesystem(filesystem),
            None => catalog,
        };
        catalog.visible_capabilities(request).await
    }

    /// Best-effort cancellation fanout for active work in one scope.
    ///
    /// Background processes can be terminalized through the process store and
    /// cooperative cancellation registry. Inline capability invocations do not
    /// yet expose a cancellation token through [`CapabilityHost`], so active
    /// invocation records are returned as `unsupported` instead of silently
    /// disappearing behind an empty outcome.
    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        tracing::debug!(
            correlation_id = %request.correlation_id,
            reason = ?request.reason,
            "host runtime cancellation requested"
        );

        let mut outcome = CancelRuntimeWorkOutcome::default();
        let mut process_invocations = Vec::new();

        if let Some(process_store) = &self.process_store {
            let records = process_store
                .records_for_scope(&request.scope)
                .await
                .map_err(unavailable_from_process_error)?;
            let mut process_host = ProcessHost::new(process_store.as_ref());
            if let Some(registry) = &self.process_cancellation_registry {
                process_host = process_host.with_cancellation_registry(Arc::clone(registry));
            }
            if let Some(result_store) = &self.process_result_store {
                process_host = process_host.with_result_store_dyn(Arc::clone(result_store));
            }

            for record in records {
                if record.status != ProcessStatus::Running {
                    continue;
                }
                process_invocations.push(record.invocation_id);
                let work_id = RuntimeWorkId::Process(record.process_id);
                match process_host.kill(&request.scope, record.process_id).await {
                    Ok(_) => {
                        outcome.cancelled.push(work_id);
                    }
                    Err(ProcessError::InvalidTransition { .. }) => {
                        outcome.already_terminal.push(work_id);
                    }
                    Err(error) => return Err(unavailable_from_process_error(error)),
                }
            }
        }

        if let Some(run_state) = &self.run_state {
            let records = run_state
                .records_for_scope(&request.scope)
                .await
                .map_err(unavailable_from_run_state)?;
            outcome.unsupported.extend(
                records
                    .into_iter()
                    .filter(|record| record.status == RunStatus::Running)
                    .filter(|record| !process_invocations.contains(&record.invocation_id))
                    .map(|record| RuntimeWorkId::Invocation(record.invocation_id)),
            );
        }

        Ok(outcome)
    }

    /// Snapshot of active host runtime work for one scope.
    ///
    /// `correlation_id` is carried for tracing/audit only — at this layer we
    /// surface every running invocation in scope rather than narrowing to the
    /// caller's correlation. Upper turn/loop services that need per-correlation
    /// fan-in are expected to filter the returned summaries themselves.
    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        let mut active_work = Vec::new();
        let registry = self.registry.snapshot();

        if let Some(run_state) = &self.run_state {
            let records = run_state
                .records_for_scope(&request.scope)
                .await
                .map_err(unavailable_from_run_state)?;

            active_work.extend(
                records
                    .into_iter()
                    .filter(|record| record.status == RunStatus::Running)
                    .map(|record| {
                        let runtime = registry
                            .get_capability(&record.capability_id)
                            .map(|descriptor| descriptor.runtime);
                        RuntimeWorkSummary {
                            work_id: RuntimeWorkId::Invocation(record.invocation_id),
                            capability_id: Some(record.capability_id),
                            runtime,
                        }
                    }),
            );
        }

        if let Some(process_store) = &self.process_store {
            let records = process_store
                .records_for_scope(&request.scope)
                .await
                .map_err(unavailable_from_process_error)?;
            let mut process_invocations = Vec::new();
            active_work.extend(
                records
                    .into_iter()
                    .filter(|record| record.status == ProcessStatus::Running)
                    .map(|record| {
                        process_invocations.push(record.invocation_id);
                        RuntimeWorkSummary {
                            work_id: RuntimeWorkId::Process(record.process_id),
                            capability_id: Some(record.capability_id),
                            runtime: Some(record.runtime),
                        }
                    }),
            );
            if !process_invocations.is_empty() {
                active_work.retain(|summary| match &summary.work_id {
                    RuntimeWorkId::Invocation(invocation_id) => {
                        !process_invocations.contains(invocation_id)
                    }
                    RuntimeWorkId::Process(_) | RuntimeWorkId::Gate(_) => true,
                });
            }
        }

        Ok(HostRuntimeStatus { active_work })
    }

    /// Returns readiness for runtime backends required by registered capabilities.
    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        let registry = self.registry.snapshot();
        let required = required_runtime_backends(&registry);
        if required.is_empty() {
            return Ok(HostRuntimeHealth {
                ready: true,
                missing_runtime_backends: Vec::new(),
            });
        }

        let missing_runtime_backends = if let Some(health) = &self.runtime_health {
            let reported = health.missing_runtime_backends(&required).await?;
            normalize_missing_runtime_backends(&required, reported)
        } else {
            required
        };
        Ok(HostRuntimeHealth {
            ready: missing_runtime_backends.is_empty(),
            missing_runtime_backends,
        })
    }
}

impl DefaultHostRuntime {
    fn capability_host<'a>(
        &'a self,
        registry: &'a ExtensionRegistry,
    ) -> CapabilityHost<'a, dyn CapabilityDispatcher> {
        let mut host = CapabilityHost::new(
            registry,
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
            self.trust_policy.as_ref(),
            &self.runtime_policy,
            // `DefaultHostRuntime` supplies the host-mediated policy facts the
            // kernel's `authorize()` fold reads (credential pre-flight); `self`
            // coerces to `&dyn HostPolicyFacts`.
            self,
        );
        if let Some(run_state_approval_store) = &self.run_state_approval_store {
            host = host.with_run_state_approval_store(run_state_approval_store.as_ref());
        } else {
            if let Some(run_state) = &self.run_state {
                host = host.with_run_state(run_state.as_ref());
            }
            if let Some(approval_requests) = &self.approval_requests {
                host = host.with_approval_requests(approval_requests.as_ref());
            }
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(process_manager) = &self.process_manager {
            host = host.with_process_manager(process_manager.as_ref());
        }
        if let Some(obligation_handler) = &self.obligation_handler {
            host = host.with_obligation_handler(obligation_handler.as_ref());
        }
        host
    }

    /// Rejects a resume whose sealed ingress actor differs from the actor that
    /// started the run. Callers invoke this before any preflight that can fail
    /// or mutate the blocked run; `CapabilityHost` repeats the check before
    /// claiming leases or dispatching.
    async fn resume_actor_preflight_guard(
        &self,
        context: &ironclaw_host_api::ExecutionContext,
        capability_id: &CapabilityId,
    ) -> Result<Option<RuntimeCapabilityOutcome>, HostRuntimeError> {
        context
            .validate()
            .map_err(|error| HostRuntimeError::invalid_request(error.to_string()))?;
        let Some(run_state) = self.run_state.as_ref() else {
            return Ok(None);
        };
        let Some(record) = run_state
            .get(&context.resource_scope, context.invocation_id)
            .await
            .map_err(unavailable_from_run_state)?
        else {
            return Ok(None);
        };
        if record.authenticated_actor_user_id == context.authenticated_actor_user_id {
            return Ok(None);
        }

        let error = CapabilityInvocationError::AuthorizationDenied {
            capability: capability_id.clone(),
            reason: DenyReason::PolicyDenied,
            detail: None,
        };
        Ok(Some(RuntimeCapabilityOutcome::Failed(failure_from(
            error,
            capability_id.clone(),
        ))))
    }

    async fn translate_invocation_error(
        &self,
        error: CapabilityInvocationError,
        capability_id: CapabilityId,
        scope: ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        match error {
            CapabilityInvocationError::AuthorizationRequiresApproval { capability } => {
                match self.lookup_approval_request_id(&scope, invocation_id).await {
                    Ok(Some(approval_request_id)) => Ok(
                        RuntimeCapabilityOutcome::ApprovalRequired(RuntimeApprovalGate {
                            approval_request_id,
                            capability_id: capability,
                            reason: RuntimeBlockedReason::ApprovalRequired,
                        }),
                    ),
                    Ok(None) => Ok(RuntimeCapabilityOutcome::Failed(
                        RuntimeCapabilityFailure::new(
                            capability,
                            RuntimeFailureKind::Authorization,
                            Some(
                                "approval required but no approval request was persisted"
                                    .to_string(),
                            ),
                        ),
                    )),
                    Err(host_error) => {
                        // Surface persistence outages as Unavailable rather than
                        // pretending the approval was never persisted; otherwise a
                        // transient run-state failure looks indistinguishable from
                        // the (separately bug-prone) cap-host-skipped-persist path.
                        tracing::warn!(
                            capability_id = %capability,
                            error = %host_error,
                            "approval request lookup failed; surfacing as host runtime unavailability"
                        );
                        Err(host_error)
                    }
                }
            }
            CapabilityInvocationError::AuthorizationRequiresAuth {
                capability,
                required_secrets,
                credential_requirements,
            } => Ok(auth_required_outcome(
                capability,
                required_secrets,
                credential_requirements,
            )),
            other => {
                let should_fail_dispatch_run =
                    matches!(other, CapabilityInvocationError::Dispatch { .. });
                let failure = failure_from(other, capability_id);
                if should_fail_dispatch_run {
                    self.fail_dispatch_run(&failure, &scope, invocation_id)
                        .await;
                }
                Ok(RuntimeCapabilityOutcome::Failed(failure))
            }
        }
    }

    async fn fail_dispatch_run(
        &self,
        failure: &RuntimeCapabilityFailure,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) {
        let Some(run_state) = self.run_state.as_ref() else {
            return;
        };
        if let Err(error) = run_state
            .fail(scope, invocation_id, "Dispatch".to_string())
            .await
        {
            tracing::warn!(
                invocation_id = %invocation_id,
                capability_id = %failure.capability_id,
                failure_kind = failure.kind.as_str(),
                transition_error = %unavailable_from_run_state(error),
                "terminal dispatch failure could not transition run state; failure is returned to caller",
            );
        }
    }

    async fn lookup_approval_request_id(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ApprovalRequestId>, HostRuntimeError> {
        let Some(run_state) = self.run_state.as_ref() else {
            return Ok(None);
        };
        let record = run_state
            .get(scope, invocation_id)
            .await
            .map_err(unavailable_from_run_state)?;
        Ok(record.and_then(|record| record.approval_request_id))
    }
}

/// `DefaultHostRuntime` is the sole production implementor of the kernel's
/// [`ironclaw_capabilities::HostPolicyFacts`] port (§5.3.2/§9). It surfaces
/// host-mediated policy *facts* — never a verdict — that the capability kernel's
/// `authorize()` fold maps into the sealed authorization result:
///
/// - [`credential_presence`](DefaultHostRuntime::credential_presence) is the
///   relocation of the former `credential_preflight_check`; and
/// - [`persistent_grants`](DefaultHostRuntime::persistent_grants) surfaces the
///   active persistent-approval grants (via the same scope × grantee fan-out the
///   former `apply_persistent_approval_policy` used). The kernel's `authorize()`
///   fold owns the re-authorize loop that reads it and adopts the first grant
///   that flips the decision to `Allow`.
#[async_trait]
impl ironclaw_capabilities::HostPolicyFacts for DefaultHostRuntime {
    async fn credential_presence(
        &self,
        capability_id: &CapabilityId,
        scope: &ResourceScope,
    ) -> ironclaw_capabilities::CredentialPresence {
        use ironclaw_capabilities::CredentialPresence;

        // No store wired ⇒ pre-flight disabled (as before): treat as satisfied so
        // the kernel proceeds and the dispatch-time obligation check enforces.
        let Some(secret_store) = self.credential_preflight_store.as_ref() else {
            return CredentialPresence::Satisfied;
        };
        // The kernel already validated the descriptor exists; if this fresh
        // snapshot cannot see it there is nothing to pre-flight — satisfied.
        let registry = self.registry.snapshot();
        let Some(descriptor) = registry.get_capability(capability_id) else {
            return CredentialPresence::Satisfied;
        };
        let (required_secrets, requirements) = capability_credential_requirements(descriptor);
        if required_secrets.is_empty() {
            return CredentialPresence::Satisfied;
        }

        for handle in &required_secrets {
            // `secret_owner_scope` is the single owner of the presence+ownership
            // rule, shared with the dispatch-time obligation backstop so the two
            // paths cannot drift on "what counts as a present credential". Here we
            // need presence only (Some vs None) for gate ordering.
            match secret_owner_scope(secret_store.as_ref(), scope, handle).await {
                Ok(Some(_)) => {
                    // Present — keep checking the remaining handles.
                }
                Ok(None) => {
                    tracing::debug!(
                        capability_id = %capability_id,
                        secret_handle = handle.as_str(),
                        "credential pre-flight (kernel): required secret absent; surfacing AuthRequired before approval gate"
                    );
                    return CredentialPresence::Missing {
                        required_secrets,
                        requirements,
                    };
                }
                Err(error) => {
                    // Fail-open: a transient store fault must not masquerade as a
                    // missing credential and burn a user auth interaction. The
                    // kernel maps `Indeterminate` to "skip the pre-flight"; the
                    // dispatch-time obligation check is the enforcing backstop.
                    tracing::debug!(
                        capability_id = %capability_id,
                        secret_handle = handle.as_str(),
                        error = %error,
                        "credential pre-flight (kernel): secret store metadata query failed; treating as indeterminate (dispatch-time check enforces)"
                    );
                    return CredentialPresence::Indeterminate;
                }
            }
        }

        CredentialPresence::Satisfied
    }

    async fn persistent_grants(
        &self,
        capability_id: &CapabilityId,
        context: &ironclaw_host_api::ExecutionContext,
        action: ironclaw_capabilities::PolicyAction,
    ) -> Vec<ironclaw_host_api::CapabilityGrant> {
        let Some(policies) = self.persistent_approval_policies.as_ref() else {
            return Vec::new();
        };
        let action = match action {
            ironclaw_capabilities::PolicyAction::Dispatch => PersistentApprovalAction::Dispatch,
            ironclaw_capabilities::PolicyAction::SpawnCapability => {
                PersistentApprovalAction::SpawnCapability
            }
        };
        // The kernel passes the full `ExecutionContext`, so the grantee fan-out is
        // derived through the SAME helpers the former
        // `apply_persistent_approval_policy` used — including the
        // `Principal::Extension` grantee read from `context.extension_id`, which a
        // bare `ResourceScope` cannot carry. This recovers extension-grantee
        // persistent approvals that a scope-only lookup would silently drop.
        let scopes = persistent_approval_lookup_scopes(&context.resource_scope);
        let grantees = persistent_approval_grantees(context);
        let mut grants = Vec::new();
        for policy_scope in &scopes {
            for grantee in &grantees {
                let key = PersistentApprovalPolicyKey {
                    scope: policy_scope.clone(),
                    action,
                    capability_id: capability_id.clone(),
                    grantee: grantee.clone(),
                };
                match policies.lookup(&key).await {
                    Ok(Some(policy)) => {
                        if let Some(grant) = policy.active_grant() {
                            grants.push(grant);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        // A lookup fault yields no synthesized grant; skip the
                        // entry and fall back to normal authorization.
                        tracing::warn!(
                            capability_id = %capability_id,
                            error = %error,
                            "persistent approval policy lookup failed; skipping grant"
                        );
                    }
                }
            }
        }
        grants
    }
}

/// Maps a [`RunStateError`] to a sanitized [`HostRuntimeError::Unavailable`].
///
/// `RunStateError::InvalidPath` and `Filesystem` carry raw filesystem
/// strings; `Serialization`/`Deserialization` carry serde internals. Forward
/// the redacted variant discriminator instead of `error.to_string()` so the
/// boundary stays infrastructure-opaque to upper services.
// arch-exempt: large_file, host runtime production wiring; +1 arm for RunStateError::GateRecordAlreadyExists (#6243 left this match non-exhaustive), plan #6175
fn unavailable_from_run_state(error: RunStateError) -> HostRuntimeError {
    let reason = match error {
        RunStateError::UnknownInvocation { .. } => "run-state record not found",
        RunStateError::InvocationAlreadyExists { .. } => "run-state record already exists",
        RunStateError::UnknownApprovalRequest { .. } => "approval request not found",
        RunStateError::ApprovalRequestAlreadyExists { .. } => "approval request already exists",
        RunStateError::GateRecordAlreadyExists { .. } => "gate record already exists",
        RunStateError::ApprovalNotPending { .. } => "approval request not pending",
        RunStateError::InvalidPath(_) => "run-state storage path invalid",
        RunStateError::Filesystem(_) => "run-state filesystem unavailable",
        RunStateError::Serialization(_) => "run-state serialization failed",
        RunStateError::Deserialization(_) => "run-state deserialization failed",
        RunStateError::Backend(_) => "run-state backend unavailable",
    };
    HostRuntimeError::unavailable(reason)
}

/// Maps a [`ProcessError`] to a sanitized [`HostRuntimeError::Unavailable`].
fn unavailable_from_process_error(error: ProcessError) -> HostRuntimeError {
    let reason = match error {
        ProcessError::UnknownProcess { .. } => "process record not found",
        ProcessError::ProcessAlreadyExists { .. } => "process record already exists",
        ProcessError::InvalidTransition { .. } => "process lifecycle transition invalid",
        ProcessError::ResourceReservationMismatch { .. } => "process resource reservation mismatch",
        ProcessError::ResourceReservationAlreadyAssigned { .. } => {
            "process resource reservation already assigned"
        }
        ProcessError::ResourceReservationNotOwned { .. } => {
            "process resource reservation not owned"
        }
        ProcessError::Resource(_) => "process resource lifecycle failed",
        ProcessError::ResourceCleanupFailed { .. } => "process resource cleanup failed",
        ProcessError::ProcessResultStoreUnavailable => "process result store unavailable",
        ProcessError::ProcessResultUnavailable { .. } => "process result unavailable",
        ProcessError::InvalidStoredRecord { .. } => "process stored record invalid",
        ProcessError::InvalidPath(_) => "process storage path invalid",
        ProcessError::Filesystem(_) => "process filesystem unavailable",
        ProcessError::Serialization(_) => "process serialization failed",
        ProcessError::Deserialization(_) => "process deserialization failed",
    };
    HostRuntimeError::unavailable(reason)
}

fn required_runtime_backends(registry: &ExtensionRegistry) -> Vec<RuntimeKind> {
    let mut required = Vec::new();
    for descriptor in registry.capabilities() {
        if !required.contains(&descriptor.runtime) {
            required.push(descriptor.runtime);
        }
    }
    required.sort_by_key(|runtime| runtime_kind_rank(*runtime));
    required
}

fn normalize_missing_runtime_backends(
    required: &[RuntimeKind],
    reported: Vec<RuntimeKind>,
) -> Vec<RuntimeKind> {
    let mut missing = Vec::new();
    for runtime in reported {
        if required.contains(&runtime) && !missing.contains(&runtime) {
            missing.push(runtime);
        }
    }
    missing.sort_by_key(|runtime| runtime_kind_rank(*runtime));
    missing
}

fn runtime_kind_rank(runtime: RuntimeKind) -> u8 {
    match runtime {
        RuntimeKind::Wasm => 0,
        RuntimeKind::Mcp => 1,
        RuntimeKind::Script => 2,
        RuntimeKind::FirstParty => 3,
        RuntimeKind::System => 4,
    }
}

fn completed_outcome_from(
    result: CapabilityInvocationResult,
    capability_id: CapabilityId,
) -> RuntimeCapabilityCompleted {
    RuntimeCapabilityCompleted {
        capability_id,
        output: result.dispatch.output,
        display_preview: result.dispatch.display_preview,
        usage: result.dispatch.usage,
    }
}

/// Returns the required secrets and OAuth credential requirements declared in
/// the capability descriptor.
///
/// This is the canonical extraction used by the **pre-flight credential
/// presence check** (before the approval gate). The dispatch-time obligation
/// check remains the enforcement backstop; it derives the same handles through
/// the obligation-handler iteration over `descriptor.runtime_credentials`
/// (same source, different code path — both iterate `required == true` entries).
/// The two paths agree on which handles are required; the pre-flight additionally
/// computes `credential_requirements` for the auth-gate payload.
///
/// Callers outside the pre-flight check must not recompute the requirement set
/// independently — call this function instead.
///
/// Only entries with `required == true` **and** `source == SecretHandle` are
/// included in `required_secrets`. `ProductAuthAccount`-source credentials are
/// staged by the credential-account resolver at dispatch time (not via
/// `secret_store.metadata`), so including their slot handle here would produce
/// a false-positive `AuthRequired` for capabilities whose product-auth account
/// is already connected.
pub(crate) fn capability_credential_requirements(
    descriptor: &ironclaw_host_api::CapabilityDescriptor,
) -> (
    Vec<SecretHandle>,
    Vec<ironclaw_host_api::RuntimeCredentialAuthRequirement>,
) {
    let provider = descriptor.provider.clone();
    let mut required_secrets = Vec::new();
    let mut credential_requirements = Vec::new();

    // Double-read accepted: the dispatch-time obligation path (in
    // ironclaw_host_runtime::obligations) will re-check each handle's presence via
    // the same secret_store when the capability executes. Threading the pre-flight
    // result into the obligation path would cross crate-boundary constraints (per
    // CLAUDE.md) without meaningful gain; the ordering guarantee (auth before
    // approval gate) is the pre-flight's sole purpose.
    for cred in &descriptor.runtime_credentials {
        if !cred.required {
            continue;
        }
        // Only SecretHandle-source credentials are presence-checkable in the
        // secret store. ProductAuthAccount credentials are staged by the
        // credential-account resolver at dispatch time (not via secret_store.metadata),
        // so including their slot handle here would produce a false-positive AuthRequired
        // for capabilities whose product-auth account is already connected.
        if matches!(
            cred.source,
            ironclaw_host_api::RuntimeCredentialRequirementSource::SecretHandle
        ) {
            required_secrets.push(cred.handle.clone());
        }
        if let Some(auth_req) = cred.product_auth_requirement_for(provider.clone()) {
            credential_requirements.push(auth_req);
        }
    }
    (required_secrets, credential_requirements)
}

fn auth_required_outcome(
    capability_id: CapabilityId,
    required_secrets: Vec<SecretHandle>,
    credential_requirements: Vec<ironclaw_host_api::RuntimeCredentialAuthRequirement>,
) -> RuntimeCapabilityOutcome {
    RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
        gate_id: stable_auth_gate_id(&capability_id, &required_secrets, &credential_requirements),
        capability_id,
        reason: RuntimeBlockedReason::AuthRequired,
        required_secrets,
        credential_requirements,
    })
}

fn stable_auth_gate_id(
    capability_id: &CapabilityId,
    required_secrets: &[SecretHandle],
    credential_requirements: &[RuntimeCredentialAuthRequirement],
) -> RuntimeGateId {
    let mut parts = Vec::new();
    parts.push(format!("capability={}", capability_id.as_str()));

    let mut secret_handles = required_secrets
        .iter()
        .map(|handle| handle.as_str().to_string())
        .collect::<Vec<_>>();
    secret_handles.sort();
    for handle in secret_handles {
        parts.push(format!("secret={handle}"));
    }

    let mut requirements = credential_requirements
        .iter()
        .map(|requirement| {
            // `setup` MUST be part of the fingerprint (#6299 IronLoop): two
            // requirements that agree on provider/extension/provider_scopes but
            // differ in `setup` (e.g. a ManualToken record vs a later OAuth or
            // Pairing record, or differing OAuth setup scopes) are DIFFERENT auth
            // requirements. Omitting it lets them derive the same deterministic
            // `for_auth_gate` key; the write-once store then reports
            // `GateRecordAlreadyExists` and silently keeps the stale record, so
            // the runner reloads and renders the wrong authentication flow.
            format!(
                "credential={}:{}:setup={}:{}",
                requirement.provider.as_str(),
                requirement.requester_extension.as_str(),
                stable_setup_token(&requirement.setup),
                canonical_scope_list(&requirement.provider_scopes),
            )
        })
        .collect::<Vec<_>>();
    requirements.sort();
    parts.extend(requirements);

    let digest = sha256_digest_token(parts.join("\n").as_bytes());
    let suffix = digest.strip_prefix("sha256:").unwrap_or(&digest);
    RuntimeGateId::from_stable_suffix(&format!("auth-{suffix}"))
        .unwrap_or_else(|_| RuntimeGateId::new())
}

/// Canonical, deterministic fingerprint token for a credential-account setup,
/// so [`stable_auth_gate_id`] distinguishes auth requirements that differ only
/// in their setup flow (#6299 IronLoop). Exhaustive by design: a new
/// `RuntimeCredentialAccountSetup` variant fails the build here rather than
/// silently hashing to an existing token. OAuth setup scopes use the same
/// injective [`canonical_scope_list`] encoding as `provider_scopes`.
fn stable_setup_token(setup: &ironclaw_host_api::RuntimeCredentialAccountSetup) -> String {
    use ironclaw_host_api::RuntimeCredentialAccountSetup as Setup;
    match setup {
        Setup::ManualToken => "manual_token".to_string(),
        Setup::OAuth { scopes } => format!("oauth:{}", canonical_scope_list(scopes)),
        Setup::Pairing => "pairing".to_string(),
        Setup::Retired => "retired".to_string(),
    }
}

/// Injective canonical encoding of a scope list for the auth-gate fingerprint
/// (#6299 IronLoop). Scopes are not validated to exclude a join delimiter, so a
/// plain `join(",")` is ambiguous — `["a,b"]` and `["a", "b"]` would collide and
/// derive the same write-once gate key. Sort (a scope set is order-independent),
/// then length-prefix each element (`<byte_len>:<scope>`) so distinct sets can
/// never share an encoding regardless of which characters the scopes contain.
fn canonical_scope_list(scopes: &[String]) -> String {
    let mut sorted = scopes.to_vec();
    sorted.sort();
    sorted
        .iter()
        .map(|scope| format!("{}:{scope}", scope.len()))
        .collect::<Vec<_>>()
        .join("|")
}

fn spawned_process_outcome_from(
    result: CapabilitySpawnResult,
    capability_id: CapabilityId,
) -> crate::RuntimeProcessHandle {
    crate::RuntimeProcessHandle {
        process_id: result.process.process_id,
        capability_id,
    }
}

fn persistent_approval_grantees(context: &ironclaw_host_api::ExecutionContext) -> Vec<Principal> {
    let mut grantees = vec![
        Principal::Extension(context.extension_id.clone()),
        Principal::User(context.user_id.clone()),
    ];
    if let Some(agent_id) = &context.agent_id {
        grantees.push(Principal::Agent(agent_id.clone()));
    }
    if let Some(project_id) = &context.project_id {
        grantees.push(Principal::Project(project_id.clone()));
    }
    if let Some(mission_id) = &context.mission_id {
        grantees.push(Principal::Mission(mission_id.clone()));
    }
    // No `Principal::Thread` grantee: persistent approval policies are never
    // written under a thread grantee (the grantee always comes from
    // `ApprovalRequest.requested_by`, which is `Principal::User` or
    // `Principal::Extension`), so looking one up could never match. Persistent
    // approvals are deliberately thread-agnostic (see #4825).
    grantees
}

fn persistent_approval_lookup_scopes(scope: &ResourceScope) -> Vec<PersistentApprovalScope> {
    let user_scope = PersistentApprovalScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: None,
        project_id: None,
    };
    let legacy_scope = PersistentApprovalScope::from_resource_scope(scope);
    if legacy_scope == user_scope {
        vec![user_scope]
    } else {
        // User-scope settings-page policies intentionally win lookup order over
        // legacy agent/project-scoped prompt policies.
        vec![user_scope, legacy_scope]
    }
}

/// Outcome of preparing model-supplied spawn input for a capability.
///
/// A malformed or invalid process-sandbox plan is a *model-fixable* condition:
/// the model chose bad arguments and can correct them on a retry. It must
/// surface as a recoverable, model-visible tool error
/// ([`RuntimeFailureKind::InvalidInput`] → `ModelVisibleToolError`), never as a
/// terminal [`HostRuntimeError`] that ends the whole run. Genuine host-side
/// faults (serializing the validated host struct back to JSON) remain errors.
enum SpawnInputPreparation {
    /// Input is ready to dispatch to the capability host.
    Ready(serde_json::Value),
    /// Model supplied an unparseable/invalid plan — recoverable, model-visible.
    ModelInputRejected(RuntimeCapabilityFailure),
}

fn host_runtime_spawn_input_for_capability(
    capability_id: &CapabilityId,
    input: serde_json::Value,
) -> Result<SpawnInputPreparation, HostRuntimeError> {
    if capability_id.as_str() != PROCESS_SANDBOX_CAPABILITY_ID {
        return Ok(SpawnInputPreparation::Ready(input));
    }
    let plan = match serde_json::from_value::<SandboxProcessPlan>(input) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(SpawnInputPreparation::ModelInputRejected(
                RuntimeCapabilityFailure::new(
                    capability_id.clone(),
                    RuntimeFailureKind::InvalidInput,
                    Some(
                        "process sandbox capability input must be a SandboxProcessPlan".to_string(),
                    ),
                )
                // The parse cause ("missing field `run`", …) rides the
                // model-visible Diagnostic channel — scrubbed at the loop
                // seam — so the model can correct the plan shape on retry.
                .with_model_visible_cause(error.to_string()),
            ));
        }
    };
    let plan = match ValidatedSandboxProcessPlan::new(plan) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(SpawnInputPreparation::ModelInputRejected(
                RuntimeCapabilityFailure::new(
                    capability_id.clone(),
                    RuntimeFailureKind::InvalidInput,
                    Some(
                        "process sandbox capability input failed SandboxProcessPlan validation"
                            .to_string(),
                    ),
                )
                // `ProcessSandboxPlanError` names the offending field and rule
                // ("run command must not be empty"); carry it to the model.
                .with_model_visible_cause(error.to_string()),
            ));
        }
    };
    // Serializing the *validated host struct* back to JSON is a host-side
    // operation, not model input. A failure here is a genuine internal fault,
    // so it stays a terminal error rather than a model-visible tool error.
    let value = serde_json::to_value(plan.into_plan()).map_err(|_| {
        HostRuntimeError::invalid_request("validated process sandbox plan could not be serialized")
    })?;
    Ok(SpawnInputPreparation::Ready(value))
}

/// Shared default leak detector for the model-visible-cause belt. Building one
/// compiles the registry regex set + prefix matcher, so it is memoized rather
/// than rebuilt on every failure (retry storms would otherwise pay it per call).
fn model_visible_cause_scrubber() -> &'static ironclaw_safety::LeakDetector {
    static DETECTOR: std::sync::LazyLock<ironclaw_safety::LeakDetector> =
        std::sync::LazyLock::new(ironclaw_safety::LeakDetector::new);
    &DETECTOR
}

fn failure_from(
    error: CapabilityInvocationError,
    capability_id: CapabilityId,
) -> RuntimeCapabilityFailure {
    let kind = failure_kind_from(&error);
    let raw_cause = raw_failure_cause(&error);
    let message = sanitized_failure_message(&error);
    let detail = match error {
        CapabilityInvocationError::Dispatch {
            detail: Some(detail),
            ..
        } => Some(detail),
        CapabilityInvocationError::Dispatch {
            detail: None,
            safe_summary: Some(summary),
            ..
        } => rejected_summary_diagnostic(summary),
        _ => None,
    };
    let mut failure = RuntimeCapabilityFailure::new(capability_id, kind, message);
    if let Some(detail) = detail {
        failure = failure.with_detail(detail);
    }
    if let Some(raw_cause) = raw_cause {
        // Registry-scrubbed here (belt); the loop-support Diagnostic seam
        // re-scrubs and injection-fences fail-closed (suspenders). Never
        // rendered in Debug, run-state rows, or runtime events.
        let (scrubbed, _) = model_visible_cause_scrubber().redact_all_secrets(&raw_cause);
        failure = failure.with_model_visible_cause(scrubbed);
    }
    failure
}

/// The raw descriptive cause for the model-visible Diagnostic channel, before
/// any public-surface gating.
fn raw_failure_cause(error: &CapabilityInvocationError) -> Option<String> {
    use CapabilityInvocationError::Dispatch;
    match error {
        Dispatch { safe_summary, .. } => safe_summary.clone(),
        _ => None,
    }
}

/// Preserve a host-authored failure reason that the strict loop safe-summary
/// validator rejects (paths, payload delimiters, newlines).
///
/// [`dispatch_failure_message`] degrades such reasons to the fixed category
/// sentence, which is correct for the summary — but the reason itself is what
/// the model needs to repair its call (e.g. which path was out of scope), so
/// it must ride the model-visible diagnostic detail channel instead of being
/// dropped. Secret VALUES are scrubbed and disallowed control characters
/// normalized at the loop boundary before the model observes the text.
fn rejected_summary_diagnostic(
    summary: String,
) -> Option<ironclaw_host_api::DispatchFailureDetail> {
    if LoopSafeSummary::new(summary.clone()).is_ok() {
        // The reason survives into `message`; the loop layer derives the
        // model-visible diagnostic from it directly, so attaching it here
        // would only duplicate it.
        return None;
    }
    const MAX_DIAGNOSTIC_CHARS: usize = 512;
    let text = if summary.chars().count() <= MAX_DIAGNOSTIC_CHARS {
        summary
    } else {
        let mut text: String = summary.chars().take(MAX_DIAGNOSTIC_CHARS - 3).collect();
        text.push_str("...");
        text
    };
    Some(ironclaw_host_api::DispatchFailureDetail::Diagnostic { text })
}

/// Returns a stable, redacted summary message for a capability invocation
/// failure.
///
/// Variants that wrap inner errors (`Lease`, `RunState`, `Process`,
/// `InvocationFingerprint`) or that surface free-form storage/runtime
/// strings are mapped to fixed, infrastructure-opaque labels. Dispatch causes
/// remain raw at this host-internal layer so loop support can split them into
/// a strict fallback card summary and a secret-value-scrubbed Diagnostic.
fn sanitized_failure_message(error: &CapabilityInvocationError) -> Option<String> {
    use CapabilityInvocationError::*;
    match error {
        // Surface the planner's specific fail-closed reason (threaded on
        // `detail`) behind the collapsed `DenyReason` so the model-visible
        // message explains the denial instead of a bare `PolicyDenied`.
        AuthorizationDenied {
            detail: Some(detail),
            ..
        } => Some(format!("{error}: {detail}")),
        UnknownCapability { .. }
        | AuthorizationDenied { .. }
        | UnsupportedObligations { .. }
        | ObligationFailed { .. }
        | AuthorizationRequiresAuth { .. }
        | AuthorizationRequiresApproval { .. }
        | ApprovalRequestMismatch { .. }
        | ApprovalFingerprintMismatch { .. }
        | ApprovalNotApproved { .. }
        | ApprovalLeaseMissing { .. }
        | ApprovalStoreMissing { .. }
        | ResumeStoreMissing { .. }
        | ProcessManagerMissing { .. }
        | ResumeNotBlocked { .. }
        | ResumeContextMismatch { .. } => Some(error.to_string()),
        Dispatch {
            safe_summary, kind, ..
        } => Some(dispatch_failure_message(safe_summary.as_deref(), *kind)),
        InvocationFingerprint { .. } => Some("invocation fingerprint failed".to_string()),
        Lease(_) => Some("capability lease store unavailable".to_string()),
        RunState(_) => Some("run-state store unavailable".to_string()),
        Process(_) => Some("process manager unavailable".to_string()),
    }
}

fn dispatch_failure_message(
    safe_summary: Option<&str>,
    kind: ironclaw_host_api::DispatchFailureKind,
) -> String {
    // This message is the PUBLIC label: persisted into run-state rows and
    // published on the runtime event sink before any downstream validation
    // (reborn_e2e_gate_sanitizes_runtime_backend_failure_before_public_surfaces
    // pins the boundary). It fails closed: only summaries that pass the strict
    // loop-summary validation (host-authored sentences, sanitized guest error
    // codes) pass through; wild raw causes degrade to the kind's fixed
    // sentence. The full descriptive cause is NOT lost — it rides the private
    // `model_visible_cause` channel to the model-visible Diagnostic seam.
    safe_summary
        .and_then(|summary| {
            ironclaw_turns::run_profile::LoopSafeSummary::new(summary.to_string()).ok()
        })
        .map(|summary| summary.as_str().to_string())
        .unwrap_or_else(|| kind.human_summary().to_string())
}

pub(crate) fn failure_kind_from(error: &CapabilityInvocationError) -> RuntimeFailureKind {
    match error {
        CapabilityInvocationError::UnknownCapability { .. } => RuntimeFailureKind::MissingRuntime,
        CapabilityInvocationError::AuthorizationRequiresAuth { .. } => {
            RuntimeFailureKind::Authorization
        }
        CapabilityInvocationError::AuthorizationDenied { .. }
        | CapabilityInvocationError::UnsupportedObligations { .. }
        | CapabilityInvocationError::AuthorizationRequiresApproval { .. }
        | CapabilityInvocationError::ApprovalRequestMismatch { .. }
        | CapabilityInvocationError::ApprovalFingerprintMismatch { .. }
        | CapabilityInvocationError::ApprovalNotApproved { .. }
        | CapabilityInvocationError::ApprovalLeaseMissing { .. }
        | CapabilityInvocationError::ResumeNotBlocked { .. }
        | CapabilityInvocationError::ResumeContextMismatch { .. } => {
            RuntimeFailureKind::Authorization
        }
        CapabilityInvocationError::ObligationFailed { kind, .. } => match kind {
            ironclaw_capabilities::CapabilityObligationFailureKind::Audit => {
                RuntimeFailureKind::Backend
            }
            ironclaw_capabilities::CapabilityObligationFailureKind::Mount => {
                RuntimeFailureKind::Authorization
            }
            ironclaw_capabilities::CapabilityObligationFailureKind::Network => {
                RuntimeFailureKind::Network
            }
            ironclaw_capabilities::CapabilityObligationFailureKind::Output => {
                RuntimeFailureKind::OutputTooLarge
            }
            ironclaw_capabilities::CapabilityObligationFailureKind::Resource => {
                RuntimeFailureKind::Resource
            }
            ironclaw_capabilities::CapabilityObligationFailureKind::Secret => {
                RuntimeFailureKind::Authorization
            }
        },
        CapabilityInvocationError::InvocationFingerprint { .. } => RuntimeFailureKind::InvalidInput,
        CapabilityInvocationError::ApprovalStoreMissing { .. }
        | CapabilityInvocationError::ResumeStoreMissing { .. }
        | CapabilityInvocationError::ProcessManagerMissing { .. } => RuntimeFailureKind::Backend,
        CapabilityInvocationError::Lease(_)
        | CapabilityInvocationError::RunState(_)
        | CapabilityInvocationError::Process(_) => RuntimeFailureKind::Backend,
        CapabilityInvocationError::Dispatch { kind, .. } => RuntimeFailureKind::from(*kind),
    }
}

impl From<DispatchFailureKind> for RuntimeFailureKind {
    fn from(kind: DispatchFailureKind) -> Self {
        match kind {
            DispatchFailureKind::UnknownCapability | DispatchFailureKind::UnknownProvider => {
                RuntimeFailureKind::InvalidOutput
            }
            DispatchFailureKind::MissingRuntimeBackend
            | DispatchFailureKind::UnsupportedRuntime => RuntimeFailureKind::MissingRuntime,
            DispatchFailureKind::AuthRequired => RuntimeFailureKind::Authorization,
            DispatchFailureKind::RuntimeMismatch => RuntimeFailureKind::Backend,
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::ExtensionRuntimeMismatch) => {
                RuntimeFailureKind::MissingRuntime
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Memory)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Resource) => {
                RuntimeFailureKind::Resource
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::NetworkDenied) => {
                RuntimeFailureKind::Network
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::PolicyDenied) => {
                RuntimeFailureKind::PolicyDenied
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OutputTooLarge) => {
                RuntimeFailureKind::OutputTooLarge
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::FilesystemDenied) => {
                RuntimeFailureKind::Authorization
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::SecretDenied) => {
                RuntimeFailureKind::Authorization
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::ExitFailure) => {
                RuntimeFailureKind::Process
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode) => {
                RuntimeFailureKind::InvalidInput
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OutputDecode)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InvalidResult) => {
                RuntimeFailureKind::InvalidOutput
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed) => {
                RuntimeFailureKind::OperationFailed
            }
            // A method or capability the model named that does not exist is a
            // model-fixable request error, not an infra fault: classify it as
            // InvalidInput so it surfaces as an immediate model-visible tool
            // error instead of burning the retry budget on a call that can
            // never resolve by retrying.
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::MethodMissing)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::UndeclaredCapability) => {
                RuntimeFailureKind::InvalidInput
            }
            // A guest trap is an extension-local execution failure. It can be
            // an extension defect or a call-specific failure, but retrying the
            // same guest invocation as host infrastructure cannot repair it.
            // Surface it as an operation failure so the model can change
            // approach or report the broken extension.
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest) => {
                RuntimeFailureKind::OperationFailed
            }
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Client)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Executor)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Manifest)
            | DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::UnsupportedRunner) => {
                RuntimeFailureKind::Backend
            }
            // The fail-safe "uncategorized" redaction bucket collapses to a
            // concrete internal failure rather than propagating a dedicated
            // `Unknown` category downstream. `Internal` is retryable and
            // surfaces to the model/user, so an unclassified dispatch error is
            // no longer an opaque run-ending dead-end. See
            // `docs/plans/2026-06-28-reborn-error-recoverability-audit.md`.
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Unknown) => Self::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Pinning tests for the host-runtime failure-kind and sanitized-message
    //! mappings.
    //!
    //! The dispatch failure kinds come from typed
    //! [`ironclaw_host_api::DispatchFailureKind`] values. Their display
    //! strings remain part of the public observability contract, but runtime
    //! failure mapping stays type-directed instead of reparsing strings.

    use super::*;
    use ironclaw_capabilities::CapabilityInvocationError;
    use ironclaw_extensions::{
        ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource,
    };
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
    use ironclaw_host_api::{
        CapabilityId, DispatchFailureKind, ExtensionId, HostPortCatalog,
        RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind, SecretHandle, VendorId,
        VirtualPath,
    };

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn dispatch(kind: DispatchFailureKind) -> CapabilityInvocationError {
        CapabilityInvocationError::Dispatch {
            kind,
            safe_summary: None,
            detail: None,
        }
    }

    fn auth_requirement(scopes: &[&str]) -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: VendorId::new("notion").unwrap(),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
            },
            requester_extension: ExtensionId::new("notion").unwrap(),
            provider_scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
        }
    }

    #[test]
    fn auth_required_outcome_uses_stable_gate_for_identical_requirements() {
        let capability_id = cap();
        let secrets = vec![SecretHandle::new("notion-token").unwrap()];
        let requirements = vec![auth_requirement(&["read", "write"])];

        let first =
            auth_required_outcome(capability_id.clone(), secrets.clone(), requirements.clone());
        let second = auth_required_outcome(capability_id, secrets, requirements);

        let RuntimeCapabilityOutcome::AuthRequired(first_gate) = first else {
            panic!("expected auth gate");
        };
        let RuntimeCapabilityOutcome::AuthRequired(second_gate) = second else {
            panic!("expected auth gate");
        };
        assert_eq!(first_gate.gate_id, second_gate.gate_id);
        assert!(
            first_gate.gate_id.as_str().starts_with("auth-"),
            "gate id should be stable and auth-specific: {}",
            first_gate.gate_id.as_str()
        );
    }

    #[test]
    fn auth_required_outcome_changes_gate_when_requirements_change() {
        let first = auth_required_outcome(cap(), Vec::new(), vec![auth_requirement(&["read"])]);
        let second = auth_required_outcome(cap(), Vec::new(), vec![auth_requirement(&["write"])]);

        let RuntimeCapabilityOutcome::AuthRequired(first_gate) = first else {
            panic!("expected auth gate");
        };
        let RuntimeCapabilityOutcome::AuthRequired(second_gate) = second else {
            panic!("expected auth gate");
        };
        assert_ne!(first_gate.gate_id, second_gate.gate_id);
    }

    #[test]
    fn auth_required_outcome_changes_gate_when_only_setup_changes() {
        // Regression (#6299 IronLoop): two requirements identical in provider,
        // requester, and `provider_scopes` but differing ONLY in `setup` are
        // DIFFERENT auth flows and must NOT collide on the deterministic
        // `for_auth_gate` key. Before the fix `setup` was omitted from the
        // fingerprint, so e.g. a ManualToken record and a later OAuth/Pairing
        // record produced the same gate id; the write-once gate-record store
        // then reported `GateRecordAlreadyExists`, kept the stale record, and
        // the runner reloaded and rendered the wrong authentication flow.
        use ironclaw_host_api::RuntimeCredentialAccountSetup as Setup;
        let requirement_with = |setup: Setup| RuntimeCredentialAuthRequirement {
            provider: VendorId::new("notion").unwrap(),
            setup,
            requester_extension: ExtensionId::new("notion").unwrap(),
            provider_scopes: vec!["read".to_string()],
        };
        let gate_id = |setup: Setup| {
            let RuntimeCapabilityOutcome::AuthRequired(gate) =
                auth_required_outcome(cap(), Vec::new(), vec![requirement_with(setup)])
            else {
                panic!("expected auth gate");
            };
            gate.gate_id
        };

        let manual = gate_id(Setup::ManualToken);
        let oauth = gate_id(Setup::OAuth {
            scopes: vec!["read".to_string()],
        });
        let pairing = gate_id(Setup::Pairing);
        // Distinct setup KINDS never collide (all `provider_scopes` equal).
        assert_ne!(manual, oauth, "ManualToken vs OAuth must not collide");
        assert_ne!(manual, pairing, "ManualToken vs Pairing must not collide");
        assert_ne!(oauth, pairing, "OAuth vs Pairing must not collide");

        // OAuth setups differing ONLY in their setup scopes are distinct flows
        // too (`provider_scopes` held fixed at ["read"] above and here).
        let oauth_readwrite = gate_id(Setup::OAuth {
            scopes: vec!["read".to_string(), "write".to_string()],
        });
        assert_ne!(
            oauth, oauth_readwrite,
            "OAuth setups with different setup scopes must not collide"
        );

        // Injective encoding: a single scope containing the old `,` join
        // delimiter must not collide with two scopes that join to the same
        // string — `["a,b"]` and `["a", "b"]` are DIFFERENT scope sets. Before
        // the length-prefixed `canonical_scope_list`, both encoded to "a,b".
        let one_comma_scope = gate_id(Setup::OAuth {
            scopes: vec!["a,b".to_string()],
        });
        let two_scopes = gate_id(Setup::OAuth {
            scopes: vec!["a".to_string(), "b".to_string()],
        });
        assert_ne!(
            one_comma_scope, two_scopes,
            "OAuth setup scopes must encode injectively: [\"a,b\"] != [\"a\", \"b\"]",
        );

        // The same injective guarantee must hold for the per-requirement
        // `provider_scopes` list, not only OAuth setup scopes — otherwise a
        // revert of the `provider_scopes` encoding alone would go uncaught (the
        // cases above hold `provider_scopes` fixed). Fixed ManualToken setup,
        // `provider_scopes` `["a,b"]` vs `["a", "b"]`.
        let provider_scopes_gate = |scopes: Vec<String>| {
            let requirement = RuntimeCredentialAuthRequirement {
                provider: VendorId::new("notion").unwrap(),
                setup: Setup::ManualToken,
                requester_extension: ExtensionId::new("notion").unwrap(),
                provider_scopes: scopes,
            };
            let RuntimeCapabilityOutcome::AuthRequired(gate) =
                auth_required_outcome(cap(), Vec::new(), vec![requirement])
            else {
                panic!("expected auth gate");
            };
            gate.gate_id
        };
        assert_ne!(
            provider_scopes_gate(vec!["a,b".to_string()]),
            provider_scopes_gate(vec!["a".to_string(), "b".to_string()]),
            "provider_scopes must encode injectively: [\"a,b\"] != [\"a\", \"b\"]",
        );
    }

    #[test]
    fn dispatch_kind_to_failure_pins_every_runtime_dispatch_error_kind() {
        // Every RuntimeDispatchErrorKind variant must map to a non-Unknown
        // failure kind so upstream additions are surfaced explicitly.
        let cases: &[(RuntimeDispatchErrorKind, RuntimeFailureKind)] = &[
            (
                RuntimeDispatchErrorKind::Backend,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Client,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Executor,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::ExitFailure,
                RuntimeFailureKind::Process,
            ),
            (
                RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
                RuntimeFailureKind::MissingRuntime,
            ),
            (
                RuntimeDispatchErrorKind::FilesystemDenied,
                RuntimeFailureKind::Authorization,
            ),
            (
                RuntimeDispatchErrorKind::Guest,
                RuntimeFailureKind::OperationFailed,
            ),
            (
                RuntimeDispatchErrorKind::InputEncode,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::InvalidResult,
                RuntimeFailureKind::InvalidOutput,
            ),
            (
                RuntimeDispatchErrorKind::Manifest,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Memory,
                RuntimeFailureKind::Resource,
            ),
            (
                RuntimeDispatchErrorKind::MethodMissing,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::NetworkDenied,
                RuntimeFailureKind::Network,
            ),
            (
                RuntimeDispatchErrorKind::OperationFailed,
                RuntimeFailureKind::OperationFailed,
            ),
            (
                RuntimeDispatchErrorKind::OutputDecode,
                RuntimeFailureKind::InvalidOutput,
            ),
            (
                RuntimeDispatchErrorKind::OutputTooLarge,
                RuntimeFailureKind::OutputTooLarge,
            ),
            (
                RuntimeDispatchErrorKind::PolicyDenied,
                RuntimeFailureKind::PolicyDenied,
            ),
            (
                RuntimeDispatchErrorKind::Resource,
                RuntimeFailureKind::Resource,
            ),
            (
                RuntimeDispatchErrorKind::SecretDenied,
                RuntimeFailureKind::Authorization,
            ),
            (
                RuntimeDispatchErrorKind::UndeclaredCapability,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::UnsupportedRunner,
                RuntimeFailureKind::Backend,
            ),
            // The fail-safe "uncategorized" redaction bucket collapses to a
            // concrete, surfacing `Internal` rather than a dedicated `Unknown`
            // category (which no longer exists on `RuntimeFailureKind`).
            (
                RuntimeDispatchErrorKind::Unknown,
                RuntimeFailureKind::Internal,
            ),
        ];
        for (variant, expected) in cases {
            let kind = DispatchFailureKind::Runtime(*variant);
            let actual = RuntimeFailureKind::from(kind);
            assert_eq!(
                actual, *expected,
                "dispatch kind {kind:?} should map to {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn dispatch_kind_to_failure_pins_dispatch_error_top_level_kinds() {
        let cases: &[(DispatchFailureKind, RuntimeFailureKind)] = &[
            (
                DispatchFailureKind::UnknownCapability,
                RuntimeFailureKind::InvalidOutput,
            ),
            (
                DispatchFailureKind::UnknownProvider,
                RuntimeFailureKind::InvalidOutput,
            ),
            (
                DispatchFailureKind::MissingRuntimeBackend,
                RuntimeFailureKind::MissingRuntime,
            ),
            (
                DispatchFailureKind::UnsupportedRuntime,
                RuntimeFailureKind::MissingRuntime,
            ),
            (
                DispatchFailureKind::RuntimeMismatch,
                RuntimeFailureKind::Backend,
            ),
            (
                DispatchFailureKind::AuthRequired,
                RuntimeFailureKind::Authorization,
            ),
        ];
        for (kind, expected) in cases {
            assert_eq!(RuntimeFailureKind::from(*kind), *expected, "kind {kind:?}");
        }
    }

    #[test]
    fn failure_kind_from_dispatch_unknown_capability_maps_to_invalid_output() {
        let error = dispatch(DispatchFailureKind::UnknownCapability);
        assert_eq!(failure_kind_from(&error), RuntimeFailureKind::InvalidOutput);
    }

    #[test]
    fn failure_kind_from_unknown_capability_variant_maps_to_missing_runtime() {
        let error = CapabilityInvocationError::UnknownCapability { capability: cap() };
        assert_eq!(
            failure_kind_from(&error),
            RuntimeFailureKind::MissingRuntime
        );
    }

    #[test]
    fn host_runtime_spawn_input_for_capability_passthrough_for_non_sandbox() {
        let input = serde_json::json!({
            "run": {
                "command": "",
            },
            "other": ["unchanged"],
        });

        let output = host_runtime_spawn_input_for_capability(&cap(), input.clone())
            .expect("non-sandbox capability input should pass through");

        match output {
            SpawnInputPreparation::Ready(value) => assert_eq!(value, input),
            SpawnInputPreparation::ModelInputRejected(_) => {
                panic!("non-sandbox input must pass through unchanged")
            }
        }
    }

    fn process_sandbox_cap() -> CapabilityId {
        CapabilityId::new(PROCESS_SANDBOX_CAPABILITY_ID).expect("valid process sandbox capability")
    }

    #[test]
    fn host_runtime_spawn_input_rejects_malformed_plan_as_recoverable_invalid_input() {
        // The model supplied JSON that is not a `SandboxProcessPlan` at all.
        // This is model-fixable: it must surface as a recoverable, model-visible
        // tool error (`InvalidInput`), never a terminal `HostRuntimeError`.
        let input = serde_json::json!({ "not_run": true });

        let output = host_runtime_spawn_input_for_capability(&process_sandbox_cap(), input)
            .expect("malformed model plan must not be a terminal host error");

        match output {
            SpawnInputPreparation::ModelInputRejected(failure) => {
                assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
                assert_eq!(
                    failure.disposition(),
                    crate::CapabilityFailureDisposition::ModelVisibleToolError
                );
                // The serde cause must ride the model-visible Diagnostic channel
                // so the model learns WHAT is malformed, not just that it is.
                let cause = failure
                    .model_visible_cause()
                    .expect("malformed plan rejection must carry the parse cause");
                assert!(
                    cause.contains("missing field"),
                    "cause must name the missing field, got: {cause}"
                );
            }
            SpawnInputPreparation::Ready(_) => {
                panic!("malformed plan must be rejected as model-visible InvalidInput")
            }
        }
    }

    #[test]
    fn host_runtime_spawn_input_rejects_invalid_plan_as_recoverable_invalid_input() {
        // The model supplied a structurally-parseable plan that fails
        // `ValidatedSandboxProcessPlan` validation (empty command). Still
        // model-fixable → recoverable `InvalidInput`, not terminal.
        let input = serde_json::json!({ "run": { "command": "" } });

        let output = host_runtime_spawn_input_for_capability(&process_sandbox_cap(), input)
            .expect("invalid model plan must not be a terminal host error");

        match output {
            SpawnInputPreparation::ModelInputRejected(failure) => {
                assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
                assert_eq!(
                    failure.disposition(),
                    crate::CapabilityFailureDisposition::ModelVisibleToolError
                );
                // The validation cause must ride the model-visible Diagnostic
                // channel so the model learns which field broke which rule.
                let cause = failure
                    .model_visible_cause()
                    .expect("invalid plan rejection must carry the validation cause");
                assert!(
                    cause.contains("run command must not be empty"),
                    "cause must name the offending field and rule, got: {cause}"
                );
            }
            SpawnInputPreparation::Ready(_) => {
                panic!("invalid plan must be rejected as model-visible InvalidInput")
            }
        }
    }

    #[test]
    fn host_runtime_spawn_input_accepts_valid_plan() {
        let input = serde_json::json!({ "run": { "command": "echo", "args": ["ok"] } });

        let output = host_runtime_spawn_input_for_capability(&process_sandbox_cap(), input)
            .expect("valid plan preparation must not error");

        match output {
            SpawnInputPreparation::Ready(value) => {
                assert!(value.is_object(), "validated plan serializes to an object");
            }
            SpawnInputPreparation::ModelInputRejected(_) => {
                panic!("valid plan must be accepted")
            }
        }
    }

    #[test]
    fn sanitized_failure_message_redacts_dispatch_kind_to_stable_form() {
        let error = dispatch(DispatchFailureKind::Runtime(
            RuntimeDispatchErrorKind::NetworkDenied,
        ));
        let message = sanitized_failure_message(&error).expect("dispatch produces a message");
        // With no host-authored safe_summary, the message is the fixed
        // human-readable summary for the redacted kind — derived only from the
        // category, never from raw backend strings.
        assert_eq!(message, "the tool was denied network access");
    }

    #[test]
    fn sanitized_failure_message_uses_dispatch_safe_summary() {
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            safe_summary: Some(
                "apply_patch failed for path workspace main.rs: old_string matched 0 times"
                    .to_string(),
            ),
            detail: None,
        };

        assert_eq!(
            sanitized_failure_message(&error).as_deref(),
            Some("apply_patch failed for path workspace main.rs: old_string matched 0 times")
        );
    }

    #[test]
    fn sanitized_failure_message_retains_dispatch_cause_for_detail_consumer() {
        // The public message fails CLOSED: it is persisted into run-state rows
        // and published on the runtime event sink, so a wild raw cause (paths,
        // tokens) degrades to the kind's fixed sentence. The descriptive cause
        // is not lost — failure_from carries it (registry-scrubbed) on the
        // in-process-only model_visible_cause channel for the Diagnostic seam.
        let secret = concat!("ghp_", "012345678901234567890123456789012345");
        let raw = format!("read_file failed at /workspace/config using {secret}");
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            safe_summary: Some(raw),
            detail: None,
        };

        let message = sanitized_failure_message(&error).expect("dispatch produces a message");
        assert_eq!(
            message,
            RuntimeDispatchErrorKind::OperationFailed.human_summary(),
            "wild raw cause must degrade the public message to the kind sentence"
        );

        let failure = failure_from(error, CapabilityId::new("demo.read_file").unwrap());
        let cause = failure
            .model_visible_cause
            .as_deref()
            .expect("raw cause must ride the model-visible channel");
        assert!(
            cause.contains("read_file failed at /workspace/config"),
            "descriptive cause (paths included) must survive for the model: {cause}"
        );
        assert!(
            !cause.contains(secret),
            "registry secret must be scrubbed from the model-visible cause: {cause}"
        );
        let rendered = format!("{failure:?}");
        assert!(
            !rendered.contains("/workspace/config") && !rendered.contains(secret),
            "Debug must not render the model-visible cause: {rendered}"
        );

        // A host-authored, validation-clean summary still passes through to
        // the public message unchanged.
        let clean = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            safe_summary: Some("trigger_create input failed validation".to_string()),
            detail: None,
        };
        assert_eq!(
            sanitized_failure_message(&clean).as_deref(),
            Some("trigger_create input failed validation")
        );
    }

    #[test]
    fn failure_from_carries_rejected_safe_summary_on_the_diagnostic_detail() {
        // A path-bearing (or newline-bearing) failure reason fails the strict
        // loop safe-summary validator, so the message degrades to the fixed
        // category sentence. The raw reason must NOT be dropped: it rides the
        // model-visible diagnostic detail channel, which is exactly how the
        // model learns what to repair (e.g. which path was denied).
        let raw = "shell execution failed: cannot read /etc/passwd\nsecond line".to_string();
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Executor),
            safe_summary: Some(raw.clone()),
            detail: None,
        };

        let failure = failure_from(error, cap());

        assert_eq!(
            failure.message.as_deref(),
            Some("the tool executor failed"),
            "message must stay the fixed category sentence"
        );
        assert_eq!(
            failure.detail,
            Some(ironclaw_host_api::DispatchFailureDetail::Diagnostic { text: raw }),
            "the raw reason must ride the diagnostic detail"
        );
    }

    #[test]
    fn failure_from_bounds_rejected_summary_diagnostic_on_char_boundaries() {
        let raw = format!("/{}", "é".repeat(600));
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Executor),
            safe_summary: Some(raw),
            detail: None,
        };

        let failure = failure_from(error, cap());
        let Some(ironclaw_host_api::DispatchFailureDetail::Diagnostic { text }) = failure.detail
        else {
            panic!("expected bounded diagnostic detail");
        };
        assert_eq!(text.chars().count(), 512);
        assert!(text.ends_with("..."));
    }

    #[test]
    fn failure_from_leaves_validator_safe_summaries_on_the_message_alone() {
        // When the reason already passes the strict validator it travels via
        // `message` (the loop layer derives the model-visible diagnostic from
        // it directly), so no duplicate diagnostic detail is attached.
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            safe_summary: Some(
                "apply_patch failed for path workspace main.rs: old_string matched 0 times"
                    .to_string(),
            ),
            detail: None,
        };

        let failure = failure_from(error, cap());

        assert_eq!(
            failure.message.as_deref(),
            Some("apply_patch failed for path workspace main.rs: old_string matched 0 times")
        );
        assert_eq!(failure.detail, None);
    }

    #[test]
    fn failure_from_preserves_dispatch_detail() {
        let issue = ironclaw_host_api::DispatchInputIssue::new(
            "schedule.kind",
            ironclaw_host_api::DispatchInputIssueCode::MissingRequired,
        )
        .expected("cron or once");
        let error = CapabilityInvocationError::Dispatch {
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            safe_summary: Some("trigger_create input failed validation".to_string()),
            detail: Some(ironclaw_host_api::DispatchFailureDetail::InvalidInput {
                issues: vec![issue.clone()],
            }),
        };

        let failure = failure_from(
            error,
            CapabilityId::new("builtin.trigger_create").expect("valid capability id"),
        );

        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
        assert_eq!(
            failure.detail,
            Some(ironclaw_host_api::DispatchFailureDetail::InvalidInput {
                issues: vec![issue]
            })
        );
    }

    #[test]
    fn runtime_failure_kind_as_str_is_stable_snake_case() {
        // Pin the public metric/tracing tokens; renaming any of these is a
        // breaking observability contract change.
        assert_eq!(RuntimeFailureKind::Authorization.as_str(), "authorization");
        assert_eq!(RuntimeFailureKind::Backend.as_str(), "backend");
        assert_eq!(RuntimeFailureKind::Cancelled.as_str(), "cancelled");
        assert_eq!(RuntimeFailureKind::Dispatcher.as_str(), "dispatcher");
        assert_eq!(RuntimeFailureKind::Internal.as_str(), "internal");
        assert_eq!(RuntimeFailureKind::InvalidInput.as_str(), "invalid_input");
        assert_eq!(RuntimeFailureKind::InvalidOutput.as_str(), "invalid_output");
        assert_eq!(
            RuntimeFailureKind::MissingRuntime.as_str(),
            "missing_runtime"
        );
        assert_eq!(RuntimeFailureKind::Network.as_str(), "network");
        assert_eq!(
            RuntimeFailureKind::OperationFailed.as_str(),
            "operation_failed"
        );
        assert_eq!(
            RuntimeFailureKind::OutputTooLarge.as_str(),
            "output_too_large"
        );
        assert_eq!(RuntimeFailureKind::PolicyDenied.as_str(), "policy_denied");
        assert_eq!(RuntimeFailureKind::Process.as_str(), "process");
        assert_eq!(RuntimeFailureKind::Resource.as_str(), "resource");
        assert_eq!(RuntimeFailureKind::Transient.as_str(), "transient");
        assert_eq!(RuntimeFailureKind::Unavailable.as_str(), "unavailable");
    }

    #[test]
    fn capability_failure_disposition_maps_failure_kinds_once() {
        use crate::CapabilityFailureDisposition::*;

        let cases = [
            (RuntimeFailureKind::Authorization, ModelVisibleToolError),
            (RuntimeFailureKind::Backend, RetrySameCall),
            (RuntimeFailureKind::Cancelled, ModelVisibleToolError),
            (RuntimeFailureKind::Dispatcher, ModelVisibleToolError),
            (RuntimeFailureKind::Internal, RetrySameCall),
            (RuntimeFailureKind::InvalidInput, ModelVisibleToolError),
            (RuntimeFailureKind::InvalidOutput, ModelVisibleToolError),
            (RuntimeFailureKind::MissingRuntime, ModelVisibleToolError),
            (RuntimeFailureKind::Network, RetrySameCall),
            (RuntimeFailureKind::OperationFailed, ModelVisibleToolError),
            (RuntimeFailureKind::OutputTooLarge, ModelVisibleToolError),
            (RuntimeFailureKind::PolicyDenied, ModelVisibleToolError),
            (RuntimeFailureKind::Process, ModelVisibleToolError),
            (RuntimeFailureKind::Resource, ModelVisibleToolError),
            (RuntimeFailureKind::Transient, RetrySameCall),
            (RuntimeFailureKind::Unavailable, RetrySameCall),
        ];

        for (kind, expected) in cases {
            assert_eq!(
                crate::capability_failure_disposition(kind),
                expected,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn capability_failure_disposition_retries_retryable_kinds_before_exhaustion() {
        use crate::CapabilityFailureDisposition::*;
        for kind in [
            RuntimeFailureKind::Backend,
            RuntimeFailureKind::Internal,
            RuntimeFailureKind::Network,
            RuntimeFailureKind::Transient,
            RuntimeFailureKind::Unavailable,
        ] {
            assert_eq!(
                crate::capability_failure_disposition(kind),
                RetrySameCall,
                "{kind:?}"
            );
        }
    }

    // ─── capability_credential_requirements unit tests ──────────────────────────
    //
    // These were previously integration tests in host_runtime_services_contract.rs
    // that called the function via `ironclaw_host_runtime::capability_credential_requirements`.
    // They are kept here as unit tests because the function is now `pub(crate)`,
    // making it invisible to external test binaries. Coverage is equivalent.

    fn build_descriptor_for_manifest(
        manifest_toml: &str,
    ) -> ironclaw_host_api::CapabilityDescriptor {
        let manifest = ExtensionManifest::parse(
            manifest_toml,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            &capability_provider_contracts(),
        )
        .expect("manifest must parse");
        let cap_id = manifest.capabilities[0].id.clone();
        let root =
            VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
        let package = ExtensionPackage::from_manifest(manifest, root).expect("package must build");
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).unwrap();
        registry.get_capability(&cap_id).unwrap().clone()
    }

    /// `capability_credential_requirements` must return exactly the required
    /// `SecretHandle`-source handles declared in the descriptor, filtered to
    /// `required == true`, and must not include `ProductAuthAccount`-source handles.
    ///
    /// Previously `credential_requirements_extraction_matches_descriptor_required_credentials`
    /// in host_runtime_services_contract.rs (moved here because the function is
    /// now `pub(crate)`; coverage is identical).
    #[test]
    fn credential_requirements_extraction_matches_descriptor_required_credentials() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "script"
name = "Script With Credential"
version = "0.1.0"
description = "Script extension that requires a runtime credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test/input.v1.json"
output_schema_ref = "schemas/test/output.v1.json"
prompt_doc_ref = "prompts/test.md"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "script_api_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "x-api-key" }
required = true
"#;
        let descriptor = build_descriptor_for_manifest(MANIFEST);

        let (preflight_handles, preflight_reqs) = capability_credential_requirements(&descriptor);

        // The obligation handler iterates `descriptor.runtime_credentials` filtered
        // to `required == true` — verify `capability_credential_requirements` produces
        // the same handles from the same source.
        let expected_handles: Vec<SecretHandle> = descriptor
            .runtime_credentials
            .iter()
            .filter(|cred| cred.required)
            .map(|cred| cred.handle.clone())
            .collect();

        assert_eq!(
            preflight_handles, expected_handles,
            "capability_credential_requirements must return exactly the required handles from the descriptor"
        );
        assert_eq!(preflight_handles.len(), 1, "expected one required handle");
        assert_eq!(
            preflight_handles[0].as_str(),
            "script_api_token",
            "required handle must be script_api_token"
        );
        // The manifest source is `secret_handle` (not `product_auth_account`), so
        // `product_auth_requirement_for` returns None — credential_requirements is empty.
        assert!(
            preflight_reqs.is_empty(),
            "credential_requirements must be empty for secret_handle source (no product_auth_account)"
        );
    }

    /// A capability descriptor with only `required = false` credentials must
    /// produce empty `required_secrets` and `credential_requirements`.
    ///
    /// Previously `credential_requirements_extraction_returns_empty_for_all_optional_credentials`
    /// in host_runtime_services_contract.rs (moved here because the function is now
    /// `pub(crate)`; coverage is identical).
    #[test]
    fn credential_requirements_extraction_returns_empty_for_all_optional_credentials() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "script"
name = "Script With Optional Credential"
version = "0.1.0"
description = "Script extension with an optional runtime credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test/input.v1.json"
output_schema_ref = "schemas/test/output.v1.json"
prompt_doc_ref = "prompts/test.md"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "optional_api_token"
source = { type = "secret_handle" }
audience = { scheme = "https", host_pattern = "api.example.com" }
target = { type = "header", name = "x-api-key" }
required = false
"#;
        let descriptor = build_descriptor_for_manifest(MANIFEST);

        let (required_secrets, credential_requirements) =
            capability_credential_requirements(&descriptor);

        assert!(
            required_secrets.is_empty(),
            "capability with only optional credentials must produce empty required_secrets; got {required_secrets:?}"
        );
        assert!(
            credential_requirements.is_empty(),
            "capability with only optional credentials must produce empty credential_requirements; got {credential_requirements:?}"
        );
    }

    /// A REQUIRED `product_auth_account`-source credential must NOT be pushed into
    /// `required_secrets` (its handle is only an injection slot that the account
    /// resolver stages later, so a pre-flight `metadata()` probe would false-positive
    /// `AuthRequired` for an already-connected account). It MUST still surface in
    /// `credential_requirements` so the auth payload can describe the product-auth need.
    #[test]
    fn credential_requirements_extraction_excludes_required_product_auth_account() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "script"
name = "Script With Product-Auth Credential"
version = "0.1.0"
description = "Script extension that requires a product-auth account credential"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"
args = []

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "script.echo"
description = "Echo through Script"
effects = ["dispatch_capability", "use_secret"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test/input.v1.json"
output_schema_ref = "schemas/test/output.v1.json"
prompt_doc_ref = "prompts/test.md"

[[capability_provider.tools.capabilities.runtime_credentials]]
handle = "github_runtime_token"
source = { type = "product_auth_account", provider = "github" }
audience = { scheme = "https", host_pattern = "api.github.com" }
target = { type = "header", name = "authorization", prefix = "Bearer " }
required = true
"#;
        let descriptor = build_descriptor_for_manifest(MANIFEST);

        let (required_secrets, credential_requirements) =
            capability_credential_requirements(&descriptor);

        assert!(
            required_secrets.is_empty(),
            "a required product_auth_account credential must be excluded from required_secrets \
             (the slot handle is not a presence-checkable secret); got {required_secrets:?}"
        );
        assert!(
            !credential_requirements.is_empty(),
            "a required product_auth_account credential must still surface in credential_requirements"
        );
    }

    #[test]
    fn runtime_failure_summary_is_bounded_and_blank_messages_are_not_safe() {
        let blank = RuntimeCapabilityFailure::new(
            cap(),
            RuntimeFailureKind::InvalidInput,
            Some("   ".to_string()),
        );
        assert!(blank.safe_summary().is_none());
        assert_eq!(
            blank.disposition(),
            crate::CapabilityFailureDisposition::ModelVisibleToolError
        );

        let long = RuntimeCapabilityFailure::new(
            cap(),
            RuntimeFailureKind::InvalidInput,
            Some("x".repeat(3000)),
        );
        let summary = long.safe_summary().expect("long message is still safe");
        assert_eq!(summary.chars().count(), 512);
        assert!(summary.ends_with("..."));

        let multibyte = RuntimeCapabilityFailure::new(
            cap(),
            RuntimeFailureKind::InvalidInput,
            Some("é".repeat(3000)),
        );
        let summary = multibyte
            .safe_summary()
            .expect("long multibyte message is still safe");
        assert_eq!(summary.chars().count(), 512);
        assert!(summary.ends_with("..."));

        let exact = RuntimeCapabilityFailure::new(
            cap(),
            RuntimeFailureKind::InvalidInput,
            Some("x".repeat(512)),
        );
        assert_eq!(exact.safe_summary(), Some("x".repeat(512)));
    }

    #[test]
    fn unavailable_from_run_state_uses_redacted_reasons() {
        let error = RunStateError::InvalidPath("/private/users/secret/database.sqlite".to_string());
        let host_error = unavailable_from_run_state(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/private/"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "run-state storage path invalid");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }

        let error = RunStateError::Filesystem("connection refused at /tmp/runstate.db".to_string());
        let host_error = unavailable_from_run_state(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/tmp"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "run-state filesystem unavailable");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn unavailable_from_process_error_uses_redacted_reasons() {
        let error = ProcessError::InvalidPath("/private/users/secret/processes".to_string());
        let host_error = unavailable_from_process_error(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/private/"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "process storage path invalid");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }

        let error = ProcessError::Filesystem(FilesystemError::Backend {
            path: VirtualPath::new("/users/user1/processes.db").unwrap(),
            operation: FilesystemOperation::ReadFile,
            reason: "connection refused at /tmp/processes.db".to_string(),
        });
        let host_error = unavailable_from_process_error(error);
        match host_error {
            HostRuntimeError::Unavailable { reason } => {
                assert!(
                    !reason.contains("/tmp"),
                    "sanitized reason must not leak filesystem paths, got {reason:?}"
                );
                assert_eq!(reason, "process filesystem unavailable");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }
}
