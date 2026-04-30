//! Concrete service graph for the Reborn [`HostRuntime`](crate::HostRuntime).
//!
//! This module is intentionally composition-only. It wires the owning Reborn
//! service crates together, adapts Script/MCP/WASM runtimes into the neutral
//! dispatcher port, and hands upper services a single [`DefaultHostRuntime`]
//! facade. Authorization, run-state transitions, approval leases, process
//! lifecycle, and runtime execution semantics remain in their owning crates.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_approvals::ApprovalResolver;
use ironclaw_authorization::{CapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_capabilities::CapabilityHost;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{AuditSink, EventSink};
use ironclaw_extensions::{ExtensionRegistry, ExtensionRuntime};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatcher, DispatchError, ResourceReservationId,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_processes::{
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
    ProcessHost, ProcessManager, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_wasm::{
    PreparedWitTool, WasmError, WitToolHost, WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
};

use crate::{CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntimeError, RuntimeBackendHealth};

/// Concrete composition bundle for one Reborn host-runtime vertical slice.
///
/// The bundle owns shared `Arc` handles for the configured substrate services
/// and can build the narrow caller-facing [`DefaultHostRuntime`] facade. Lower
/// handles are available for setup/tests inside the host-runtime layer, but
/// product/upper Reborn code should prefer [`Self::host_runtime`] and depend on
/// `Arc<dyn crate::HostRuntime>` instead of reaching around the facade.
pub struct HostRuntimeServices<F, G, S, R>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    registry: Arc<ExtensionRegistry>,
    filesystem: Arc<F>,
    governor: Arc<G>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    process_services: ProcessServices<S, R>,
    surface_version: CapabilitySurfaceVersion,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    runtime_health: Option<Arc<dyn RuntimeBackendHealth>>,
    script_runtime: Option<Arc<dyn ScriptExecutor>>,
    mcp_runtime: Option<Arc<dyn McpExecutor>>,
    wasm_runtime: Option<Arc<WasmRuntimeAdapter>>,
}

impl<F, G, S, R> HostRuntimeServices<F, G, S, R>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
    pub fn new(
        registry: Arc<ExtensionRegistry>,
        filesystem: Arc<F>,
        governor: Arc<G>,
        authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
        process_services: ProcessServices<S, R>,
        surface_version: CapabilitySurfaceVersion,
    ) -> Self {
        Self {
            registry,
            filesystem,
            governor,
            authorizer,
            process_services,
            surface_version,
            run_state: None,
            approval_requests: None,
            capability_leases: None,
            event_sink: None,
            audit_sink: None,
            runtime_health: None,
            script_runtime: None,
            mcp_runtime: None,
            wasm_runtime: None,
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

    pub fn authorizer(&self) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
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

    pub fn with_event_sink<T>(mut self, event_sink: Arc<T>) -> Self
    where
        T: EventSink + 'static,
    {
        self.event_sink = Some(event_sink);
        self
    }

    pub fn with_audit_sink<T>(mut self, audit_sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        self.audit_sink = Some(audit_sink);
        self
    }

    pub fn with_runtime_health<T>(mut self, runtime_health: Arc<T>) -> Self
    where
        T: RuntimeBackendHealth + 'static,
    {
        self.runtime_health = Some(runtime_health);
        self
    }

    pub fn with_script_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntimeAdapter>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn try_with_wasm_runtime(
        self,
        config: WitToolRuntimeConfig,
        host: WitToolHost,
    ) -> Result<Self, WasmError> {
        let adapter = Arc::new(WasmRuntimeAdapter::try_new(config, host)?);
        Ok(self.with_wasm_runtime(adapter))
    }

    /// Builds a runtime dispatcher with every configured runtime adapter.
    pub fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        );

        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Script,
                Arc::new(ScriptRuntimeAdapter::from_executor(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Mcp,
                Arc::new(McpRuntimeAdapter::from_executor(Arc::clone(runtime))),
            );
        }
        if let Some(runtime) = &self.wasm_runtime {
            dispatcher =
                dispatcher.with_runtime_adapter_arc(RuntimeKind::Wasm, Arc::clone(runtime));
        }
        if let Some(event_sink) = &self.event_sink {
            dispatcher = dispatcher.with_event_sink_arc(Arc::clone(event_sink));
        }

        dispatcher
    }

    pub fn runtime_dispatcher_arc(&self) -> Arc<RuntimeDispatcher<'static, F, G>> {
        Arc::new(self.runtime_dispatcher())
    }

    /// Builds the upper facade with the same dispatcher, process services,
    /// stores, cancellation registry, result store, and runtime health graph.
    pub fn host_runtime(&self) -> DefaultHostRuntime {
        let dispatcher: Arc<dyn CapabilityDispatcher> = self.runtime_dispatcher_arc();
        let process_executor =
            Arc::new(RuntimeDispatchProcessExecutor::new(Arc::clone(&dispatcher)));
        let process_manager: Arc<dyn ProcessManager> =
            Arc::new(self.process_services.background_manager(process_executor));
        let process_store: Arc<dyn ProcessStore> = self.process_services.process_store();
        let process_result_store: Arc<dyn ProcessResultStore> =
            self.process_services.result_store();
        let runtime_health = self.runtime_health.clone().unwrap_or_else(|| {
            Arc::new(RegisteredRuntimeHealth::new(
                self.registered_runtime_backends(),
            ))
        });

        let mut runtime = DefaultHostRuntime::new(
            Arc::clone(&self.registry),
            dispatcher,
            Arc::clone(&self.authorizer),
            self.surface_version.clone(),
        )
        .with_process_manager(process_manager)
        .with_process_store(process_store)
        .with_process_result_store(process_result_store)
        .with_process_cancellation_registry(self.process_services.cancellation_registry())
        .with_runtime_health(runtime_health);

        if let Some(run_state) = &self.run_state {
            runtime = runtime.with_run_state(Arc::clone(run_state));
        }
        if let Some(approval_requests) = &self.approval_requests {
            runtime = runtime.with_approval_requests(Arc::clone(approval_requests));
        }
        if let Some(capability_leases) = &self.capability_leases {
            runtime = runtime.with_capability_leases(Arc::clone(capability_leases));
        }

        runtime
    }

    /// Builds an approval resolver over the same approval and lease stores used
    /// by the capability host resume paths. Returns `None` until both stores are
    /// configured, which keeps approval resolution fail-closed at composition.
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

    pub fn capability_host<'a, D>(
        &'a self,
        dispatcher: &'a D,
        process_manager: Option<&'a dyn ProcessManager>,
    ) -> CapabilityHost<'a, D>
    where
        D: CapabilityDispatcher + ?Sized,
    {
        let mut host =
            CapabilityHost::new(self.registry.as_ref(), dispatcher, self.authorizer.as_ref());
        if let Some(run_state) = &self.run_state {
            host = host.with_run_state(run_state.as_ref());
        }
        if let Some(approval_requests) = &self.approval_requests {
            host = host.with_approval_requests(approval_requests.as_ref());
        }
        if let Some(capability_leases) = &self.capability_leases {
            host = host.with_capability_leases(capability_leases.as_ref());
        }
        if let Some(process_manager) = process_manager {
            host = host.with_process_manager(process_manager);
        }
        host
    }

    fn registered_runtime_backends(&self) -> Vec<RuntimeKind> {
        let mut backends = Vec::new();
        if self.wasm_runtime.is_some() {
            backends.push(RuntimeKind::Wasm);
        }
        if self.mcp_runtime.is_some() {
            backends.push(RuntimeKind::Mcp);
        }
        if self.script_runtime.is_some() {
            backends.push(RuntimeKind::Script);
        }
        backends
    }
}

