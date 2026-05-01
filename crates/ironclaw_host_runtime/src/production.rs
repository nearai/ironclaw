//! Production composition of the [`HostRuntime`] contract.
//!
//! [`DefaultHostRuntime`] is the contract-level facade that upper turn/loop
//! services should depend on. Internally it composes
//! [`ironclaw_capabilities::CapabilityHost`] with neutral kernel services —
//! extension registry, capability dispatcher, trust-aware authorizer,
//! run-state and approval stores, capability-lease store, and process
//! manager.
//!
//! This layer evaluates the package's manifest-derived trust input immediately
//! before invoking [`CapabilityHost`] so authorization consumes a host-owned
//! [`TrustDecision`](ironclaw_trust::TrustDecision) instead of caller-supplied
//! claims. The default empty policy fails closed until composition supplies a
//! concrete host policy.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_authorization::{CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityInvocationError, CapabilityInvocationRequest,
    CapabilityInvocationResult, CapabilityObligationHandler,
};
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry};
use ironclaw_host_api::{
    ApprovalRequestId, CapabilityDispatcher, CapabilityId, InvocationId, PackageSource,
    ResourceScope, RuntimeKind,
};
use ironclaw_processes::{
    ProcessCancellationRegistry, ProcessError, ProcessHost, ProcessManager, ProcessResultStore,
    ProcessStatus, ProcessStore,
};
use ironclaw_run_state::{ApprovalRequestStore, RunStateError, RunStateStore, RunStatus};
use ironclaw_trust::{HostTrustPolicy, TrustDecision, TrustError, TrustPolicy, TrustProvenance};

use crate::{
    BuiltinObligationHandler, BuiltinObligationServices, CancelRuntimeWorkOutcome,
    CancelRuntimeWorkRequest, CapabilitySurfaceVersion, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalGate, RuntimeBackendHealth,
    RuntimeBlockedReason, RuntimeCapabilityCompleted, RuntimeCapabilityFailure,
    RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind, RuntimeStatusRequest,
    RuntimeWorkId, RuntimeWorkSummary, VisibleCapabilityRequest, VisibleCapabilitySurface,
};

/// Default production wiring for [`HostRuntime`].
pub struct DefaultHostRuntime {
    registry: Arc<ExtensionRegistry>,
    dispatcher: Arc<dyn CapabilityDispatcher>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    trust_policy: Arc<dyn TrustPolicy>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    process_manager: Option<Arc<dyn ProcessManager>>,
    process_store: Option<Arc<dyn ProcessStore>>,
    process_result_store: Option<Arc<dyn ProcessResultStore>>,
    process_cancellation_registry: Option<Arc<ProcessCancellationRegistry>>,
    runtime_health: Option<Arc<dyn RuntimeBackendHealth>>,
    obligation_handler: Option<Arc<dyn CapabilityObligationHandler>>,
    surface_version: CapabilitySurfaceVersion,
}

impl DefaultHostRuntime {
    /// Constructs a default host runtime over the supplied kernel services.
    ///
    /// The runtime starts with an empty host trust policy, so capability
    /// dispatch fails closed until composition attaches a concrete policy with
    /// [`Self::with_trust_policy`] or [`Self::with_trust_policy_dyn`].
    ///
    /// Callers must additionally attach a run-state store and approval-
    /// request store via [`with_run_state`](Self::with_run_state) and
    /// [`with_approval_requests`](Self::with_approval_requests) before
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
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
            trust_policy: Arc::new(HostTrustPolicy::empty()),
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            process_manager: None,
            process_store: None,
            process_result_store: None,
            process_cancellation_registry: None,
            runtime_health: None,
            obligation_handler: None,
            surface_version,
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

    /// Attaches the run-state store used to record invocation lifecycle.
    pub fn with_run_state(mut self, run_state: Arc<dyn RunStateStore>) -> Self {
        self.run_state = Some(run_state);
        self
    }

