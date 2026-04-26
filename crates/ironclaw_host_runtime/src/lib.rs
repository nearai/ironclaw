//! Host runtime composition helpers for IronClaw Reborn.
//!
//! This crate is intentionally composition-only. It wires existing host services
//! together without moving authorization, dispatch, process lifecycle, approval,
//! or run-state responsibilities out of their owning crates.

use std::{net::IpAddr, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::ApprovalResolver;
use ironclaw_authorization::{CapabilityDispatchAuthorizer, CapabilityLeaseStore};
use ironclaw_capabilities::{
    CapabilityHost, CapabilityObligationError, CapabilityObligationFailureKind,
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
    DispatchProcessExecutor,
};
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{AuditSink, EventSink};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityDispatcher, DecisionSummary, DispatchError, EffectKind, NetworkPolicy, Obligation,
    RuntimeDispatchErrorKind, RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_network::is_private_or_loopback_ip;
use ironclaw_processes::{
    ProcessExecutor, ProcessHost, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_wasm::{CapabilityInvocation, WasmError, WasmExecutionRequest, WasmRuntime};

/// Dispatcher adapter for the concrete WASM runtime crate.
pub struct WasmRuntimeAdapter {
    runtime: Arc<WasmRuntime>,
}

impl WasmRuntimeAdapter {
    pub fn new(runtime: Arc<WasmRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for WasmRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let execution = self
            .runtime
            .execute_extension_json(
                request.filesystem,
                request.governor,
                WasmExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: CapabilityInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(wasm_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Dispatcher adapter for the concrete script executor port.
pub struct ScriptRuntimeAdapter {
    runtime: Arc<dyn ScriptExecutor>,
}

impl ScriptRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        Self { runtime }
    }

    pub fn from_dyn(runtime: Arc<dyn ScriptExecutor>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for ScriptRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(script_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Dispatcher adapter for the concrete MCP executor port.
pub struct McpRuntimeAdapter {
    runtime: Arc<dyn McpExecutor>,
}

impl McpRuntimeAdapter {
    pub fn new<T>(runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        Self { runtime }
    }

    pub fn from_dyn(runtime: Arc<dyn McpExecutor>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for McpRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let execution = self
            .runtime
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(mcp_dispatch_error)?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

/// Built-in metadata-only obligation handler for the current host-runtime slice.
///
/// Supported obligations:
///
/// - `AuditBefore`: emits one metadata-only audit record and fails closed if no
///   audit sink is configured or emission fails.
/// - `ApplyNetworkPolicy`: validates policy metadata without performing I/O.
///
/// Runtime/input/output plumbing obligations remain unsupported and fail closed.
#[derive(Clone, Default)]
pub struct BuiltinObligationHandler {
    audit_sink: Option<Arc<dyn AuditSink>>,
}

impl BuiltinObligationHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_audit_sink_dyn(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    async fn emit_audit_before(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let Some(audit_sink) = &self.audit_sink else {
            return Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            });
        };

        audit_sink
            .emit_audit(audit_before_record(request))
            .await
            .map_err(|_| CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            })
    }
}

#[async_trait]
impl CapabilityObligationHandler for BuiltinObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let unsupported = unsupported_obligations(request.obligations);
        if !unsupported.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            });
        }

        for obligation in request.obligations {
            if let Obligation::ApplyNetworkPolicy { policy } = obligation {
                validate_network_policy_metadata(policy)?;
            }
        }

        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            self.emit_audit_before(&request).await?;
        }

        Ok(())
    }
}

fn unsupported_obligations(obligations: &[Obligation]) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| {
            !matches!(
                obligation,
                Obligation::AuditBefore | Obligation::ApplyNetworkPolicy { .. }
            )
        })
        .cloned()
        .collect()
}

fn validate_network_policy_metadata(
    policy: &NetworkPolicy,
) -> Result<(), CapabilityObligationError> {
    if policy.allowed_targets.is_empty() {
        return Err(network_obligation_failed());
    }

    if policy.deny_private_ip_ranges {
        for target in &policy.allowed_targets {
            let host = target
                .host_pattern
                .strip_prefix("*.")
                .unwrap_or(target.host_pattern.as_str());
            if let Ok(ip) = host.parse::<IpAddr>()
                && is_private_or_loopback_ip(ip)
            {
                return Err(network_obligation_failed());
            }
        }
    }

    Ok(())
}

fn network_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Network,
    }
}

fn audit_before_record(request: &CapabilityObligationRequest<'_>) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: request.context.correlation_id,
        stage: AuditStage::Before,
        timestamp: Utc::now(),
        tenant_id: request.context.tenant_id.clone(),
        user_id: request.context.user_id.clone(),
        project_id: request.context.project_id.clone(),
        mission_id: request.context.mission_id.clone(),
        thread_id: request.context.thread_id.clone(),
        invocation_id: request.context.invocation_id,
        process_id: request.context.process_id,
        approval_request_id: None,
        extension_id: Some(request.context.extension_id.clone()),
        action: ActionSummary {
            kind: capability_action_kind(request.phase).to_string(),
            target: Some(request.capability_id.as_str().to_string()),
            effects: capability_action_effects(request.phase),
        },
        decision: DecisionSummary {
            kind: "obligation_satisfied".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(obligation_status(request.obligations)),
            output_bytes: None,
        }),
    }
}