#[derive(Debug, Clone)]
pub struct RegisteredRuntimeHealth {
    available: Vec<RuntimeKind>,
}

impl RegisteredRuntimeHealth {
    pub fn new(available: impl IntoIterator<Item = RuntimeKind>) -> Self {
        let mut available = available.into_iter().collect::<Vec<_>>();
        normalize_runtime_kinds(&mut available);
        Self { available }
    }
}

#[async_trait]
impl RuntimeBackendHealth for RegisteredRuntimeHealth {
    async fn missing_runtime_backends(
        &self,
        required: &[RuntimeKind],
    ) -> Result<Vec<RuntimeKind>, HostRuntimeError> {
        let mut missing = required
            .iter()
            .copied()
            .filter(|runtime| !self.available.contains(runtime))
            .collect::<Vec<_>>();
        normalize_runtime_kinds(&mut missing);
        Ok(missing)
    }
}

#[derive(Clone)]
pub struct ScriptRuntimeAdapter {
    executor: Arc<dyn ScriptExecutor>,
}

impl ScriptRuntimeAdapter {
    pub fn from_executor(executor: Arc<dyn ScriptExecutor>) -> Self {
        Self { executor }
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
            .executor
            .execute_extension_json(
                request.governor,
                ScriptExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    resource_reservation: request.resource_reservation,
                    invocation: ScriptInvocation {
                        input: request.input,
                    },
                },
            )
            .map_err(|error| DispatchError::Script {
                kind: script_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

#[derive(Clone)]
pub struct McpRuntimeAdapter {
    executor: Arc<dyn McpExecutor>,
}

impl McpRuntimeAdapter {
    pub fn from_executor(executor: Arc<dyn McpExecutor>) -> Self {
        Self { executor }
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
            .executor
            .execute_extension_json(
                request.governor,
                McpExecutionRequest {
                    package: request.package,
                    capability_id: request.capability_id,
                    scope: request.scope,
                    estimate: request.estimate,
                    resource_reservation: request.resource_reservation,
                    invocation: McpInvocation {
                        input: request.input,
                    },
                },
            )
            .await
            .map_err(|error| DispatchError::Mcp {
                kind: mcp_error_kind(&error),
            })?;

        Ok(RuntimeAdapterResult {
            output: execution.result.output,
            usage: execution.result.usage,
            receipt: execution.receipt,
            output_bytes: execution.result.output_bytes,
        })
    }
}

pub struct WasmRuntimeAdapter {
    runtime: WitToolRuntime,
    host: WitToolHost,
    prepared: Mutex<HashMap<String, Arc<PreparedWitTool>>>,
}

impl WasmRuntimeAdapter {
    pub fn new(runtime: WitToolRuntime, host: WitToolHost) -> Self {
        Self {
            runtime,
            host,
            prepared: Mutex::new(HashMap::new()),
        }
    }

    pub fn try_new(config: WitToolRuntimeConfig, host: WitToolHost) -> Result<Self, WasmError> {
        Ok(Self::new(WitToolRuntime::new(config)?, host))
    }

    fn prepared_guard(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, Arc<PreparedWitTool>>>, DispatchError> {
        self.prepared.lock().map_err(|_| DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Executor,
        })
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
        let module_path = match &request.package.manifest.runtime {
            ExtensionRuntime::Wasm { module } => module
                .resolve_under(&request.package.root)
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Manifest,
                })?,
            other => {
                return Err(DispatchError::Wasm {
                    kind: if other.kind() == RuntimeKind::Wasm {
                        RuntimeDispatchErrorKind::Manifest
                    } else {
                        RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
                    },
                });
            }
        };
        let cache_key = format!(
            "{}:{}",
            request.capability_id.as_str(),
            module_path.as_str()
        );
        if let Some(prepared) = self.prepared_guard()?.get(&cache_key).cloned() {
            return execute_prepared_wasm(&self.runtime, &prepared, self.host.clone(), request);
        }

        let wasm_bytes = request
            .filesystem
            .read_file(&module_path)
            .await
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::FilesystemDenied,
            })?;
        let prepared = Arc::new(
            self.runtime
                .prepare(request.capability_id.as_str(), &wasm_bytes)
                .map_err(|error| DispatchError::Wasm {
                    kind: wasm_error_kind(&error),
                })?,
        );
        let prepared = {
            let mut prepared_cache = self.prepared_guard()?;
            if let Some(existing) = prepared_cache.get(&cache_key).cloned() {
                existing
            } else {
                prepared_cache.insert(cache_key, Arc::clone(&prepared));
                prepared
            }
        };
        execute_prepared_wasm(&self.runtime, &prepared, self.host.clone(), request)
    }
}