    /// Attaches the approval-request store used to persist approval prompts.
    pub fn with_approval_requests(
        mut self,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        self.approval_requests = Some(approval_requests);
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
}

#[async_trait]
impl HostRuntime for DefaultHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let RuntimeCapabilityRequest {
            mut context,
            capability_id,
            estimate,
            input,
            idempotency_key,
            trust_decision: _caller_trust_decision,
        } = request;
        let scope = context.resource_scope.clone();
        let invocation_id = context.invocation_id;
        // Forward the (currently advisory) idempotency key into spans for
        // audit/tracing only — dedupe enforcement is not yet implemented at
        // this layer (see `RuntimeCapabilityRequest::idempotency_key`).
        let idempotency_key = idempotency_key.map(|key| key.as_str().to_string());
        if let Some(key) = idempotency_key.as_deref() {
            tracing::debug!(
                capability_id = %capability_id,
                idempotency_key = %key,
                "capability invocation accepted advisory idempotency key (not yet enforced)"
            );
        }

        let trust_decision = match self.evaluate_invocation_trust(&capability_id) {
            Ok(host_decision) => host_decision,
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    trust_error_kind = error.kind(),
                    "capability trust evaluation failed before dispatch"
                );
                return Ok(trust_evaluation_failure(capability_id, error));
            }
        };
        context.trust = trust_decision.effective_trust.class();

        let mut host = CapabilityHost::new(
            self.registry.as_ref(),
            self.dispatcher.as_ref(),
            self.authorizer.as_ref(),
        );
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
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

        let invocation = CapabilityInvocationRequest {
            context,
            capability_id: capability_id.clone(),
            estimate,
            input,
            trust_decision,
        };

        match host.invoke_json(invocation).await {
            Ok(result) => Ok(RuntimeCapabilityOutcome::Completed(Box::new(
                completed_outcome_from(result, capability_id),
            ))),
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    error_kind = failure_kind_from(&error).as_str(),
                    idempotency_key = idempotency_key.as_deref().unwrap_or(""),
                    "capability invocation failed"
                );
                self.translate_invocation_error(error, capability_id, scope, invocation_id)
                    .await
            }
        }
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
        let _ = request;
        let descriptors = self.registry.capabilities().cloned().collect();
        Ok(VisibleCapabilitySurface {
            version: self.surface_version.clone(),
            descriptors,
        })
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

            for record in records {
                if record.status != ProcessStatus::Running {
                    continue;
                }
                process_invocations.push(record.invocation_id);
                let work_id = RuntimeWorkId::Process(record.process_id);
                match process_host.kill(&request.scope, record.process_id).await {
                    Ok(record) => {
                        if let Some(result_store) = &self.process_result_store {
                            result_store
                                .kill(&record.scope, record.process_id)
                                .await
                                .map_err(unavailable_from_process_error)?;
                        }
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
                        let runtime = self
                            .registry
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
        let required = required_runtime_backends(&self.registry);
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
    fn evaluate_invocation_trust(
        &self,
        capability_id: &CapabilityId,
    ) -> Result<TrustDecision, TrustEvaluationError> {
        let policy = self.trust_policy.as_ref();

        let descriptor = self
            .registry
            .get_capability(capability_id)
            .ok_or(TrustEvaluationError::UnknownCapability)?;
        let package = self
            .registry
            .get_extension(&descriptor.provider)
            .ok_or(TrustEvaluationError::MissingPackage)?;
        let package_descriptor = package
            .capabilities
            .iter()
            .find(|candidate| candidate.id == *capability_id)
            .ok_or(TrustEvaluationError::StalePackageDescriptor)?;
        if package_descriptor != descriptor {
            return Err(TrustEvaluationError::ConflictingPackageDescriptor);
        }

        let input = trust_policy_input_for_local_manifest(package)?;
        let decision = match policy.evaluate(&input) {
            Ok(decision) => decision,
            Err(error) => {
                tracing::debug!(
                    capability_id = %capability_id,
                    trust_policy_error_kind = trust_error_label(&error),
                    "host trust policy evaluation returned an error"
                );
                return Err(TrustEvaluationError::Policy);
            }
        };
        trace_trust_decision(capability_id, &decision);
        Ok(decision)
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
                    Ok(None) => Ok(RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                        capability_id: capability,
                        kind: RuntimeFailureKind::Authorization,
                        message: Some(
                            "approval required but no approval request was persisted".to_string(),
                        ),
                    })),
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
            other => Ok(RuntimeCapabilityOutcome::Failed(failure_from(
                other,
                capability_id,
            ))),
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

#[derive(Debug, Clone, Copy)]
enum TrustEvaluationError {
    UnknownCapability,
    MissingPackage,
    StalePackageDescriptor,
    ConflictingPackageDescriptor,
    TrustInput,
    Policy,
}

impl TrustEvaluationError {
    const fn kind(self) -> &'static str {
        match self {
            Self::UnknownCapability => "unknown_capability",
            Self::MissingPackage => "missing_package",
            Self::StalePackageDescriptor => "stale_package_descriptor",
            Self::ConflictingPackageDescriptor => "conflicting_package_descriptor",
            Self::TrustInput => "trust_input",
            Self::Policy => "policy",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::UnknownCapability => "unknown capability",
            Self::MissingPackage => "capability provider trust metadata is missing",
            Self::StalePackageDescriptor | Self::ConflictingPackageDescriptor => {
                "capability provider trust metadata is stale"
            }
            Self::TrustInput => "capability provider trust metadata is invalid",
            Self::Policy => "capability provider trust policy evaluation failed",
        }
    }
}

fn trust_policy_input_for_local_manifest(
    package: &ExtensionPackage,
) -> Result<ironclaw_trust::TrustPolicyInput, TrustEvaluationError> {
    package
        .trust_policy_input(local_manifest_source(package), None, None)
        .map_err(|_| TrustEvaluationError::TrustInput)
}

fn local_manifest_source(package: &ExtensionPackage) -> PackageSource {
    PackageSource::LocalManifest {
        path: format!(
            "{}/manifest.toml",
            package.root.as_str().trim_end_matches('/')
        ),
    }
}

fn trace_trust_decision(capability_id: &CapabilityId, decision: &TrustDecision) {
    tracing::debug!(
        capability_id = %capability_id,
        effective_trust = ?decision.effective_trust.class(),
        trust_provenance = trust_provenance_label(&decision.provenance),
        trust_allowed_effect_count = decision.authority_ceiling.allowed_effects.len(),
        trust_has_resource_ceiling = decision.authority_ceiling.max_resource_ceiling.is_some(),
        "evaluated capability provider trust from host policy"
    );
}

fn trust_provenance_label(provenance: &TrustProvenance) -> &'static str {
    match provenance {
        TrustProvenance::Default => "default",
        TrustProvenance::Bundled => "bundled",
        TrustProvenance::AdminConfig => "admin_config",
        TrustProvenance::SignedRegistry { .. } => "signed_registry",
        TrustProvenance::LocalManifest => "local_manifest",
    }
}