fn capability_action_kind(phase: CapabilityObligationPhase) -> &'static str {
    match phase {
        CapabilityObligationPhase::Invoke => "capability_invoke",
        CapabilityObligationPhase::Resume => "capability_resume",
        CapabilityObligationPhase::Spawn => "capability_spawn",
    }
}

fn capability_action_effects(phase: CapabilityObligationPhase) -> Vec<EffectKind> {
    match phase {
        CapabilityObligationPhase::Invoke | CapabilityObligationPhase::Resume => {
            vec![EffectKind::DispatchCapability]
        }
        CapabilityObligationPhase::Spawn => {
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess]
        }
    }
}

fn obligation_status(obligations: &[Obligation]) -> String {
    obligations
        .iter()
        .filter_map(obligation_label)
        .collect::<Vec<_>>()
        .join(",")
}

fn obligation_label(obligation: &Obligation) -> Option<&'static str> {
    match obligation {
        Obligation::AuditBefore => Some("audit_before"),
        Obligation::ApplyNetworkPolicy { .. } => Some("apply_network_policy"),
        _ => None,
    }
}

/// Composition root for the Reborn host/runtime vertical slice.
///
/// `HostRuntimeServices` owns shared service handles and can build the narrow
/// service facades used by callers:
///
/// - `RuntimeDispatcher` for already-authorized runtime dispatch
/// - `CapabilityHost` for caller-facing invocation/spawn workflows
/// - `ProcessHost` for lifecycle/result/output/cancellation operations
///
/// It is deliberately not an authority engine, dispatcher, process manager, or
/// lifecycle store. Those responsibilities remain in their owning crates.
pub struct HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    registry: Arc<ExtensionRegistry>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    authorizer: Arc<A>,
    process_services: ProcessServices<S, R>,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    wasm_runtime: Option<Arc<WasmRuntime>>,
    script_runtime: Option<Arc<dyn ScriptExecutor>>,
    mcp_runtime: Option<Arc<dyn McpExecutor>>,
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    obligation_handler: Option<Arc<dyn CapabilityObligationHandler>>,
}

impl<F, G, S, R, A> HostRuntimeServices<F, G, S, R, A>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
    A: CapabilityDispatchAuthorizer + 'static,
{
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        authorizer: Arc<A>,
        process_services: ProcessServices<S, R>,
    ) -> Self {
        Self {
            registry,
            filesystem,
            governor,
            authorizer,
            process_services,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            wasm_runtime: None,
            script_runtime: None,
            mcp_runtime: None,
            event_sink: None,
            audit_sink: None,
            obligation_handler: None,
        }
    }

    pub fn registry(&self) -> Arc<ExtensionRegistry> {
        Arc::clone(&self.registry)
    }

    pub fn filesystem(&self) -> Arc<F> {
        Arc::clone(&self.filesystem)
    }

    pub fn governor(&self) -> Arc<G> {
        Arc::clone(&self.governor)
    }

    pub fn authorizer(&self) -> Arc<A> {
        Arc::clone(&self.authorizer)
    }

    pub fn process_services(&self) -> &ProcessServices<S, R> {
        &self.process_services
    }

    pub fn process_host(&self) -> ProcessHost<'_> {
        self.process_services.host()
    }

    pub fn with_run_state<T>(mut self, run_state: Arc<T>) -> Self
    where
        T: RunStateStore + 'static,
    {
        self.run_state = Some(run_state);
        self
    }

    pub fn with_approval_requests<T>(mut self, approval_requests: Arc<T>) -> Self
    where
        T: ApprovalRequestStore + 'static,
    {
        self.approval_requests = Some(approval_requests);
        self
    }

    pub fn with_capability_leases<T>(mut self, capability_leases: Arc<T>) -> Self
    where
        T: CapabilityLeaseStore + 'static,
    {
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntime>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn with_script_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        let runtime: Arc<dyn ScriptExecutor> = runtime;
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        let runtime: Arc<dyn McpExecutor> = runtime;
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_event_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: EventSink + 'static,
    {
        let sink: Arc<dyn EventSink> = sink;
        self.event_sink = Some(sink);
        self
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_obligation_handler<T>(mut self, handler: Arc<T>) -> Self
    where
        T: CapabilityObligationHandler + 'static,
    {
        let handler: Arc<dyn CapabilityObligationHandler> = handler;
        self.obligation_handler = Some(handler);
        self
    }

    pub fn with_builtin_obligation_handler(mut self) -> Self {
        let handler = if let Some(audit_sink) = &self.audit_sink {
            BuiltinObligationHandler::new().with_audit_sink_dyn(Arc::clone(audit_sink))
        } else {
            BuiltinObligationHandler::new()
        };
        self.obligation_handler = Some(Arc::new(handler));
        self
    }

    pub fn approval_resolver(
        &self,
    ) -> Option<ApprovalResolver<'_, dyn ApprovalRequestStore, dyn CapabilityLeaseStore>> {
        let approval_requests = self.approval_requests.as_deref()?;
        let capability_leases = self.capability_leases.as_deref()?;
        let mut resolver = ApprovalResolver::new(approval_requests, capability_leases);
        if let Some(audit_sink) = &self.audit_sink {
            resolver = resolver.with_audit_sink(audit_sink.as_ref());
        }
        Some(resolver)
    }

    pub fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        );

        if let Some(runtime) = &self.wasm_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Wasm,
                Arc::new(WasmRuntimeAdapter::new(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Script,
                Arc::new(ScriptRuntimeAdapter::from_dyn(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Mcp,
                Arc::new(McpRuntimeAdapter::from_dyn(Arc::clone(runtime))),
            );
        }
        if let Some(sink) = &self.event_sink {
            dispatcher = dispatcher.with_event_sink_arc(Arc::clone(sink));
        }

        dispatcher
    }

    pub fn runtime_dispatcher_arc(&self) -> Arc<RuntimeDispatcher<'static, F, G>> {
        Arc::new(self.runtime_dispatcher())
    }

    pub fn capability_host<'a, D, E>(
        &'a self,
        dispatcher: &'a D,
        executor: Arc<E>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
        E: ProcessExecutor + 'static,
    {
        self.configure_capability_host(
            CapabilityHost::new(self.registry.as_ref(), dispatcher, self.authorizer.as_ref())
                .with_process_services(&self.process_services, executor),
        )
    }

    pub fn capability_host_for_runtime_dispatcher<'a>(
        &'a self,
        dispatcher: &'a Arc<RuntimeDispatcher<'static, F, G>>,
    ) -> CapabilityHost<'a, RuntimeDispatcher<'static, F, G>> {
        let executor = Arc::new(DispatchProcessExecutor::new(Arc::clone(dispatcher)));
        self.capability_host(dispatcher.as_ref(), executor)
    }

    fn configure_capability_host<'a, D>(
        &'a self,
        host: CapabilityHost<'a, D>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
    {
        let mut host = host;
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(obligation_handler) = &self.obligation_handler {
            host = host.with_obligation_handler(obligation_handler.as_ref());
        }
        host
    }
}