#[derive(Clone)]
pub struct RuntimeDispatchProcessExecutor {
    dispatcher: Arc<dyn CapabilityDispatcher>,
}

impl RuntimeDispatchProcessExecutor {
    pub fn new(dispatcher: Arc<dyn CapabilityDispatcher>) -> Self {
        Self { dispatcher }
    }
}

#[async_trait]
impl ProcessExecutor for RuntimeDispatchProcessExecutor {
    async fn execute(
        &self,
        request: ProcessExecutionRequest,
    ) -> Result<ProcessExecutionResult, ProcessExecutionError> {
        if request.cancellation.is_cancelled() {
            return Err(ProcessExecutionError::new("cancelled"));
        }
        let result = self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: request.scope,
                estimate: request.estimate,
                mounts: None,
                resource_reservation: None,
                input: request.input,
            })
            .await
            .map_err(|error| ProcessExecutionError::new(dispatch_error_kind(&error)))?;
        if request.cancellation.is_cancelled() {
            return Err(ProcessExecutionError::new("cancelled"));
        }
        Ok(ProcessExecutionResult {
            output: result.output,
        })
    }
}

fn execute_prepared_wasm<G>(
    runtime: &WitToolRuntime,
    prepared: &PreparedWitTool,
    host: WitToolHost,
    request: RuntimeAdapterRequest<'_, impl RootFilesystem, G>,
) -> Result<RuntimeAdapterResult, DispatchError>
where
    G: ResourceGovernor,
{
    let input_json = serde_json::to_string(&request.input).map_err(|_| DispatchError::Wasm {
        kind: RuntimeDispatchErrorKind::InputEncode,
    })?;
    let reservation = match request.resource_reservation {
        Some(reservation) => reservation,
        None => request
            .governor
            .reserve(request.scope.clone(), request.estimate.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?,
    };
    let execution = match runtime.execute(prepared, host, WitToolRequest::new(input_json)) {
        Ok(execution) => execution,
        Err(error) => {
            if let Some(usage) = preserved_wasm_error_usage(&error) {
                if request.governor.reconcile(reservation.id, usage).is_err() {
                    release_wasm_reservation(request.governor, reservation.id);
                    return Err(DispatchError::Wasm {
                        kind: RuntimeDispatchErrorKind::Resource,
                    });
                }
            } else {
                release_wasm_reservation(request.governor, reservation.id);
            }
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            });
        }
    };
    if execution.error.is_some() {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        });
    }
    let Some(output_json) = execution.output_json else {
        release_wasm_reservation(request.governor, reservation.id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
        });
    };
    let output = match serde_json::from_str(&output_json) {
        Ok(output) => output,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            });
        }
    };
    let receipt = match request
        .governor
        .reconcile(reservation.id, execution.usage.clone())
    {
        Ok(receipt) => receipt,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            });
        }
    };
    Ok(RuntimeAdapterResult {
        output,
        output_bytes: execution.usage.output_bytes,
        usage: execution.usage,
        receipt,
    })
}

