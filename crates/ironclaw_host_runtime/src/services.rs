//! Concrete service graph for the Reborn [`HostRuntime`](crate::HostRuntime).
//!
//! This module is intentionally composition-only. It wires the owning Reborn
//! service crates together, adapts Script/MCP/WASM runtimes into the neutral
//! dispatcher port, and hands upper services a single [`DefaultHostRuntime`]
//! facade. Authorization, run-state transitions, approval leases, process
//! lifecycle, and runtime execution semantics remain in their owning crates.

mod process_executor;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_approvals::{
    ApprovalResolver, InMemoryPersistentApprovalPolicyStore, PersistentApprovalPolicyStore,
};
use ironclaw_authorization::{
    CapabilityLeaseStore, InMemoryCapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_capabilities::CapabilityObligationHandler;
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventSink,
    InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink,
    SecurityAuditSink,
};
use ironclaw_extensions::{ExtensionRegistry, ExtensionRuntime, SharedExtensionRegistry};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityDispatcher, CapabilityId, DispatchError, ResourceReservationId, ResourceScope,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeKind, SecretHandle,
    runtime_policy::{
        DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode,
        ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_network::NetworkHttpEgress;
use ironclaw_processes::{
    BackgroundFailureStage, InMemoryProcessResultStore, InMemoryProcessStore, ProcessExecutor,
    ProcessManager, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_reborn_event_store::{
    CoalescingEventSink, EventBatchConfig, RebornEventStoreConfig, RebornEventStoreError,
    RebornEventStores, RebornProfile, build_reborn_event_stores,
};
use ironclaw_resources::{
    FilesystemResourceGovernorStore, InMemoryResourceGovernor, PersistentResourceGovernor,
    ResourceGovernor,
};
use ironclaw_run_state::{
    ApprovalRequestStore, FilesystemApprovalRequestStore, FilesystemRunStateStore,
    InMemoryApprovalRequestStore, InMemoryRunStateStore, RunStateApprovalStore, RunStateStore,
};
use ironclaw_scripts::{ScriptError, ScriptExecutionRequest, ScriptExecutor, ScriptInvocation};
use ironclaw_secrets::{
    CredentialAccountStore, CredentialSessionStore, InMemoryCredentialBroker, InMemorySecretStore,
    SecretStore, SecretStoreError,
};
use ironclaw_trust::{HostTrustPolicy, TrustPolicy};
use ironclaw_turns::{
    DefaultTurnCoordinator, FilesystemTurnStateStore, InMemoryTurnStateStore,
    NoopTurnRunWakeNotifier, RunProfileResolver, TurnRunWakeNotifier, TurnStateStore,
    runner::TurnRunTransitionPort,
};
use ironclaw_wasm::{
    DenyWasmHostHttp, EmptyWasmRuntimeCredentials, PreparedWitTool, WasmError,
    WasmRuntimeCredentialProvider, WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder,
    WasmStagedRuntimeCredentials, WitToolExecution, WitToolHost, WitToolRequest, WitToolRuntime,
    WitToolRuntimeConfig,
};

use crate::obligations::{
    NetworkObligationPolicyStore, RuntimeCredentialAccountResolver, RuntimeSecretInjectionStore,
    SharedSecretStore,
};
use crate::{
    BuiltinObligationHandler, CapabilitySurfaceVersion, DefaultHostRuntime,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, HostRuntimeError,
    HostRuntimeHttpEgressPort, InvocationServicesResolutionRequest, InvocationServicesResolver,
    LocalHostProcessPort, LocalInvocationServicesResolver, PlannerError,
    ProcessObligationLifecycleStore, RuntimeBackendHealth, RuntimeProcessPort,
    RuntimeSecretMaterialStager, RuntimeSecretStageError, TenantSandboxProcessPort,
    ToolCallHttpEgress, TurnRunExecutor, TurnRunScheduler, TurnRunSchedulerConfig, plan_capability,
};
use process_executor::{HostProcessExecutor, RuntimeDispatchProcessExecutor};

type SharedRuntimeHttpEgress = Arc<Mutex<Option<Arc<dyn RuntimeHttpEgress>>>>;
type SharedToolCallHttpEgress = Arc<Mutex<Option<Arc<dyn ToolCallHttpEgress>>>>;

mod builder;
mod production_services;
mod production_wiring;
mod runtime_adapters;
mod wasm_diagnostics;
mod wasm_execution;

use production_wiring::{
    ProductionComponentType, ProductionComponentTypes, ProductionImplementationReadiness,
    component_name, production_wiring_report,
};
pub use production_wiring::{
    ProductionEventStoreWiringError, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssue, ProductionWiringIssueKind, ProductionWiringReport,
};
use runtime_adapters::{
    FirstPartyRuntimeAdapter, McpRuntimeAdapter, ScriptRuntimeAdapter,
    ServiceResolvedRuntimeAdapter, WasmRuntimeAdapter,
};

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
    registry: Arc<SharedExtensionRegistry>,
    trust_policy: Arc<dyn TrustPolicy>,
    trust_policy_configured: bool,
    filesystem: Arc<F>,
    governor: Arc<G>,
    authorizer: Arc<dyn TrustAwareCapabilityDispatchAuthorizer>,
    process_services: ProcessServices<S, R>,
    surface_version: CapabilitySurfaceVersion,
    run_state: Option<Arc<dyn RunStateStore>>,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    run_state_approval_store: Option<Arc<dyn RunStateApprovalStore>>,
    capability_leases: Option<Arc<dyn CapabilityLeaseStore>>,
    // arch-exempt: optional_arc, service builders support minimal/test host runtime
    // graphs while production Reborn wiring installs this store, plan #4539
    persistent_approval_policies: Option<Arc<dyn PersistentApprovalPolicyStore>>,
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    credential_account_store: Arc<dyn CredentialAccountStore>,
    credential_session_store: Arc<dyn CredentialSessionStore>,
    runtime_credential_account_resolver: Option<Arc<dyn RuntimeCredentialAccountResolver>>,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    secret_injection_store: Arc<RuntimeSecretInjectionStore>,
    process_lifecycle_store: Arc<ProcessObligationLifecycleStore>,
    runtime_http_egress: SharedRuntimeHttpEgress,
    tool_call_http_egress: SharedToolCallHttpEgress,
    process_port: Arc<dyn RuntimeProcessPort>,
    managed_process_port: bool,
    tenant_sandbox_process_port: Option<Arc<dyn RuntimeProcessPort>>,
    wasm_credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    runtime_health: Option<Arc<dyn RuntimeBackendHealth>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    process_sandbox_executor: Option<Arc<dyn ProcessExecutor>>,
    script_runtime: Option<Arc<dyn ScriptExecutor>>,
    mcp_runtime: Option<Arc<dyn McpExecutor>>,
    first_party_runtime: Option<Arc<FirstPartyCapabilityRegistry>>,
    wasm_runtime: Option<Arc<WasmRuntimeAdapter>>,
    turn_state: Option<Arc<dyn TurnStateStore>>,
    run_profile_resolver: Option<Arc<dyn RunProfileResolver>>,
    turn_run_transition_port: Option<Arc<dyn TurnRunTransitionPort>>,
    turn_run_wake_notifier: Option<Arc<dyn TurnRunWakeNotifier>>,
    component_types: ProductionComponentTypes,
}

/// Canonical host-runtime ports used by product-auth provider adapters.
///
/// This intentionally exposes only the already-composed egress, obligation
/// handler, and scoped one-shot secret staging operation. Product/auth adapters
/// must not receive the mutable handoff stores that back those ports.
#[derive(Clone)]
pub struct ProductAuthProviderRuntimePorts {
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
    secret_store: Arc<dyn SecretStore>,
    secret_injection_store: Arc<RuntimeSecretInjectionStore>,
}

/// Alias for [`RuntimeSecretStageError`], which re-exports
/// [`ironclaw_host_api::CredentialStageError`].
pub type ProductAuthCredentialStageError = RuntimeSecretStageError;

impl ProductAuthProviderRuntimePorts {
    fn new(
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
        secret_store: Arc<dyn SecretStore>,
        secret_injection_store: Arc<RuntimeSecretInjectionStore>,
    ) -> Self {
        Self {
            runtime_http_egress,
            obligation_handler,
            secret_store,
            secret_injection_store,
        }
    }

    pub fn runtime_http_egress(&self) -> Arc<dyn RuntimeHttpEgress> {
        Arc::clone(&self.runtime_http_egress)
    }

    pub fn obligation_handler(&self) -> Arc<dyn CapabilityObligationHandler> {
        Arc::clone(&self.obligation_handler)
    }

    pub async fn stage_secret_once(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<(), ProductAuthCredentialStageError> {
        self.stage_secret_from_scope_once(scope, scope, capability_id, handle)
            .await
    }

    pub async fn stage_secret_from_scope_once(
        &self,
        source_scope: &ResourceScope,
        target_scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<(), ProductAuthCredentialStageError> {
        let lease = self
            .secret_store
            .lease_once(source_scope, handle)
            .await
            .map_err(stage_secret_error)?;
        let secret = self
            .secret_store
            .consume(source_scope, lease.id)
            .await
            .map_err(stage_secret_error)?;
        self.secret_injection_store
            .insert(target_scope, capability_id, handle, secret)
            .map_err(|_| ProductAuthCredentialStageError::Backend)
    }
}

/// Classify a [`SecretStoreError`] for the host staging surface.
///
/// Unknown / expired / revoked secrets are all user-actionable re-auth
/// conditions: they map to [`ProductAuthCredentialStageError::AuthRequired`]
/// so the runtime auth gate fires instead of surfacing a generic backend
/// failure. Anything else is a true backend defect.
///
/// Used in production by [`ProductAuthProviderRuntimePorts::stage_secret_once`]
/// and by the obligation-handler `stage_credential_material` helper so the
/// WASM `InjectCredentialAccountOnce` lane and the first-party stager lane
/// share identical AuthRequired classification. Crate-private: cross-crate
/// callers must go through one of the two staging entry points above.
pub(crate) fn stage_secret_error(error: SecretStoreError) -> ProductAuthCredentialStageError {
    // Unknown / expired / revoked / consumed / unknown-lease are all user-actionable
    // re-auth conditions: the credential is missing or no longer valid.  Anything
    // else is a true backend defect.
    if error.is_unknown_secret()
        || error.is_unknown_lease()
        || error.is_expired()
        || error.is_revoked()
        || error.is_consumed()
    {
        ProductAuthCredentialStageError::AuthRequired
    } else {
        ProductAuthCredentialStageError::Backend
    }
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
        let credential_broker = Arc::new(InMemoryCredentialBroker::new());
        let credential_account_store: Arc<dyn CredentialAccountStore> = credential_broker.clone();
        let credential_session_store: Arc<dyn CredentialSessionStore> = credential_broker;
        Self {
            registry: Arc::new(SharedExtensionRegistry::new((*registry).clone())),
            trust_policy: Arc::new(HostTrustPolicy::fail_closed()),
            trust_policy_configured: false,
            filesystem,
            governor,
            authorizer,
            process_services,
            surface_version,
            run_state: None,
            approval_requests: None,
            run_state_approval_store: None,
            capability_leases: None,
            persistent_approval_policies: None,
            event_sink: None,
            audit_sink: None,
            security_audit_sink: None,
            secret_store: None,
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver: None,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress: Arc::new(Mutex::new(None)),
            tool_call_http_egress: Arc::new(Mutex::new(None)),
            process_port: Arc::new(LocalHostProcessPort::new()),
            managed_process_port: true,
            tenant_sandbox_process_port: None,
            wasm_credential_provider: None,
            runtime_health: None,
            runtime_policy: None,
            process_sandbox_executor: None,
            script_runtime: None,
            mcp_runtime: None,
            first_party_runtime: None,
            wasm_runtime: None,
            turn_state: None,
            run_profile_resolver: None,
            turn_run_transition_port: None,
            turn_run_wake_notifier: None,
            component_types: ProductionComponentTypes {
                trust_policy: None,
                trust_policy_verified: false,
                filesystem: ProductionComponentType::of::<F>(),
                resource_governor: ProductionComponentType::of::<G>(),
                process_store: ProductionComponentType::of::<S>(),
                process_result_store: ProductionComponentType::of::<R>(),
                run_state: None,
                approval_requests: None,
                capability_leases: None,
                persistent_approval_policies: None,
                event_sink: None,
                audit_sink: None,
                secret_store: None,
                credential_account_store: None,
                credential_session_store: None,
                runtime_http_egress: None,
                runtime_http_egress_verified: false,
                runtime_process_port: ProductionComponentType::of::<LocalHostProcessPort>(),
                tenant_sandbox_process_port: None,
                wasm_credential_provider: None,
                wasm_credential_provider_verified: false,
                wasm_runtime_credential_provider_captured: false,
                script_runtime: None,
                mcp_runtime: None,
                first_party_runtime: None,
                turn_state: None,
                run_profile_resolver: None,
                turn_run_transition_port: None,
                turn_run_transition_port_verified: false,
                turn_run_wake_notifier: None,
            },
        }
    }

    pub fn security_audit_sink(&self) -> Option<Arc<dyn SecurityAuditSink>> {
        self.security_audit_sink.clone()
    }

    /// Builds a runtime dispatcher with every configured runtime adapter.
    fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_shared_registry(
            Arc::clone(&self.registry),
            Arc::clone(&self.filesystem),
            Arc::clone(&self.governor),
        )
        .with_runtime_policy(
            self.runtime_policy
                .clone()
                .unwrap_or_else(local_testing_runtime_policy),
        );
        let mut invocation_services_resolver = LocalInvocationServicesResolver::new(
            Arc::clone(&self.filesystem) as Arc<dyn RootFilesystem>,
            runtime_http_egress(&self.runtime_http_egress),
            Arc::clone(&self.process_port),
            self.secret_store.clone(),
        )
        .with_tool_call_http_egress(tool_call_http_egress(&self.tool_call_http_egress));
        if let Some(audit_sink) = &self.audit_sink {
            invocation_services_resolver =
                invocation_services_resolver.with_audit_sink(Arc::clone(audit_sink));
        }
        if let Some(process_port) = &self.tenant_sandbox_process_port {
            invocation_services_resolver = invocation_services_resolver
                .with_tenant_sandbox_process_port(Arc::clone(process_port));
        }
        let invocation_services: Arc<dyn InvocationServicesResolver> =
            Arc::new(invocation_services_resolver);

        if let Some(runtime) = &self.script_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Script,
                Arc::new(ServiceResolvedRuntimeAdapter::new(
                    Arc::new(ScriptRuntimeAdapter::from_executor(Arc::clone(runtime))),
                    Arc::clone(&invocation_services),
                )),
            );
        }
        if let Some(runtime) = &self.mcp_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Mcp,
                Arc::new(ServiceResolvedRuntimeAdapter::new(
                    Arc::new(McpRuntimeAdapter::from_executor(Arc::clone(runtime))),
                    Arc::clone(&invocation_services),
                )),
            );
        }
        if let Some(runtime) = &self.first_party_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::FirstParty,
                Arc::new(FirstPartyRuntimeAdapter::from_registry(
                    Arc::clone(runtime),
                    Arc::clone(&invocation_services),
                )),
            );
        }
        if let Some(runtime) = &self.wasm_runtime {
            dispatcher = dispatcher.with_runtime_adapter_arc(
                RuntimeKind::Wasm,
                Arc::new(ServiceResolvedRuntimeAdapter::new(
                    Arc::clone(runtime),
                    Arc::clone(&invocation_services),
                )),
            );
        }
        if let Some(event_sink) = &self.event_sink {
            dispatcher = dispatcher.with_event_sink_arc(Arc::clone(event_sink));
        }

        dispatcher
    }

    /// Builds the upper facade without production validation.
    pub fn shared_extension_registry(&self) -> Arc<SharedExtensionRegistry> {
        Arc::clone(&self.registry)
    }

    /// Returns the canonical host-runtime HTTP egress port when configured.
    pub fn runtime_http_egress(&self) -> Option<Arc<dyn RuntimeHttpEgress>> {
        runtime_http_egress(&self.runtime_http_egress)
    }

    /// Returns the canonical host-runtime obligation handler.
    pub fn obligation_handler(&self) -> Arc<dyn CapabilityObligationHandler> {
        Arc::new(self.builtin_obligation_handler())
    }

    /// Returns the canonical host-runtime one-shot secret material stager.
    pub fn runtime_secret_material_stager(&self) -> RuntimeSecretMaterialStager {
        RuntimeSecretMaterialStager::new(Arc::clone(&self.secret_injection_store))
    }

    /// Returns a host-mediated HTTP egress port that owns network-obligation
    /// authorization and one-shot host-held credential staging.
    pub fn host_runtime_http_egress_port(&self) -> Option<HostRuntimeHttpEgressPort> {
        Some(HostRuntimeHttpEgressPort::new(
            self.runtime_http_egress()?,
            self.obligation_handler(),
            self.runtime_secret_material_stager(),
        ))
    }

    /// Returns the canonical host-runtime egress/obligation ports for
    /// product-auth provider adapters.
    pub fn product_auth_provider_runtime_ports(&self) -> Option<ProductAuthProviderRuntimePorts> {
        let runtime_http_egress = self.runtime_http_egress()?;
        let secret_store = self.secret_store.clone()?;
        Some(ProductAuthProviderRuntimePorts::new(
            runtime_http_egress,
            self.obligation_handler(),
            secret_store,
            Arc::clone(&self.secret_injection_store),
        ))
    }

    #[doc(hidden)]
    pub fn host_runtime_for_local_testing(&self) -> DefaultHostRuntime {
        self.build_host_runtime()
    }

    /// Builds the upper facade with the same dispatcher, process services,
    /// stores, cancellation registry, result store, and runtime health graph.
    fn build_host_runtime(&self) -> DefaultHostRuntime {
        let dispatcher: Arc<dyn CapabilityDispatcher> = Arc::new(self.runtime_dispatcher());
        let process_executor = Arc::new(HostProcessExecutor::new(
            Arc::new(RuntimeDispatchProcessExecutor::new(Arc::clone(&dispatcher))),
            self.process_sandbox_executor.clone(),
        ));
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
        let runtime_policy = self
            .runtime_policy
            .clone()
            .unwrap_or_else(local_testing_runtime_policy);
        let surface_filesystem: Arc<dyn RootFilesystem> = self.filesystem.clone();

        let mut runtime = DefaultHostRuntime::from_shared_registry(
            Arc::clone(&self.registry),
            dispatcher,
            Arc::clone(&self.authorizer),
            self.surface_version.clone(),
            runtime_policy,
        )
        .with_surface_filesystem(surface_filesystem)
        .with_trust_policy_dyn(Arc::clone(&self.trust_policy))
        .with_process_manager(process_manager)
        .with_process_store(process_store)
        .with_process_result_store(process_result_store)
        .with_process_cancellation_registry(self.process_services.cancellation_registry())
        .with_runtime_health(runtime_health);

        if let Some(run_state) = &self.run_state {
            runtime = runtime.with_run_state(Arc::clone(run_state));
        }
        if let Some(run_state_approval_store) = &self.run_state_approval_store {
            runtime = runtime.with_run_state_approval_store(Arc::clone(run_state_approval_store));
        } else if let Some(approval_requests) = &self.approval_requests {
            runtime = runtime.with_approval_requests(Arc::clone(approval_requests));
        }
        if let Some(capability_leases) = &self.capability_leases {
            runtime = runtime.with_capability_leases(Arc::clone(capability_leases));
        }
        if let Some(policies) = &self.persistent_approval_policies {
            runtime = runtime.with_persistent_approval_policies(Arc::clone(policies));
        }
        // Wire the credential pre-flight store from the same secret_store used by
        // the obligation handler (below). This ensures that contract tests driving
        // host_runtime_for_local_testing() with .with_secret_store(...) genuinely
        // exercise the pre-flight path, not just the dispatch-time obligation check.
        if let Some(secret_store) = &self.secret_store {
            runtime = runtime.with_credential_preflight_store(Arc::clone(secret_store));
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
        if let Some(security_audit_sink) = &self.security_audit_sink {
            handler = handler.with_security_audit_sink(Arc::clone(security_audit_sink));
        }
        if let Some(secret_store) = &self.secret_store {
            handler = handler.with_secret_store_dyn(Arc::clone(secret_store));
        }
        if let Some(resolver) = &self.runtime_credential_account_resolver {
            handler = handler.with_credential_account_resolver_dyn(Arc::clone(resolver));
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
        if self.first_party_runtime_covers_declared_capabilities() {
            backends.push(RuntimeKind::FirstParty);
        }
        backends
    }

    fn first_party_runtime_covers_declared_capabilities(&self) -> bool {
        let Some(first_party_runtime) = &self.first_party_runtime else {
            return false;
        };
        let registry = self.registry.snapshot();
        let mut declared = registry
            .capabilities()
            .filter(|descriptor| descriptor.runtime == RuntimeKind::FirstParty)
            .peekable();
        if declared.peek().is_none() {
            return false;
        }
        declared.all(|descriptor| first_party_runtime.contains_handler(&descriptor.id))
    }

    fn first_party_runtime_uses_process_port(&self) -> bool {
        let Some(first_party_runtime) = &self.first_party_runtime else {
            return false;
        };
        self.registry.snapshot().capabilities().any(|descriptor| {
            descriptor.runtime == RuntimeKind::FirstParty
                && descriptor.id.as_str() == crate::SHELL_CAPABILITY_ID
                && first_party_runtime.contains_handler(&descriptor.id)
        })
    }
}

fn local_testing_runtime_policy() -> EffectiveRuntimePolicy {
    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalDev,
    ))
    .unwrap_or_else(|error| {
        panic!("LocalSingleUser + LocalDev runtime policy must resolve for local testing: {error}")
    })
}