fn trust_error_label(error: &TrustError) -> &'static str {
    match error {
        TrustError::InvariantViolation { .. } => "invariant_violation",
    }
}

fn trust_evaluation_failure(
    capability_id: CapabilityId,
    error: TrustEvaluationError,
) -> RuntimeCapabilityOutcome {
    RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
        capability_id,
        kind: trust_evaluation_failure_kind(error),
        message: Some(error.message().to_string()),
    })
}

fn trust_evaluation_failure_kind(error: TrustEvaluationError) -> RuntimeFailureKind {
    match error {
        TrustEvaluationError::UnknownCapability => RuntimeFailureKind::MissingRuntime,
        TrustEvaluationError::MissingPackage
        | TrustEvaluationError::StalePackageDescriptor
        | TrustEvaluationError::ConflictingPackageDescriptor
        | TrustEvaluationError::TrustInput
        | TrustEvaluationError::Policy => RuntimeFailureKind::Authorization,
    }
}

/// Maps a [`RunStateError`] to a sanitized [`HostRuntimeError::Unavailable`].
///
/// `RunStateError::InvalidPath` and `Filesystem` carry raw filesystem
/// strings; `Serialization`/`Deserialization` carry serde internals. Forward
/// the redacted variant discriminator instead of `error.to_string()` so the
/// boundary stays infrastructure-opaque to upper services.
fn unavailable_from_run_state(error: RunStateError) -> HostRuntimeError {
    let reason = match error {
        RunStateError::UnknownInvocation { .. } => "run-state record not found",
        RunStateError::InvocationAlreadyExists { .. } => "run-state record already exists",
        RunStateError::UnknownApprovalRequest { .. } => "approval request not found",
        RunStateError::ApprovalRequestAlreadyExists { .. } => "approval request already exists",
        RunStateError::ApprovalNotPending { .. } => "approval request not pending",
        RunStateError::InvalidPath(_) => "run-state storage path invalid",
        RunStateError::Filesystem(_) => "run-state filesystem unavailable",
        RunStateError::Serialization(_) => "run-state serialization failed",
        RunStateError::Deserialization(_) => "run-state deserialization failed",
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
        usage: result.dispatch.usage,
    }
}

