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
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventSink,
};
use ironclaw_extensions::{ExtensionRegistry, ExtensionRuntime};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatcher, CapabilityId, DispatchError,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind,
    RuntimeHttpEgress, RuntimeKind,
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_processes::{
    BackgroundFailureStage, ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult,
    ProcessExecutor, ProcessManager, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_run_state::{ApprovalRequestStore, RunStateStore};
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_secrets::SecretStore;
use ironclaw_trust::{HostTrustPolicy, TrustPolicy};
use ironclaw_wasm::{
    DenyWasmHostHttp, PreparedWitTool, WasmError, WasmRuntimeCredentialProvider,
    WasmRuntimeHttpAdapter, WitToolHost, WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
};

use crate::{
    BuiltinObligationHandler, CapabilitySurfaceVersion, DefaultHostRuntime, HostRuntimeError,
    NetworkObligationPolicyStore, ProcessObligationLifecycleStore, RuntimeBackendHealth,
    RuntimeSecretInjectionStore,
};

type SharedRuntimeHttpEgress = Arc<Mutex<Option<Arc<dyn RuntimeHttpEgress>>>>;

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
    trust_policy: Arc<dyn TrustPolicy>,
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
    secret_store: Option<Arc<dyn SecretStore>>,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    secret_injection_store: Arc<RuntimeSecretInjectionStore>,
    process_lifecycle_store: Arc<ProcessObligationLifecycleStore>,
    runtime_http_egress: SharedRuntimeHttpEgress,
    wasm_credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
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
        let network_policy_store = Arc::new(NetworkObligationPolicyStore::new());
        let secret_injection_store = Arc::new(RuntimeSecretInjectionStore::new());
        let process_lifecycle_store = Arc::new(ProcessObligationLifecycleStore::new(
            process_services.process_store(),
            Arc::clone(&network_policy_store),
            Arc::clone(&secret_injection_store),
            governor.clone(),
        ));
        Self {
            registry,
            trust_policy: Arc::new(HostTrustPolicy::empty()),
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
            secret_store: None,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress: Arc::new(Mutex::new(None)),
            wasm_credential_provider: None,
            runtime_health: None,
            script_runtime: None,
            mcp_runtime: None,
            wasm_runtime: None,
        }
    }

    /// Attaches the host-owned trust policy used by the produced
    /// [`DefaultHostRuntime`]. Without this, the service graph keeps the
    /// default empty policy and capability dispatch fails closed.
    pub fn with_trust_policy<T>(mut self, trust_policy: Arc<T>) -> Self
    where
        T: TrustPolicy + 'static,
    {
        self.trust_policy = trust_policy;
        self
    }

    pub fn with_trust_policy_dyn(mut self, trust_policy: Arc<dyn TrustPolicy>) -> Self {
        self.trust_policy = trust_policy;
        self
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

    pub fn with_durable_event_log<T>(self, event_log: Arc<T>) -> Self
    where
        T: DurableEventLog + 'static,
    {
        let event_log: Arc<dyn DurableEventLog> = event_log;
        self.with_event_sink(Arc::new(DurableEventSink::new(event_log)))
    }

    pub fn with_audit_sink<T>(mut self, audit_sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        self.audit_sink = Some(audit_sink);
        self
    }

    pub fn with_durable_audit_log<T>(self, audit_log: Arc<T>) -> Self
    where
        T: DurableAuditLog + 'static,
    {
        let audit_log: Arc<dyn DurableAuditLog> = audit_log;
        self.with_audit_sink(Arc::new(DurableAuditSink::new(audit_log)))
    }

    pub fn with_secret_store<T>(mut self, secret_store: Arc<T>) -> Self
    where
        T: SecretStore + 'static,
    {
        self.secret_store = Some(secret_store);
        self
    }

    pub fn with_runtime_http_egress<T>(self, runtime_http_egress: Arc<T>) -> Self
    where
        T: RuntimeHttpEgress + 'static,
    {
        let runtime_http_egress: Arc<dyn RuntimeHttpEgress> = runtime_http_egress;
        set_runtime_http_egress(&self.runtime_http_egress, runtime_http_egress);
        self
    }

    pub fn with_runtime_health<T>(mut self, runtime_health: Arc<T>) -> Self
    where
        T: RuntimeBackendHealth + 'static,
    {
        self.runtime_health = Some(runtime_health);
        self
    }

    pub fn with_wasm_runtime_credential_provider<T>(mut self, provider: Arc<T>) -> Self
    where
        T: WasmRuntimeCredentialProvider + 'static,
    {
        let provider: Arc<dyn WasmRuntimeCredentialProvider> = provider;
        self.wasm_credential_provider = Some(provider);
        self
    }

    pub fn secret_injection_store(&self) -> Arc<RuntimeSecretInjectionStore> {
        Arc::clone(&self.secret_injection_store)
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

    fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntimeAdapter>) -> Self {
        self.wasm_runtime = Some(runtime);
        self
    }

    pub fn try_with_wasm_runtime(
        self,
        config: WitToolRuntimeConfig,
        host: WitToolHost,
    ) -> Result<Self, WasmError> {
        let adapter = Arc::new(WasmRuntimeAdapter::try_new(
            config,
            host,
            Arc::clone(&self.network_policy_store),
            Arc::clone(&self.runtime_http_egress),
            self.wasm_credential_provider.clone(),
        )?);
        Ok(self.with_wasm_runtime(adapter))
    }

    /// Builds a runtime dispatcher with every configured runtime adapter.
    fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
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

    /// Builds the upper facade with the same dispatcher, process services,
    /// stores, cancellation registry, result store, and runtime health graph.
    pub fn host_runtime(&self) -> DefaultHostRuntime {
        let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(self.runtime_dispatcher());
        let process_executor =
            Arc::new(RuntimeDispatchProcessExecutor::new(Arc::clone(&dispatcher)));
        let lifecycle_process_store = Arc::clone(&self.process_lifecycle_store);
        let process_store: Arc<dyn ProcessStore> = lifecycle_process_store.clone();
        let result_failure_cleanup_store = Arc::clone(&lifecycle_process_store);
        let process_manager: Arc<dyn ProcessManager> = Arc::new(
            ironclaw_processes::BackgroundProcessManager::new(
                lifecycle_process_store,
                process_executor,
            )
            .with_cancellation_registry(self.process_services.cancellation_registry())
            .with_result_store(self.process_services.result_store())
            .with_error_handler(move |failure| {
                let reconcile = match failure.stage {
                    BackgroundFailureStage::StoreComplete => true,
                    BackgroundFailureStage::StoreFail => false,
                    BackgroundFailureStage::ResultStoreComplete => true,
                    BackgroundFailureStage::ResultStoreFail => false,
                    _ => return,
                };
                let cleanup_store = Arc::clone(&result_failure_cleanup_store);
                tokio::spawn(async move {
                    if let Err(error) = cleanup_store
                        .cleanup_process_obligations(&failure.scope, failure.process_id, reconcile)
                        .await
                    {
                        tracing::warn!(
                            process_id = %failure.process_id,
                            stage = ?failure.stage,
                            error = %error,
                            "background process obligation cleanup failed"
                        );
                    }
                });
            }),
        );
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
        .with_trust_policy_dyn(Arc::clone(&self.trust_policy))
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

        runtime.with_obligation_handler(Arc::new(self.builtin_obligation_handler()))
    }

    fn builtin_obligation_handler(&self) -> BuiltinObligationHandler {
        let governor: Arc<dyn ResourceGovernor> = self.governor.clone();
        let mut handler = BuiltinObligationHandler::new()
            .with_network_policy_store(Arc::clone(&self.network_policy_store))
            .with_secret_injection_store(Arc::clone(&self.secret_injection_store))
            .with_resource_governor_dyn(governor);

        if let Some(audit_sink) = &self.audit_sink {
            handler = handler.with_audit_sink_dyn(Arc::clone(audit_sink));
        }
        if let Some(secret_store) = &self.secret_store {
            handler = handler.with_secret_store_dyn(Arc::clone(secret_store));
        }

        handler
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

fn set_runtime_http_egress(
    slot: &SharedRuntimeHttpEgress,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
) {
    match slot.lock() {
        Ok(mut guard) => {
            *guard = Some(runtime_http_egress);
        }
        Err(poisoned) => {
            *poisoned.into_inner() = Some(runtime_http_egress);
        }
    }
}

fn runtime_http_egress(slot: &SharedRuntimeHttpEgress) -> Option<Arc<dyn RuntimeHttpEgress>> {
    match slot.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
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
struct ScriptRuntimeAdapter {
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
                    mounts: request.mounts,
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
struct McpRuntimeAdapter {
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

struct WasmRuntimeAdapter {
    runtime: WitToolRuntime,
    host: WitToolHost,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    runtime_http_egress: SharedRuntimeHttpEgress,
    credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    prepared: Mutex<HashMap<String, Arc<PreparedWitTool>>>,
}

impl WasmRuntimeAdapter {
    pub fn new(
        runtime: WitToolRuntime,
        host: WitToolHost,
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        runtime_http_egress: SharedRuntimeHttpEgress,
        credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    ) -> Self {
        Self {
            runtime,
            host,
            network_policy_store,
            runtime_http_egress,
            credential_provider,
            prepared: Mutex::new(HashMap::new()),
        }
    }

    pub fn try_new(
        config: WitToolRuntimeConfig,
        host: WitToolHost,
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        runtime_http_egress: SharedRuntimeHttpEgress,
        credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    ) -> Result<Self, WasmError> {
        Ok(Self::new(
            WitToolRuntime::new(config)?,
            host,
            network_policy_store,
            runtime_http_egress,
            credential_provider,
        ))
    }

    fn prepared_guard(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, Arc<PreparedWitTool>>>, DispatchError> {
        self.prepared.lock().map_err(|_| DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Executor,
        })
    }

    fn host_for_scope(&self, scope: &ResourceScope, capability_id: &CapabilityId) -> WitToolHost {
        let egress = runtime_http_egress(&self.runtime_http_egress);
        let Some(policy) = self.network_policy_store.take(scope, capability_id) else {
            return if egress.is_some() {
                self.host.clone().with_http(Arc::new(DenyWasmHostHttp))
            } else {
                self.host.clone()
            };
        };
        let Some(egress) = egress else {
            return self.host.clone().with_http(Arc::new(DenyWasmHostHttp));
        };
        let mut adapter =
            WasmRuntimeHttpAdapter::new(egress, scope.clone(), capability_id.clone(), policy);
        if let Some(provider) = &self.credential_provider {
            adapter = adapter.with_credential_provider(Arc::clone(provider));
        }
        self.host.clone().with_http(Arc::new(adapter))
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
        let prepared = self.prepared_guard()?.get(&cache_key).cloned();
        if let Some(prepared) = prepared {
            let host = self.host_for_scope(&request.scope, request.capability_id);
            return execute_prepared_wasm(&self.runtime, &prepared, host, request);
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
        let host = self.host_for_scope(&request.scope, request.capability_id);
        execute_prepared_wasm(&self.runtime, &prepared, host, request)
    }
}

#[derive(Clone)]
struct RuntimeDispatchProcessExecutor {
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
    let reservation = match request.resource_reservation {
        Some(reservation) => reservation,
        None => request
            .governor
            .reserve(request.scope.clone(), request.estimate.clone())
            .map_err(|_| DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::Resource,
            })?,
    };
    let input_json = match serde_json::to_string(&request.input) {
        Ok(json) => json,
        Err(_) => {
            release_wasm_reservation(request.governor, reservation.id);
            return Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::InputEncode,
            });
        }
    };
    let execution = match runtime.execute(prepared, host, WitToolRequest::new(input_json)) {
        Ok(execution) => execution,
        Err(error) => {
            if let Some(usage) = preserved_wasm_error_usage(&error) {
                account_or_release_failed_wasm_execution(request.governor, reservation.id, &usage)?;
            } else {
                release_wasm_reservation(request.governor, reservation.id);
            }
            return Err(DispatchError::Wasm {
                kind: wasm_error_kind(&error),
            });
        }
    };
    if execution.error.is_some() {
        account_or_release_failed_wasm_execution(
            request.governor,
            reservation.id,
            &execution.usage,
        )?;
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        });
    }
    let Some(output_json) = execution.output_json else {
        account_or_release_failed_wasm_execution(
            request.governor,
            reservation.id,
            &execution.usage,
        )?;
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::InvalidResult,
        });
    };
    let output = match serde_json::from_str(&output_json) {
        Ok(output) => output,
        Err(_) => {
            account_or_release_failed_wasm_execution(
                request.governor,
                reservation.id,
                &execution.usage,
            )?;
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

fn account_or_release_failed_wasm_execution<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    usage: &ResourceUsage,
) -> Result<(), DispatchError>
where
    G: ResourceGovernor + ?Sized,
{
    if !has_accountable_effects(usage) {
        release_wasm_reservation(governor, reservation_id);
        return Ok(());
    }

    if governor.reconcile(reservation_id, usage.clone()).is_err() {
        release_wasm_reservation(governor, reservation_id);
        return Err(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Resource,
        });
    }

    Ok(())
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
        || usage.wall_clock_ms > 0
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