fn mcp_dispatch_error(error: McpError) -> DispatchError {
    DispatchError::Mcp {
        kind: mcp_error_kind(&error),
    }
}

fn script_dispatch_error(error: ScriptError) -> DispatchError {
    DispatchError::Script {
        kind: script_error_kind(&error),
    }
}

fn wasm_dispatch_error(error: WasmError) -> DispatchError {
    DispatchError::Wasm {
        kind: wasm_error_kind(&error),
    }
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::UnsupportedTransport { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        McpError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        McpError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
        McpError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        McpError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
    }
}

fn script_error_kind(error: &ScriptError) -> RuntimeDispatchErrorKind {
    match error {
        ScriptError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        ScriptError::Backend { .. } => RuntimeDispatchErrorKind::Backend,
        ScriptError::UnsupportedRunner { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        ScriptError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        ScriptError::DescriptorMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        ScriptError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        ScriptError::ExitFailure { .. } => RuntimeDispatchErrorKind::ExitFailure,
        ScriptError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

fn wasm_error_kind(error: &WasmError) -> RuntimeDispatchErrorKind {
    match error {
        WasmError::Engine { .. } | WasmError::Cache { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::Extension(_) => RuntimeDispatchErrorKind::Manifest,
        WasmError::Filesystem(_) => RuntimeDispatchErrorKind::FilesystemDenied,
        WasmError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        WasmError::InvalidModule { .. } => RuntimeDispatchErrorKind::Manifest,
        WasmError::UnsupportedImport { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::ExtensionRuntimeMismatch,
        WasmError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        WasmError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        WasmError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        WasmError::MissingReservation => RuntimeDispatchErrorKind::Resource,
        WasmError::MissingExport { .. } => RuntimeDispatchErrorKind::Executor,
        WasmError::MissingMemory => RuntimeDispatchErrorKind::Memory,
        WasmError::GuestAllocation { .. } => RuntimeDispatchErrorKind::Memory,
        WasmError::GuestError { .. } => RuntimeDispatchErrorKind::Guest,
        WasmError::InvalidGuestOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
        WasmError::FuelExhausted { .. } => RuntimeDispatchErrorKind::Resource,
        WasmError::MemoryExceeded { .. } => RuntimeDispatchErrorKind::Memory,
        WasmError::Timeout { .. } => RuntimeDispatchErrorKind::Resource,
        WasmError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        WasmError::Trap { .. } => RuntimeDispatchErrorKind::Guest,
    }
}