fn failure_from(
    error: CapabilityInvocationError,
    capability_id: CapabilityId,
) -> RuntimeCapabilityFailure {
    let kind = failure_kind_from(&error);
    let message = sanitized_failure_message(&error);
    RuntimeCapabilityFailure {
        capability_id,
        kind,
        message,
    }
}

/// Returns a stable, redacted summary message for a capability invocation
/// failure.
///
/// Variants that wrap inner errors (`Lease`, `RunState`, `Process`,
/// `InvocationFingerprint`) or that surface free-form storage/runtime
/// strings are mapped to fixed, infrastructure-opaque labels. Variants whose
/// `Display` impl is itself stable (capability id + enum discriminator) flow
/// through unchanged.
fn sanitized_failure_message(error: &CapabilityInvocationError) -> Option<String> {
    use CapabilityInvocationError::*;
    match error {
        UnknownCapability { .. }
        | AuthorizationDenied { .. }
        | UnsupportedObligations { .. }
        | ObligationFailed { .. }
        | AuthorizationRequiresApproval { .. }
        | ApprovalRequestMismatch { .. }
        | ApprovalFingerprintMismatch { .. }
        | ApprovalNotApproved { .. }
        | ApprovalLeaseMissing { .. }
        | ApprovalStoreMissing { .. }
        | ResumeStoreMissing { .. }
        | ProcessManagerMissing { .. }
        | ResumeNotBlocked { .. }
        | ResumeContextMismatch { .. }
        | Dispatch { .. } => Some(error.to_string()),
        InvocationFingerprint { .. } => Some("invocation fingerprint failed".to_string()),
        Lease(_) => Some("capability lease store unavailable".to_string()),
        RunState(_) => Some("run-state store unavailable".to_string()),
        Process(_) => Some("process manager unavailable".to_string()),
    }
}

pub(crate) fn failure_kind_from(error: &CapabilityInvocationError) -> RuntimeFailureKind {
    match error {
        CapabilityInvocationError::UnknownCapability { .. } => RuntimeFailureKind::MissingRuntime,
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
        CapabilityInvocationError::Dispatch { kind } => dispatch_kind_to_failure(kind),
    }
}