fn local_only_runtime_policy_reason(policy: &EffectiveRuntimePolicy) -> Option<&'static str> {
    if matches!(policy.deployment, DeploymentMode::LocalSingleUser) {
        return Some("local_single_user_deployment");
    }
    if matches!(
        policy.filesystem_backend,
        FilesystemBackendKind::HostWorkspace | FilesystemBackendKind::HostWorkspaceAndHome
    ) {
        return Some("host_workspace_filesystem");
    }
    if matches!(policy.process_backend, ProcessBackendKind::LocalHost) {
        return Some("local_host_process");
    }
    if matches!(policy.network_mode, NetworkMode::Direct) {
        return Some("direct_network");
    }
    if matches!(
        policy.secret_mode,
        SecretMode::ScrubbedEnv | SecretMode::InheritedEnv
    ) {
        return Some("local_secret_environment");
    }
    None
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

fn set_tool_call_http_egress(
    slot: &SharedToolCallHttpEgress,
    tool_call_http_egress: Arc<dyn ToolCallHttpEgress>,
) {
    match slot.lock() {
        Ok(mut guard) => {
            *guard = Some(tool_call_http_egress);
        }
        Err(poisoned) => {
            *poisoned.into_inner() = Some(tool_call_http_egress);
        }
    }
}

fn runtime_http_egress(slot: &SharedRuntimeHttpEgress) -> Option<Arc<dyn RuntimeHttpEgress>> {
    match slot.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn tool_call_http_egress(slot: &SharedToolCallHttpEgress) -> Option<Arc<dyn ToolCallHttpEgress>> {
    match slot.lock() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn runtime_http_egress_is_configured(slot: &SharedRuntimeHttpEgress) -> bool {
    runtime_http_egress(slot).is_some()
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

#[cfg(test)]
mod tests;