fn release_wasm_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
}

fn preserved_wasm_error_usage(error: &WasmError) -> Option<ResourceUsage> {
    if let WasmError::ExecutionFailed { usage, .. } = error
        && has_accountable_effects(usage)
    {
        Some(usage.clone())
    } else {
        None
    }
}

fn has_accountable_effects(usage: &ResourceUsage) -> bool {
    usage.usd != Default::default()
        || usage.input_tokens > 0
        || usage.output_tokens > 0
        || usage.output_bytes > 0
        || usage.network_egress_bytes > 0
        || usage.process_count > 0
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
        ScriptError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        ScriptError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        ScriptError::ExitFailure { .. } => RuntimeDispatchErrorKind::ExitFailure,
        ScriptError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
        ScriptError::Timeout { .. } => RuntimeDispatchErrorKind::Executor,
        ScriptError::InvalidOutput { .. } => RuntimeDispatchErrorKind::OutputDecode,
    }
}

fn mcp_error_kind(error: &McpError) -> RuntimeDispatchErrorKind {
    match error {
        McpError::Resource(_) => RuntimeDispatchErrorKind::Resource,
        McpError::Client { .. } => RuntimeDispatchErrorKind::Client,
        McpError::UnsupportedTransport { .. } => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::HostHttpEgressRequired { .. } => RuntimeDispatchErrorKind::NetworkDenied,
        McpError::ExternalStdioTransportUnsupported => RuntimeDispatchErrorKind::UnsupportedRunner,
        McpError::ExtensionRuntimeMismatch { .. } => {
            RuntimeDispatchErrorKind::ExtensionRuntimeMismatch
        }
        McpError::CapabilityNotDeclared { .. } => RuntimeDispatchErrorKind::UndeclaredCapability,
        McpError::DescriptorMismatch { .. } => RuntimeDispatchErrorKind::Manifest,
        McpError::InvalidInvocation { .. } => RuntimeDispatchErrorKind::InputEncode,
        McpError::OutputLimitExceeded { .. } => RuntimeDispatchErrorKind::OutputTooLarge,
    }
}

fn wasm_error_kind(error: &WasmError) -> RuntimeDispatchErrorKind {
    match error {
        WasmError::EngineCreationFailed(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::CompilationFailed(_) => RuntimeDispatchErrorKind::Manifest,
        WasmError::StoreConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::LinkerConfiguration(_) => RuntimeDispatchErrorKind::Executor,
        WasmError::InstantiationFailed(_) => RuntimeDispatchErrorKind::MethodMissing,
        WasmError::ExecutionFailed { .. } => RuntimeDispatchErrorKind::Guest,
        WasmError::InvalidSchema(_) => RuntimeDispatchErrorKind::Manifest,
    }
}

fn dispatch_error_kind(error: &DispatchError) -> &'static str {
    match error {
        DispatchError::UnknownCapability { .. } => "unknown_capability",
        DispatchError::UnknownProvider { .. } => "unknown_provider",
        DispatchError::RuntimeMismatch { .. } => "runtime_mismatch",
        DispatchError::MissingRuntimeBackend { .. } => "missing_runtime_backend",
        DispatchError::UnsupportedRuntime { .. } => "unsupported_runtime",
        DispatchError::Mcp { kind }
        | DispatchError::Script { kind }
        | DispatchError::Wasm { kind } => kind.event_kind(),
    }
}

fn normalize_runtime_kinds(kinds: &mut Vec<RuntimeKind>) {
    kinds.sort_by_key(|kind| runtime_sort_key(*kind));
    kinds.dedup();
}

fn runtime_sort_key(kind: RuntimeKind) -> u8 {
    match kind {
        RuntimeKind::Wasm => 0,
        RuntimeKind::Mcp => 1,
        RuntimeKind::Script => 2,
        RuntimeKind::FirstParty => 3,
        RuntimeKind::System => 4,
    }
}