fn dispatch_kind_to_failure(kind: &str) -> RuntimeFailureKind {
    match kind {
        "UnknownCapability"
        | "UnknownProvider"
        | "MissingRuntimeBackend"
        | "UnsupportedRuntime"
        | "ExtensionRuntimeMismatch" => RuntimeFailureKind::MissingRuntime,
        "RuntimeMismatch" => RuntimeFailureKind::Backend,
        "Memory" | "Resource" => RuntimeFailureKind::Resource,
        "NetworkDenied" => RuntimeFailureKind::Network,
        "OutputTooLarge" => RuntimeFailureKind::OutputTooLarge,
        "FilesystemDenied" => RuntimeFailureKind::Authorization,
        "ExitFailure" => RuntimeFailureKind::Process,
        "InputEncode" | "OutputDecode" | "InvalidResult" => RuntimeFailureKind::InvalidInput,
        "Backend"
        | "Client"
        | "Executor"
        | "Guest"
        | "Manifest"
        | "MethodMissing"
        | "UndeclaredCapability"
        | "UnsupportedRunner" => RuntimeFailureKind::Backend,
        _ => RuntimeFailureKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    //! Pinning tests for the host-runtime failure-kind and sanitized-message
    //! mappings.
    //!
    //! The dispatch-kind strings come from
    //! [`ironclaw_host_api::RuntimeDispatchErrorKind::as_str`] and from
    //! [`ironclaw_capabilities::error::dispatch_error_kind`]. Both are
    //! treated as part of the public contract surface; if an upstream rename
    //! drops a string, this module fails closed instead of silently
    //! degrading to [`RuntimeFailureKind::Unknown`].

    use super::*;
    use ironclaw_capabilities::CapabilityInvocationError;
    use ironclaw_host_api::{CapabilityId, RuntimeDispatchErrorKind};

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn dispatch(kind: &str) -> CapabilityInvocationError {
        CapabilityInvocationError::Dispatch {
            kind: kind.to_string(),
        }
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
            (RuntimeDispatchErrorKind::Guest, RuntimeFailureKind::Backend),
            (
                RuntimeDispatchErrorKind::InputEncode,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::InvalidResult,
                RuntimeFailureKind::InvalidInput,
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
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::NetworkDenied,
                RuntimeFailureKind::Network,
            ),
            (
                RuntimeDispatchErrorKind::OutputDecode,
                RuntimeFailureKind::InvalidInput,
            ),
            (
                RuntimeDispatchErrorKind::OutputTooLarge,
                RuntimeFailureKind::OutputTooLarge,
            ),
            (
                RuntimeDispatchErrorKind::Resource,
                RuntimeFailureKind::Resource,
            ),
            (
                RuntimeDispatchErrorKind::UndeclaredCapability,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::UnsupportedRunner,
                RuntimeFailureKind::Backend,
            ),
            (
                RuntimeDispatchErrorKind::Unknown,
                RuntimeFailureKind::Unknown,
            ),
        ];
        for (variant, expected) in cases {
            let kind = variant.as_str();
            let actual = dispatch_kind_to_failure(kind);
            assert_eq!(
                actual, *expected,
                "dispatch kind {kind:?} should map to {expected:?}, got {actual:?}"
            );
        }
    }

    #[test]
    fn dispatch_kind_to_failure_pins_dispatch_error_top_level_strings() {
        // These strings come from `dispatch_error_kind` for non-runtime
        // DispatchError variants (UnknownCapability, UnknownProvider, ...).
        assert_eq!(
            dispatch_kind_to_failure("UnknownCapability"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("UnknownProvider"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("MissingRuntimeBackend"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("UnsupportedRuntime"),
            RuntimeFailureKind::MissingRuntime
        );
        assert_eq!(
            dispatch_kind_to_failure("RuntimeMismatch"),
            RuntimeFailureKind::Backend
        );
    }

    #[test]
    fn dispatch_kind_to_failure_unknown_strings_fall_back_to_unknown() {
        assert_eq!(
            dispatch_kind_to_failure("some_future_kind_name"),
            RuntimeFailureKind::Unknown
        );
        assert_eq!(dispatch_kind_to_failure(""), RuntimeFailureKind::Unknown);
    }

    #[test]
    fn failure_kind_from_dispatch_unknown_capability_maps_to_missing_runtime() {
        let error = dispatch("UnknownCapability");
        assert_eq!(
            failure_kind_from(&error),
            RuntimeFailureKind::MissingRuntime
        );
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
    fn sanitized_failure_message_redacts_dispatch_kind_to_stable_form() {
        let error = dispatch("NetworkDenied");
        let message = sanitized_failure_message(&error).expect("dispatch produces a message");
        // Stable form: relies only on the redacted kind token, never on raw
        // backend strings.
        assert!(
            message.contains("NetworkDenied"),
            "sanitized dispatch message should expose the redacted kind, got {message:?}"
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
        assert_eq!(RuntimeFailureKind::InvalidInput.as_str(), "invalid_input");
        assert_eq!(
            RuntimeFailureKind::MissingRuntime.as_str(),
            "missing_runtime"
        );
        assert_eq!(RuntimeFailureKind::Network.as_str(), "network");
        assert_eq!(
            RuntimeFailureKind::OutputTooLarge.as_str(),
            "output_too_large"
        );
        assert_eq!(RuntimeFailureKind::Process.as_str(), "process");
        assert_eq!(RuntimeFailureKind::Resource.as_str(), "resource");
        assert_eq!(RuntimeFailureKind::Unknown.as_str(), "unknown");
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

        let error = ProcessError::Filesystem("connection refused at /tmp/processes.db".to_string());
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
}
