//! Concrete service graph for the Reborn [`HostRuntime`](crate::HostRuntime).
//!
//! This module is intentionally composition-only. It wires the owning Reborn
//! service crates together, adapts Script/MCP/WASM runtimes into the neutral
//! dispatcher port, and hands upper services a single [`DefaultHostRuntime`]
//! facade. Authorization, run-state transitions, approval leases, process
//! lifecycle, and runtime execution semantics remain in their owning crates.

use std::{
    any::{TypeId, type_name},
    collections::HashMap,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use futures_util::FutureExt;
use ironclaw_approvals::ApprovalResolver;
use ironclaw_authorization::{
    CapabilityLeaseStore, InMemoryCapabilityLeaseStore, TrustAwareCapabilityDispatchAuthorizer,
};
use ironclaw_dispatcher::{
    RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableAuditSink, DurableEventLog, DurableEventSink, EventSink,
    InMemoryAuditSink, InMemoryDurableAuditLog, InMemoryDurableEventLog, InMemoryEventSink,
};
use ironclaw_extensions::{ExtensionRegistry, ExtensionRuntime};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    CapabilityDispatchRequest, CapabilityDispatcher, CapabilityId, DispatchError,
    ResourceReservationId, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind,
    RuntimeHttpEgress, RuntimeKind,
    runtime_policy::{
        DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode,
        ProcessBackendKind, RuntimeProfile, SecretMode,
    },
};
use ironclaw_mcp::{McpError, McpExecutionRequest, McpExecutor, McpInvocation};
use ironclaw_network::NetworkHttpEgress;
use ironclaw_processes::{
    BackgroundFailureStage, InMemoryProcessResultStore, InMemoryProcessStore,
    ProcessExecutionError, ProcessExecutionRequest, ProcessExecutionResult, ProcessExecutor,
    ProcessManager, ProcessResultStore, ProcessServices, ProcessStore,
};
use ironclaw_reborn_event_store::{
    RebornEventStoreConfig, RebornEventStoreError, RebornEventStores, RebornProfile,
    build_reborn_event_stores,
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
use ironclaw_secrets::{InMemorySecretStore, SecretStore};
use ironclaw_trust::{HostTrustPolicy, TrustPolicy};
use ironclaw_turns::{
    DefaultTurnCoordinator, FilesystemTurnStateStore, InMemoryTurnStateStore,
    NoopTurnRunWakeNotifier, RunProfileResolver, TurnRunWakeNotifier, TurnStateStore,
    runner::TurnRunTransitionPort,
};
use ironclaw_wasm::{
    DenyWasmHostHttp, EmptyWasmRuntimeCredentials, PreparedWitTool, WasmError,
    WasmRuntimeCredentialProvider, WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder,
    WasmStagedRuntimeCredentials, WitToolHost, WitToolRequest, WitToolRuntime,
    WitToolRuntimeConfig,
};

use thiserror::Error;

use crate::obligations::{
    NetworkObligationPolicyStore, RuntimeSecretInjectionStore, SharedSecretStore,
};
use crate::{
    BuiltinObligationHandler, CapabilitySurfaceVersion, DefaultHostRuntime,
    FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest, HostRuntimeError,
    InvocationServicesResolutionRequest, InvocationServicesResolver, LocalHostProcessPort,
    LocalInvocationServicesResolver, PlannerError, ProcessObligationLifecycleStore,
    RuntimeBackendHealth, RuntimeProcessPort, TenantSandboxProcessPort, TurnRunExecutor,
    TurnRunScheduler, TurnRunSchedulerConfig, VerifiedTenantSandboxProcessPort, plan_capability,
};

type SharedRuntimeHttpEgress = Arc<Mutex<Option<Arc<dyn RuntimeHttpEgress>>>>;

#[derive(Debug, Error)]
pub enum ProductionEventStoreWiringError {
    #[error("failed to build Reborn event stores: {0}")]
    EventStore(#[from] RebornEventStoreError),
    #[error("host runtime production wiring failed")]
    ProductionWiring(ProductionWiringReport),
}

impl From<ProductionWiringReport> for ProductionEventStoreWiringError {
    fn from(report: ProductionWiringReport) -> Self {
        Self::ProductionWiring(report)
    }
}

/// Production wiring requirements used by composition roots before exposing a
/// [`HostRuntimeServices`] graph as production-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringConfig {
    required_runtime_backends: Vec<RuntimeKind>,
    require_runtime_http_egress: bool,
    require_wasm_credentials: bool,
}

impl ProductionWiringConfig {
    pub fn new<I>(required_runtime_backends: I) -> Self
    where
        I: IntoIterator<Item = RuntimeKind>,
    {
        Self {
            required_runtime_backends: required_runtime_backends.into_iter().collect(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
        }
    }

    pub fn require_runtime_http_egress(mut self) -> Self {
        self.require_runtime_http_egress = true;
        self
    }

    pub fn require_wasm_credentials(mut self) -> Self {
        self.require_wasm_credentials = true;
        self
    }

    fn requires_runtime(&self, runtime: RuntimeKind) -> bool {
        self.required_runtime_backends.contains(&runtime)
    }
}

/// Production component tracked by the host-runtime production wiring guardrail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProductionWiringComponent {
    RuntimeBackend,
    RuntimePolicy,
    TrustPolicy,
    Filesystem,
    ResourceGovernor,
    ProcessStore,
    ProcessResultStore,
    RunState,
    ApprovalRequests,
    CapabilityLeases,
    EventSink,
    AuditSink,
    SecretStore,
    RuntimeHttpEgress,
    RuntimeProcessPort,
    WasmCredentialProvider,
    ScriptRuntime,
    McpRuntime,
    WasmRuntime,
    FirstPartyRuntime,
    TurnState,
    RunProfileResolver,
    TurnRunWakeNotifier,
}

impl ProductionWiringComponent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBackend => "runtime_backend",
            Self::RuntimePolicy => "runtime_policy",
            Self::TrustPolicy => "trust_policy",
            Self::Filesystem => "filesystem",
            Self::ResourceGovernor => "resource_governor",
            Self::ProcessStore => "process_store",
            Self::ProcessResultStore => "process_result_store",
            Self::RunState => "run_state",
            Self::ApprovalRequests => "approval_requests",
            Self::CapabilityLeases => "capability_leases",
            Self::EventSink => "event_sink",
            Self::AuditSink => "audit_sink",
            Self::SecretStore => "secret_store",
            Self::RuntimeHttpEgress => "runtime_http_egress",
            Self::RuntimeProcessPort => "runtime_process_port",
            Self::WasmCredentialProvider => "wasm_credential_provider",
            Self::ScriptRuntime => "script_runtime",
            Self::McpRuntime => "mcp_runtime",
            Self::WasmRuntime => "wasm_runtime",
            Self::FirstPartyRuntime => "first_party_runtime",
            Self::TurnState => "turn_state",
            Self::RunProfileResolver => "run_profile_resolver",
            Self::TurnRunWakeNotifier => "turn_run_wake_notifier",
        }
    }
}

/// Category of production wiring issue found in a service graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionWiringIssueKind {
    Missing,
    UnsupportedRequirement,
    LocalOnlyImplementation,
    UnverifiedProductionImplementation,
}

/// One production wiring issue for a component in the host-runtime graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringIssue {
    component: ProductionWiringComponent,
    kind: ProductionWiringIssueKind,
    implementation: Option<&'static str>,
}

impl ProductionWiringIssue {
    pub fn component(&self) -> ProductionWiringComponent {
        self.component
    }

    pub fn kind(&self) -> ProductionWiringIssueKind {
        self.kind
    }

    pub fn implementation(&self) -> Option<&'static str> {
        self.implementation
    }
}

/// Report returned when a host-runtime graph is not production-ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionWiringReport {
    issues: Vec<ProductionWiringIssue>,
}

impl ProductionWiringReport {
    pub fn issues(&self) -> &[ProductionWiringIssue] {
        &self.issues
    }

    pub fn contains(
        &self,
        component: ProductionWiringComponent,
        kind: ProductionWiringIssueKind,
    ) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.component == component && issue.kind == kind)
    }
}

#[derive(Debug, Clone)]
struct ProductionComponentTypes {
    trust_policy: Option<ProductionComponentType>,
    trust_policy_verified: bool,
    filesystem: ProductionComponentType,
    resource_governor: ProductionComponentType,
    process_store: ProductionComponentType,
    process_result_store: ProductionComponentType,
    run_state: Option<ProductionComponentType>,
    approval_requests: Option<ProductionComponentType>,
    capability_leases: Option<ProductionComponentType>,
    event_sink: Option<ProductionComponentType>,
    audit_sink: Option<ProductionComponentType>,
    secret_store: Option<ProductionComponentType>,
    runtime_http_egress: Option<ProductionComponentType>,
    runtime_http_egress_verified: bool,
    runtime_process_port: ProductionComponentType,
    tenant_sandbox_process_port: Option<ProductionComponentType>,
    wasm_credential_provider: Option<ProductionComponentType>,
    wasm_credential_provider_verified: bool,
    wasm_runtime_credential_provider_captured: bool,
    script_runtime: Option<ProductionComponentType>,
    mcp_runtime: Option<ProductionComponentType>,
    first_party_runtime: Option<ProductionComponentType>,
    turn_state: Option<ProductionComponentType>,
    run_profile_resolver: Option<ProductionComponentType>,
    turn_run_transition_port: Option<ProductionComponentType>,
    turn_run_transition_port_verified: bool,
    turn_run_wake_notifier: Option<ProductionComponentType>,
}

#[derive(Debug, Clone, Copy)]
struct ProductionComponentType {
    implementation: &'static str,
    readiness: ProductionImplementationReadiness,
}

impl ProductionComponentType {
    fn of<T: 'static>() -> Self {
        Self {
            implementation: type_name::<T>(),
            readiness: classify_component_type::<T>(),
        }
    }

    fn named(implementation: &'static str, readiness: ProductionImplementationReadiness) -> Self {
        Self {
            implementation,
            readiness,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProductionImplementationReadiness {
    ProductionCandidate,
    LocalOnly,
    UnverifiedProductionImplementation,
}

fn component_name(component: Option<ProductionComponentType>) -> Option<&'static str> {
    component.map(|component| component.implementation)
}

fn classify_component_type<T: 'static>() -> ProductionImplementationReadiness {
    let type_id = TypeId::of::<T>();
    match () {
        () if type_id == TypeId::of::<LocalFilesystem>()
            || type_id == TypeId::of::<InMemoryResourceGovernor>()
            || type_id == TypeId::of::<InMemoryProcessStore>()
            || type_id == TypeId::of::<InMemoryProcessResultStore>()
            || type_id == TypeId::of::<InMemoryRunStateStore>()
            || type_id == TypeId::of::<InMemoryApprovalRequestStore>()
            || type_id == TypeId::of::<InMemoryCapabilityLeaseStore>()
            || type_id == TypeId::of::<InMemoryEventSink>()
            || type_id == TypeId::of::<InMemoryDurableEventLog>()
            || type_id == TypeId::of::<InMemoryAuditSink>()
            || type_id == TypeId::of::<InMemoryDurableAuditLog>()
            || type_id == TypeId::of::<InMemorySecretStore>()
            || type_id == TypeId::of::<EmptyWasmRuntimeCredentials>()
            || type_id == TypeId::of::<InMemoryTurnStateStore>()
            || type_id == TypeId::of::<NoopTurnRunWakeNotifier>()
            || type_id == TypeId::of::<LocalHostProcessPort>() =>
        {
            ProductionImplementationReadiness::LocalOnly
        }
        () if type_id == TypeId::of::<DurableEventSink>()
            || type_id == TypeId::of::<DurableAuditSink>() =>
        {
            ProductionImplementationReadiness::UnverifiedProductionImplementation
        }
        () => ProductionImplementationReadiness::ProductionCandidate,
    }
}

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
    event_sink: Option<Arc<dyn EventSink>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    secret_injection_store: Arc<RuntimeSecretInjectionStore>,
    process_lifecycle_store: Arc<ProcessObligationLifecycleStore>,
    runtime_http_egress: SharedRuntimeHttpEgress,
    process_port: Arc<dyn RuntimeProcessPort>,
    tenant_sandbox_process_port: Option<Arc<dyn RuntimeProcessPort>>,
    wasm_credential_provider: Option<Arc<dyn WasmRuntimeCredentialProvider>>,
    runtime_health: Option<Arc<dyn RuntimeBackendHealth>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
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
            event_sink: None,
            audit_sink: None,
            secret_store: None,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress: Arc::new(Mutex::new(None)),
            process_port: Arc::new(LocalHostProcessPort::new()),
            tenant_sandbox_process_port: None,
            wasm_credential_provider: None,
            runtime_health: None,
            runtime_policy: None,
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
                event_sink: None,
                audit_sink: None,
                secret_store: None,
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

    #[cfg(any(feature = "postgres", feature = "libsql"))]
    fn with_root_filesystem<T>(self, filesystem: Arc<T>) -> HostRuntimeServices<T, G, S, R>
    where
        T: RootFilesystem + 'static,
    {
        let Self {
            registry,
            trust_policy,
            trust_policy_configured,
            filesystem: _,
            governor,
            authorizer,
            process_services,
            surface_version,
            run_state,
            approval_requests,
            run_state_approval_store,
            capability_leases,
            event_sink,
            audit_sink,
            secret_store,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            script_runtime,
            mcp_runtime,
            first_party_runtime,
            wasm_runtime,
            turn_state,
            run_profile_resolver,
            turn_run_transition_port,
            turn_run_wake_notifier,
            mut component_types,
        } = self;
        component_types.filesystem = ProductionComponentType::of::<T>();
        HostRuntimeServices {
            registry,
            trust_policy,
            trust_policy_configured,
            filesystem,
            governor,
            authorizer,
            process_services,
            surface_version,
            run_state,
            approval_requests,
            run_state_approval_store,
            capability_leases,
            event_sink,
            audit_sink,
            secret_store,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            script_runtime,
            mcp_runtime,
            first_party_runtime,
            wasm_runtime,
            turn_state,
            run_profile_resolver,
            turn_run_transition_port,
            turn_run_wake_notifier,
            component_types,
        }
    }

    #[cfg(feature = "postgres")]
    pub fn with_postgres_root_filesystem(
        self,
        filesystem: Arc<PostgresRootFilesystem>,
    ) -> HostRuntimeServices<PostgresRootFilesystem, G, S, R> {
        self.with_root_filesystem(filesystem)
    }

    #[cfg(feature = "libsql")]
    pub fn with_libsql_root_filesystem(
        self,
        filesystem: Arc<LibSqlRootFilesystem>,
    ) -> HostRuntimeServices<LibSqlRootFilesystem, G, S, R> {
        self.with_root_filesystem(filesystem)
    }

    fn with_resource_governor<T>(self, governor: Arc<T>) -> HostRuntimeServices<F, T, S, R>
    where
        T: ResourceGovernor + 'static,
    {
        let Self {
            registry,
            trust_policy,
            trust_policy_configured,
            filesystem,
            governor: _,
            authorizer,
            process_services,
            surface_version,
            run_state,
            approval_requests,
            run_state_approval_store,
            capability_leases,
            event_sink,
            audit_sink,
            secret_store,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store: _,
            runtime_http_egress,
            process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            script_runtime,
            mcp_runtime,
            first_party_runtime,
            wasm_runtime,
            turn_state,
            run_profile_resolver,
            turn_run_transition_port,
            turn_run_wake_notifier,
            mut component_types,
        } = self;
        let lifecycle_governor: Arc<dyn ResourceGovernor> = governor.clone();
        let process_lifecycle_store = Arc::new(ProcessObligationLifecycleStore::new(
            process_services.process_store(),
            Arc::clone(&network_policy_store),
            Arc::clone(&secret_injection_store),
            lifecycle_governor,
        ));
        if let Some(event_sink) = &event_sink {
            process_lifecycle_store.set_event_sink(Arc::clone(event_sink));
        }
        component_types.resource_governor = ProductionComponentType::of::<T>();
        HostRuntimeServices {
            registry,
            trust_policy,
            trust_policy_configured,
            filesystem,
            governor,
            authorizer,
            process_services,
            surface_version,
            run_state,
            approval_requests,
            run_state_approval_store,
            capability_leases,
            event_sink,
            audit_sink,
            secret_store,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            script_runtime,
            mcp_runtime,
            first_party_runtime,
            wasm_runtime,
            turn_state,
            run_profile_resolver,
            turn_run_transition_port,
            turn_run_wake_notifier,
            component_types,
        }
    }

    /// Replace the in-memory governor with a filesystem-backed
    /// [`PersistentResourceGovernor`] over the supplied
    /// [`ScopedFilesystem`]. Backend choice (libSQL, Postgres, in-memory,
    /// local disk) is a property of the underlying
    /// [`RootFilesystem`](ironclaw_filesystem::RootFilesystem); see
    /// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
    pub fn with_filesystem_resource_governor<FsBackend>(
        self,
        scoped_filesystem: Arc<ScopedFilesystem<FsBackend>>,
    ) -> HostRuntimeServices<
        F,
        PersistentResourceGovernor<FilesystemResourceGovernorStore<FsBackend>>,
        S,
        R,
    >
    where
        FsBackend: RootFilesystem + 'static,
    {
        let store = FilesystemResourceGovernorStore::new(scoped_filesystem);
        self.with_resource_governor(Arc::new(PersistentResourceGovernor::new(store)))
    }

    pub fn resource_governor(&self) -> Arc<G> {
        Arc::clone(&self.governor)
    }

    /// Attaches the host-owned trust policy used by the produced
    /// [`DefaultHostRuntime`]. Without this, the service graph keeps the
    /// default fail-closed policy and capability dispatch is denied.
    pub fn with_trust_policy<T>(mut self, trust_policy: Arc<T>) -> Self
    where
        T: TrustPolicy + 'static,
    {
        self.component_types.trust_policy = Some(ProductionComponentType::of::<T>());
        self.component_types.trust_policy_verified = true;
        self.trust_policy = trust_policy;
        self.trust_policy_configured = true;
        self
    }

    pub fn with_trust_policy_dyn(mut self, trust_policy: Arc<dyn TrustPolicy>) -> Self {
        self.component_types.trust_policy = Some(ProductionComponentType::named(
            "dyn TrustPolicy",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
        self.component_types.trust_policy_verified = false;
        self.trust_policy = trust_policy;
        self.trust_policy_configured = true;
        self
    }

    pub fn with_run_state<T>(mut self, run_state: Arc<T>) -> Self
    where
        T: RunStateStore + 'static,
    {
        self.component_types.run_state = Some(ProductionComponentType::of::<T>());
        self.run_state = Some(run_state);
        self.run_state_approval_store = None;
        self
    }

    pub fn with_approval_requests<T>(mut self, approval_requests: Arc<T>) -> Self
    where
        T: ApprovalRequestStore + 'static,
    {
        self.component_types.approval_requests = Some(ProductionComponentType::of::<T>());
        self.approval_requests = Some(approval_requests);
        self.run_state_approval_store = None;
        self
    }

    pub fn with_run_state_approval_store<T>(self, store: Arc<T>) -> Self
    where
        T: RunStateApprovalStore + 'static,
    {
        self.with_run_state_approval_store_readiness(store, ProductionComponentType::of::<T>())
    }

    /// Attaches a combined run-state/approval store that is explicitly marked
    /// local-only by the composition root. This avoids relying on implementation
    /// type-name strings for custom test/local stores while preserving a typed
    /// production-readiness classification.
    pub fn with_local_only_run_state_approval_store<T>(self, store: Arc<T>) -> Self
    where
        T: RunStateApprovalStore + 'static,
    {
        self.with_run_state_approval_store_readiness(
            store,
            ProductionComponentType::named(
                type_name::<T>(),
                ProductionImplementationReadiness::LocalOnly,
            ),
        )
    }

    fn with_run_state_approval_store_readiness<T>(
        mut self,
        store: Arc<T>,
        component_type: ProductionComponentType,
    ) -> Self
    where
        T: RunStateApprovalStore + 'static,
    {
        self.component_types.run_state = Some(component_type);
        self.component_types.approval_requests = Some(component_type);
        self.run_state = Some(store.clone());
        self.approval_requests = Some(store.clone());
        self.run_state_approval_store = Some(store);
        self
    }

    /// Builds and attaches filesystem-backed run-state and approval-request
    /// stores over the supplied [`ScopedFilesystem`].
    ///
    /// Production composition wires both `/run-state` and `/approvals` mount
    /// aliases on the same [`ScopedFilesystem`], so a single handle is enough
    /// to construct both stores: each takes its alias-relative subtree
    /// through the shared `MountView`. The backend choice
    /// (`LibSqlRootFilesystem`, `PostgresRootFilesystem`,
    /// `InMemoryBackend`, …) happens at the `RootFilesystem` layer, not here.
    ///
    /// Replaces the legacy `with_libsql_run_state_approval_store` /
    /// `with_postgres_run_state_approval_store` builders (deleted along with
    /// the corresponding per-backend `Filesystem*Store` siblings — see
    /// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`).
    ///
    /// Unlike the deleted SQL combined store this wiring does NOT carry an
    /// atomic `save_pending_and_block_approval` transition: filesystem
    /// stores ship as two independent records under distinct mount aliases.
    /// Callers fall back to the two-step
    /// `ApprovalRequestStore::save_pending` then
    /// `RunStateStore::block_approval` path in
    /// `ironclaw_capabilities::host`. Production composition should layer a
    /// transactional wrapper (or accept the two-step semantics) when
    /// cross-record atomicity matters.
    pub fn with_filesystem_run_state<FsBackend>(
        self,
        scoped_filesystem: Arc<ScopedFilesystem<FsBackend>>,
    ) -> Self
    where
        FsBackend: RootFilesystem + 'static,
    {
        let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(&scoped_filesystem)));
        let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(scoped_filesystem));
        self.with_run_state(run_state)
            .with_approval_requests(approval_requests)
    }

    pub fn with_capability_leases<T>(mut self, capability_leases: Arc<T>) -> Self
    where
        T: CapabilityLeaseStore + 'static,
    {
        self.component_types.capability_leases = Some(ProductionComponentType::of::<T>());
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_turn_state<T>(mut self, turn_state: Arc<T>) -> Self
    where
        T: TurnStateStore + 'static,
    {
        self.component_types.turn_state = Some(ProductionComponentType::of::<T>());
        self.component_types.turn_run_transition_port = None;
        self.component_types.turn_run_transition_port_verified = false;
        self.turn_state = Some(turn_state);
        self.turn_run_transition_port = None;
        self
    }

    pub fn with_turn_state_and_transition_port<T>(mut self, turn_state: Arc<T>) -> Self
    where
        T: TurnStateStore + TurnRunTransitionPort + 'static,
    {
        self.component_types.turn_state = Some(ProductionComponentType::of::<T>());
        self.component_types.turn_run_transition_port = Some(ProductionComponentType::of::<T>());
        self.component_types.turn_run_transition_port_verified = true;
        let state: Arc<dyn TurnStateStore> = turn_state.clone();
        let transition_port: Arc<dyn TurnRunTransitionPort> = turn_state;
        self.turn_state = Some(state);
        self.turn_run_transition_port = Some(transition_port);
        self
    }

    pub fn with_turn_run_transition_port<T>(mut self, transition_port: Arc<T>) -> Self
    where
        T: TurnRunTransitionPort + 'static,
    {
        self.component_types.turn_run_transition_port = Some(ProductionComponentType::of::<T>());
        self.component_types.turn_run_transition_port_verified = false;
        self.turn_run_transition_port = Some(transition_port);
        self
    }

    pub fn with_run_profile_resolver<T>(mut self, resolver: Arc<T>) -> Self
    where
        T: RunProfileResolver + 'static,
    {
        self.component_types.run_profile_resolver = Some(ProductionComponentType::of::<T>());
        self.run_profile_resolver = Some(resolver);
        self
    }

    /// Builds and attaches a filesystem-backed turn-state store over the
    /// supplied [`ScopedFilesystem`].
    ///
    /// Production composition wires the `/turns` mount alias on the same
    /// [`ScopedFilesystem`] that carries the other consumer-store aliases,
    /// so a single handle is enough to construct this store: it takes its
    /// alias-relative subtree through the shared `MountView`. The backend
    /// choice (`LibSqlRootFilesystem`, `PostgresRootFilesystem`,
    /// `InMemoryBackend`, …) happens at the [`RootFilesystem`] layer, not
    /// here.
    ///
    /// Replaces the legacy `with_libsql_turn_state_store` /
    /// `with_postgres_turn_state_store` builders (deleted along with the
    /// corresponding per-backend `Filesystem*Store` siblings — see
    /// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`). The
    /// filesystem store implements both [`TurnStateStore`] and
    /// [`TurnRunTransitionPort`], so this wiring covers production
    /// readiness for both axes.
    pub fn with_filesystem_turn_state_store<FsBackend>(
        self,
        scoped_filesystem: Arc<ScopedFilesystem<FsBackend>>,
    ) -> Self
    where
        FsBackend: RootFilesystem + 'static,
    {
        let store = Arc::new(FilesystemTurnStateStore::new(scoped_filesystem));
        self.with_turn_state_and_transition_port(store)
    }

    pub fn with_turn_run_wake_notifier<T>(mut self, notifier: Arc<T>) -> Self
    where
        T: TurnRunWakeNotifier + 'static,
    {
        self.component_types.turn_run_wake_notifier = Some(ProductionComponentType::of::<T>());
        self.turn_run_wake_notifier = Some(notifier);
        self
    }

    pub fn with_event_sink<T>(mut self, event_sink: Arc<T>) -> Self
    where
        T: EventSink + 'static,
    {
        self.component_types.event_sink = Some(ProductionComponentType::of::<T>());
        let event_sink: Arc<dyn EventSink> = event_sink;
        self.process_lifecycle_store
            .set_event_sink(Arc::clone(&event_sink));
        self.event_sink = Some(event_sink);
        self
    }

    pub fn with_durable_event_log<T>(mut self, event_log: Arc<T>) -> Self
    where
        T: DurableEventLog + 'static,
    {
        self.component_types.event_sink = Some(ProductionComponentType::of::<T>());
        let event_log: Arc<dyn DurableEventLog> = event_log;
        let event_sink: Arc<dyn EventSink> = Arc::new(DurableEventSink::new(event_log));
        self.process_lifecycle_store
            .set_event_sink(Arc::clone(&event_sink));
        self.event_sink = Some(event_sink);
        self
    }

    pub fn with_audit_sink<T>(mut self, audit_sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        self.component_types.audit_sink = Some(ProductionComponentType::of::<T>());
        self.audit_sink = Some(audit_sink);
        self
    }

    pub fn with_durable_audit_log<T>(mut self, audit_log: Arc<T>) -> Self
    where
        T: DurableAuditLog + 'static,
    {
        self.component_types.audit_sink = Some(ProductionComponentType::of::<T>());
        let audit_log: Arc<dyn DurableAuditLog> = audit_log;
        self.audit_sink = Some(Arc::new(DurableAuditSink::new(audit_log)));
        self
    }

    /// Attaches a pre-built Reborn durable event/audit store pair to the host
    /// runtime graph. This is the production composition seam for store
    /// selection: callers choose Postgres/libSQL/accepted-JSONL through
    /// `ironclaw_reborn_event_store`, then this method adapts the durable logs
    /// into the live sink traits consumed by runtime services.
    pub fn with_reborn_event_stores(self, stores: RebornEventStores) -> Self {
        self.with_reborn_event_stores_verified(stores, false)
    }

    fn with_reborn_event_stores_verified(
        mut self,
        stores: RebornEventStores,
        production_verified: bool,
    ) -> Self {
        if production_verified {
            self.component_types.event_sink =
                Some(ProductionComponentType::of::<RebornEventStores>());
            self.component_types.audit_sink =
                Some(ProductionComponentType::of::<RebornEventStores>());
        } else {
            // Prebuilt/LocalDev/Test stores are useful for tests and lower-level
            // composition, but must not silently satisfy production guardrails.
            self.component_types.event_sink =
                Some(ProductionComponentType::of::<DurableEventSink>());
            self.component_types.audit_sink =
                Some(ProductionComponentType::of::<DurableAuditSink>());
        }
        self.event_sink = Some(Arc::new(DurableEventSink::new(stores.events)));
        self.audit_sink = Some(Arc::new(DurableAuditSink::new(stores.audit)));
        self
    }

    /// Builds Reborn event/audit stores from profile/config and attaches them
    /// to this service graph. Production JSONL/in-memory restrictions are
    /// enforced by `build_reborn_event_stores` before sinks are installed.
    pub async fn with_reborn_event_store_config(
        self,
        profile: RebornProfile,
        config: RebornEventStoreConfig,
    ) -> Result<Self, RebornEventStoreError> {
        let stores = build_reborn_event_stores(profile, config).await?;
        Ok(self.with_reborn_event_stores_verified(stores, profile == RebornProfile::Production))
    }

    pub fn with_secret_store<T>(mut self, secret_store: Arc<T>) -> Self
    where
        T: SecretStore + 'static,
    {
        self.component_types.secret_store = Some(ProductionComponentType::of::<T>());
        self.secret_store = Some(secret_store);
        self
    }

    pub fn with_runtime_http_egress<T>(mut self, runtime_http_egress: Arc<T>) -> Self
    where
        T: RuntimeHttpEgress + 'static,
    {
        self.component_types.runtime_http_egress = Some(ProductionComponentType::of::<T>());
        self.component_types.runtime_http_egress_verified = false;
        let runtime_http_egress: Arc<dyn RuntimeHttpEgress> = runtime_http_egress;
        set_runtime_http_egress(&self.runtime_http_egress, runtime_http_egress);
        self
    }

    pub fn with_runtime_process_port<T>(mut self, process_port: Arc<T>) -> Self
    where
        T: RuntimeProcessPort + 'static,
    {
        self.component_types.runtime_process_port = ProductionComponentType::of::<T>();
        self.process_port = process_port;
        self
    }

    pub fn with_runtime_process_port_dyn(
        mut self,
        process_port: Arc<dyn RuntimeProcessPort>,
    ) -> Self {
        self.component_types.runtime_process_port = ProductionComponentType::named(
            "dyn RuntimeProcessPort",
            ProductionImplementationReadiness::UnverifiedProductionImplementation,
        );
        self.process_port = process_port;
        self
    }

    pub fn with_tenant_sandbox_process_port(
        mut self,
        process_port: Arc<TenantSandboxProcessPort>,
    ) -> Self {
        self.component_types.tenant_sandbox_process_port = Some(ProductionComponentType::named(
            "TenantSandboxProcessPort",
            ProductionImplementationReadiness::UnverifiedProductionImplementation,
        ));
        self.tenant_sandbox_process_port = Some(process_port);
        self
    }

    /// Attaches a tenant sandbox process port whose transport is verified by
    /// the composition root as production-suitable for the active
    /// tenant-scoped runtime policy.
    pub fn with_verified_tenant_sandbox_process_port(
        mut self,
        process_port: VerifiedTenantSandboxProcessPort,
    ) -> Self {
        self.component_types.tenant_sandbox_process_port = Some(ProductionComponentType::named(
            "TenantSandboxProcessPort",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
        self.tenant_sandbox_process_port = Some(process_port.into_inner());
        self
    }

    /// Attaches the host HTTP egress shape required for production runtime
    /// adapters. The service must use staged network-policy handoffs and secret
    /// injection handoffs, not request-local/test policy fallback.
    pub fn with_host_http_egress_service<N, SecretBackend>(
        mut self,
        runtime_http_egress: Arc<crate::HostHttpEgressService<N, SecretBackend>>,
    ) -> Self
    where
        N: NetworkHttpEgress + 'static,
        SecretBackend: SecretStore + 'static,
    {
        self.component_types.runtime_http_egress = Some(ProductionComponentType::of::<
            crate::HostHttpEgressService<N, SecretBackend>,
        >());
        self.component_types.runtime_http_egress_verified = runtime_http_egress
            .is_production_wired_with(&self.network_policy_store, &self.secret_injection_store);
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

    pub fn with_runtime_policy(mut self, policy: EffectiveRuntimePolicy) -> Self {
        self.runtime_policy = Some(policy);
        self
    }

    pub fn with_wasm_runtime_credential_provider<T>(mut self, provider: Arc<T>) -> Self
    where
        T: WasmRuntimeCredentialProvider + 'static,
    {
        self.component_types.wasm_credential_provider = Some(ProductionComponentType::of::<T>());
        self.component_types.wasm_credential_provider_verified = false;
        let provider: Arc<dyn WasmRuntimeCredentialProvider> = provider;
        self.wasm_credential_provider = Some(provider);
        self.component_types
            .wasm_runtime_credential_provider_captured = self.wasm_runtime.is_none();
        self
    }

    pub fn with_verified_wasm_runtime_credentials(
        mut self,
        provider: Arc<WasmStagedRuntimeCredentials>,
    ) -> Self {
        self.component_types.wasm_credential_provider =
            Some(ProductionComponentType::of::<WasmStagedRuntimeCredentials>());
        self.component_types.wasm_credential_provider_verified = !provider.credentials().is_empty();
        let provider: Arc<dyn WasmRuntimeCredentialProvider> = provider;
        self.wasm_credential_provider = Some(provider);
        self.component_types
            .wasm_runtime_credential_provider_captured = self.wasm_runtime.is_none();
        self
    }

    /// Builds and attaches production-shaped host HTTP egress using this
    /// service graph's private network-policy, secret-injection, and secret-store
    /// handles. Callers provide concrete network transport, but never receive the
    /// mutable handoff stores or choose a separate secret backend.
    pub fn try_with_host_http_egress<N>(self, network: N) -> Result<Self, ProductionWiringReport>
    where
        N: NetworkHttpEgress + 'static,
    {
        let Some(secret_store) = self.secret_store.clone() else {
            return Err(production_wiring_report(
                ProductionWiringComponent::SecretStore,
                ProductionWiringIssueKind::Missing,
                None,
            ));
        };
        let runtime_http_egress = Arc::new(
            crate::HostHttpEgressService::new(network, SharedSecretStore(secret_store))
                .with_network_policy_store(Arc::clone(&self.network_policy_store))
                .with_secret_injection_store(Arc::clone(&self.secret_injection_store)),
        );
        Ok(self.with_host_http_egress_service(runtime_http_egress))
    }

    pub fn with_script_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: ScriptExecutor + 'static,
    {
        self.component_types.script_runtime = Some(ProductionComponentType::of::<T>());
        self.script_runtime = Some(runtime);
        self
    }

    pub fn with_mcp_runtime<T>(mut self, runtime: Arc<T>) -> Self
    where
        T: McpExecutor + 'static,
    {
        self.component_types.mcp_runtime = Some(ProductionComponentType::of::<T>());
        self.mcp_runtime = Some(runtime);
        self
    }

    pub fn with_first_party_capabilities(
        mut self,
        registry: Arc<FirstPartyCapabilityRegistry>,
    ) -> Self {
        self.component_types.first_party_runtime =
            Some(ProductionComponentType::of::<FirstPartyCapabilityRegistry>());
        self.first_party_runtime = Some(registry);
        self
    }

    fn with_wasm_runtime(mut self, runtime: Arc<WasmRuntimeAdapter>) -> Self {
        self.component_types
            .wasm_runtime_credential_provider_captured = self.wasm_credential_provider.is_some();
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

    /// Validates that this service graph is explicitly wired for production
    /// instead of relying on local/test defaults. This is a guardrail for
    /// composition roots; it does not build or mutate the runtime graph.
    pub fn validate_production_wiring(
        &self,
        config: &ProductionWiringConfig,
    ) -> Result<(), ProductionWiringReport> {
        let mut issues = Vec::new();

        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TrustPolicy,
            self.trust_policy_configured,
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::RuntimePolicy,
            self.runtime_policy.is_some(),
        );
        if let Some(runtime_policy) = &self.runtime_policy
            && let Some(reason) = local_only_runtime_policy_reason(runtime_policy)
        {
            self.push_issue(
                &mut issues,
                ProductionWiringComponent::RuntimePolicy,
                ProductionWiringIssueKind::LocalOnlyImplementation,
                Some(reason),
            );
        }
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::RunState,
            self.run_state.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::ApprovalRequests,
            self.approval_requests.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::CapabilityLeases,
            self.capability_leases.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.turn_state.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::RunProfileResolver,
            self.run_profile_resolver.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TurnRunWakeNotifier,
            self.turn_run_wake_notifier.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::EventSink,
            self.event_sink.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::AuditSink,
            self.audit_sink.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::SecretStore,
            self.secret_store.is_some(),
        );

        if config.require_runtime_http_egress {
            let runtime_http_configured =
                runtime_http_egress_is_configured(&self.runtime_http_egress);
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::RuntimeHttpEgress,
                runtime_http_configured,
            );
            if runtime_http_configured && !self.component_types.runtime_http_egress_verified {
                self.push_issue(
                    &mut issues,
                    ProductionWiringComponent::RuntimeHttpEgress,
                    ProductionWiringIssueKind::UnverifiedProductionImplementation,
                    component_name(self.component_types.runtime_http_egress),
                );
            }
        }
        if config.require_wasm_credentials {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::WasmCredentialProvider,
                self.wasm_credential_provider.is_some(),
            );
            if self.wasm_credential_provider.is_some()
                && !self.component_types.wasm_credential_provider_verified
            {
                self.push_issue(
                    &mut issues,
                    ProductionWiringComponent::WasmCredentialProvider,
                    ProductionWiringIssueKind::UnverifiedProductionImplementation,
                    component_name(self.component_types.wasm_credential_provider),
                );
            }
            if self.wasm_runtime.is_some()
                && self.wasm_credential_provider.is_some()
                && !self
                    .component_types
                    .wasm_runtime_credential_provider_captured
            {
                self.push_issue(
                    &mut issues,
                    ProductionWiringComponent::WasmCredentialProvider,
                    ProductionWiringIssueKind::UnverifiedProductionImplementation,
                    component_name(self.component_types.wasm_credential_provider),
                );
            }
        }
        for runtime in &config.required_runtime_backends {
            match runtime {
                RuntimeKind::Script
                | RuntimeKind::Mcp
                | RuntimeKind::Wasm
                | RuntimeKind::FirstParty => {}
                RuntimeKind::System => self.push_issue(
                    &mut issues,
                    ProductionWiringComponent::RuntimeBackend,
                    ProductionWiringIssueKind::UnsupportedRequirement,
                    None,
                ),
            }
        }
        if config.requires_runtime(RuntimeKind::Script) {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::ScriptRuntime,
                self.script_runtime.is_some(),
            );
        }
        if config.requires_runtime(RuntimeKind::Mcp) {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::McpRuntime,
                self.mcp_runtime.is_some(),
            );
        }
        if config.requires_runtime(RuntimeKind::Wasm) {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::WasmRuntime,
                self.wasm_runtime.is_some(),
            );
        }
        if config.requires_runtime(RuntimeKind::FirstParty) {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::FirstPartyRuntime,
                self.first_party_runtime_covers_declared_capabilities(),
            );
        }
        if self.first_party_runtime_uses_process_port() {
            if self
                .runtime_policy
                .as_ref()
                .is_some_and(|policy| policy.process_backend == ProcessBackendKind::TenantSandbox)
            {
                self.push_missing(
                    &mut issues,
                    ProductionWiringComponent::RuntimeProcessPort,
                    self.tenant_sandbox_process_port.is_some(),
                );
                self.push_local_only(
                    &mut issues,
                    ProductionWiringComponent::RuntimeProcessPort,
                    self.component_types.tenant_sandbox_process_port,
                );
            } else {
                self.push_local_only(
                    &mut issues,
                    ProductionWiringComponent::RuntimeProcessPort,
                    Some(self.component_types.runtime_process_port),
                );
            }
        }

        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TrustPolicy,
            self.component_types.trust_policy,
        );
        if self.trust_policy_configured && !self.component_types.trust_policy_verified {
            self.push_issue(
                &mut issues,
                ProductionWiringComponent::TrustPolicy,
                ProductionWiringIssueKind::UnverifiedProductionImplementation,
                component_name(self.component_types.trust_policy),
            );
        }
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::Filesystem,
            Some(self.component_types.filesystem),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::ResourceGovernor,
            Some(self.component_types.resource_governor),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::ProcessStore,
            Some(self.component_types.process_store),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::ProcessResultStore,
            Some(self.component_types.process_result_store),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::RunState,
            self.component_types.run_state,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::ApprovalRequests,
            self.component_types.approval_requests,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::CapabilityLeases,
            self.component_types.capability_leases,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.component_types.turn_state,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnRunWakeNotifier,
            self.component_types.turn_run_wake_notifier,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::EventSink,
            self.component_types.event_sink,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::AuditSink,
            self.component_types.audit_sink,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::SecretStore,
            self.component_types.secret_store,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::RuntimeHttpEgress,
            self.component_types.runtime_http_egress,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::WasmCredentialProvider,
            self.component_types.wasm_credential_provider,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::ScriptRuntime,
            self.component_types.script_runtime,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::McpRuntime,
            self.component_types.mcp_runtime,
        );

        if issues.is_empty() {
            Ok(())
        } else {
            Err(ProductionWiringReport { issues })
        }
    }

    fn push_missing(
        &self,
        issues: &mut Vec<ProductionWiringIssue>,
        component: ProductionWiringComponent,
        present: bool,
    ) {
        if !present {
            self.push_issue(issues, component, ProductionWiringIssueKind::Missing, None);
        }
    }

    fn push_local_only(
        &self,
        issues: &mut Vec<ProductionWiringIssue>,
        component: ProductionWiringComponent,
        implementation: Option<ProductionComponentType>,
    ) {
        if let Some(implementation) = implementation {
            match implementation.readiness {
                ProductionImplementationReadiness::LocalOnly => self.push_issue(
                    issues,
                    component,
                    ProductionWiringIssueKind::LocalOnlyImplementation,
                    Some(implementation.implementation),
                ),
                ProductionImplementationReadiness::UnverifiedProductionImplementation => self
                    .push_issue(
                        issues,
                        component,
                        ProductionWiringIssueKind::UnverifiedProductionImplementation,
                        Some(implementation.implementation),
                    ),
                ProductionImplementationReadiness::ProductionCandidate => {}
            }
        }
    }

    fn push_issue(
        &self,
        issues: &mut Vec<ProductionWiringIssue>,
        component: ProductionWiringComponent,
        kind: ProductionWiringIssueKind,
        implementation: Option<&'static str>,
    ) {
        issues.push(ProductionWiringIssue {
            component,
            kind,
            implementation,
        });
    }

    /// Validates this graph and then builds the upper facade for production
    /// callers. This consumes the service graph so callers cannot mutate shared
    /// runtime-adapter handoff slots after validation.
    pub fn host_runtime_for_production(
        self,
        config: &ProductionWiringConfig,
    ) -> Result<DefaultHostRuntime, ProductionWiringReport> {
        self.validate_production_wiring(config)?;
        Ok(self.build_host_runtime())
    }

    /// Validates this graph and builds the production turn coordinator from
    /// the configured durable turn-state store and wake notifier. This keeps
    /// turn orchestration as an upper-layer artifact while still ensuring the
    /// same production guardrail validates the actual handles returned to
    /// callers.
    pub fn turn_coordinator_for_production(
        &self,
    ) -> Result<DefaultTurnCoordinator<dyn TurnStateStore>, ProductionWiringReport> {
        self.validate_production_turn_wiring()?;
        let Some(turn_state) = self.turn_state.as_ref() else {
            return Err(production_wiring_report(
                ProductionWiringComponent::TurnState,
                ProductionWiringIssueKind::Missing,
                None,
            ));
        };
        let Some(run_profile_resolver) = self.run_profile_resolver.as_ref() else {
            return Err(production_wiring_report(
                ProductionWiringComponent::RunProfileResolver,
                ProductionWiringIssueKind::Missing,
                None,
            ));
        };
        let Some(notifier) = self.turn_run_wake_notifier.as_ref() else {
            return Err(production_wiring_report(
                ProductionWiringComponent::TurnRunWakeNotifier,
                ProductionWiringIssueKind::Missing,
                None,
            ));
        };
        Ok(DefaultTurnCoordinator::new(Arc::clone(turn_state))
            .with_run_profile_resolver(Arc::clone(run_profile_resolver))
            .with_wake_notifier(Arc::clone(notifier)))
    }

    /// Validates turn persistence wiring and builds a scheduler over the
    /// configured trusted runner transition port. The concrete executor stays
    /// injected so product loop strategy remains above host-runtime.
    pub fn turn_scheduler_for_production(
        &self,
        executor: Arc<dyn TurnRunExecutor>,
        config: TurnRunSchedulerConfig,
    ) -> Result<TurnRunScheduler, ProductionWiringReport> {
        let mut issues = Vec::new();
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.turn_state.is_some(),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.component_types.turn_state,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.component_types.turn_run_transition_port,
        );
        if self.turn_run_transition_port.is_some()
            && !self.component_types.turn_run_transition_port_verified
        {
            self.push_issue(
                &mut issues,
                ProductionWiringComponent::TurnState,
                ProductionWiringIssueKind::UnverifiedProductionImplementation,
                component_name(self.component_types.turn_run_transition_port),
            );
        }
        if self.turn_run_transition_port.is_none() {
            self.push_issue(
                &mut issues,
                ProductionWiringComponent::TurnState,
                ProductionWiringIssueKind::UnsupportedRequirement,
                component_name(self.component_types.turn_state),
            );
        }
        if !issues.is_empty() {
            return Err(ProductionWiringReport { issues });
        }
        let Some(transition_port) = self.turn_run_transition_port.as_ref() else {
            return Err(production_wiring_report(
                ProductionWiringComponent::TurnState,
                ProductionWiringIssueKind::UnsupportedRequirement,
                component_name(self.component_types.turn_state),
            ));
        };
        Ok(TurnRunScheduler::new(
            Arc::clone(transition_port),
            executor,
            config,
        ))
    }

    fn validate_production_turn_wiring(&self) -> Result<(), ProductionWiringReport> {
        let mut issues = Vec::new();
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.turn_state.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::RunProfileResolver,
            self.run_profile_resolver.is_some(),
        );
        self.push_missing(
            &mut issues,
            ProductionWiringComponent::TurnRunWakeNotifier,
            self.turn_run_wake_notifier.is_some(),
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnState,
            self.component_types.turn_state,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::TurnRunWakeNotifier,
            self.component_types.turn_run_wake_notifier,
        );
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ProductionWiringReport { issues })
        }
    }

    /// Builds and attaches the configured Reborn durable event/audit stores,
    /// validates production wiring, and returns the host runtime facade.
    pub async fn host_runtime_for_production_with_event_store_config(
        self,
        event_store_config: RebornEventStoreConfig,
        production_config: &ProductionWiringConfig,
    ) -> Result<DefaultHostRuntime, ProductionEventStoreWiringError> {
        let services = self
            .with_reborn_event_store_config(RebornProfile::Production, event_store_config)
            .await?;
        Ok(services.host_runtime_for_production(production_config)?)
    }

    /// Builds a runtime dispatcher with every configured runtime adapter.
    fn runtime_dispatcher(&self) -> RuntimeDispatcher<'static, F, G> {
        let mut dispatcher = RuntimeDispatcher::from_arcs(
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
        );
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
    #[doc(hidden)]
    pub fn host_runtime_for_local_testing(&self) -> DefaultHostRuntime {
        self.build_host_runtime()
    }

    /// Builds the upper facade with the same dispatcher, process services,
    /// stores, cancellation registry, result store, and runtime health graph.
    fn build_host_runtime(&self) -> DefaultHostRuntime {
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
        let runtime_policy = self
            .runtime_policy
            .clone()
            .unwrap_or_else(local_testing_runtime_policy);

        let mut runtime = DefaultHostRuntime::new(
            Arc::clone(&self.registry),
            dispatcher,
            Arc::clone(&self.authorizer),
            self.surface_version.clone(),
            runtime_policy,
        )
        .with_trust_policy_dyn(Arc::clone(&self.trust_policy))
        .with_process_manager(process_manager)
        .with_process_store(process_store)
        .with_process_result_store(process_result_store)
        .with_process_cancellation_registry(self.process_services.cancellation_registry())
        .with_runtime_health(runtime_health);

        if let Some(run_state_approval_store) = &self.run_state_approval_store {
            runtime = runtime.with_run_state_approval_store(Arc::clone(run_state_approval_store));
        } else {
            if let Some(run_state) = &self.run_state {
                runtime = runtime.with_run_state(Arc::clone(run_state));
            }
            if let Some(approval_requests) = &self.approval_requests {
                runtime = runtime.with_approval_requests(Arc::clone(approval_requests));
            }
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
        if self.first_party_runtime_covers_declared_capabilities() {
            backends.push(RuntimeKind::FirstParty);
        }
        backends
    }

    fn first_party_runtime_covers_declared_capabilities(&self) -> bool {
        let Some(first_party_runtime) = &self.first_party_runtime else {
            return false;
        };
        let mut declared = self
            .registry
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
        self.registry.capabilities().any(|descriptor| {
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
        FilesystemBackendKind::HostWorkspace
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

fn production_wiring_report(
    component: ProductionWiringComponent,
    kind: ProductionWiringIssueKind,
    implementation: Option<&'static str>,
) -> ProductionWiringReport {
    ProductionWiringReport {
        issues: vec![ProductionWiringIssue {
            component,
            kind,
            implementation,
        }],
    }
}

fn runtime_http_egress(slot: &SharedRuntimeHttpEgress) -> Option<Arc<dyn RuntimeHttpEgress>> {
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

struct ServiceResolvedRuntimeAdapter<T> {
    inner: Arc<T>,
    invocation_services: Arc<dyn InvocationServicesResolver>,
}

// arch-exempt: large_file, runtime adapter composition is still centralized
// in HostRuntimeServices until the Reborn architecture decomposition tracked
// by nearai/ironclaw#3231 splits runtime wiring into focused modules.
impl<T> ServiceResolvedRuntimeAdapter<T> {
    fn new(inner: Arc<T>, invocation_services: Arc<dyn InvocationServicesResolver>) -> Self {
        Self {
            inner,
            invocation_services,
        }
    }
}

#[async_trait]
impl<F, G, T> RuntimeAdapter<F, G> for ServiceResolvedRuntimeAdapter<T>
where
    F: RootFilesystem,
    G: ResourceGovernor,
    T: RuntimeAdapter<F, G>,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let plan =
            plan_capability(request.descriptor, request.runtime_policy).map_err(|error| {
                release_adapter_reservation(
                    request.governor,
                    request
                        .resource_reservation
                        .as_ref()
                        .map(|reservation| reservation.id),
                );
                dispatch_error_for_runtime(request.descriptor.runtime, planner_error_kind(&error))
            })?;
        self.invocation_services
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &request.scope,
                mounts: request.mounts.as_ref(),
            })
            .map_err(|error| {
                release_adapter_reservation(
                    request.governor,
                    request
                        .resource_reservation
                        .as_ref()
                        .map(|reservation| reservation.id),
                );
                dispatch_error_for_runtime(request.descriptor.runtime, error.kind())
            })?;

        self.inner.dispatch_json(request).await
    }
}

#[derive(Clone)]
struct ScriptRuntimeAdapter {
    executor: Arc<dyn ScriptExecutor>,
}

impl ScriptRuntimeAdapter {
    fn from_executor(executor: Arc<dyn ScriptExecutor>) -> Self {
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
    fn from_executor(executor: Arc<dyn McpExecutor>) -> Self {
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

#[derive(Clone)]
struct FirstPartyRuntimeAdapter {
    registry: Arc<FirstPartyCapabilityRegistry>,
    invocation_services: Arc<dyn InvocationServicesResolver>,
}

impl FirstPartyRuntimeAdapter {
    pub(crate) fn from_registry(
        registry: Arc<FirstPartyCapabilityRegistry>,
        invocation_services: Arc<dyn InvocationServicesResolver>,
    ) -> Self {
        Self {
            registry,
            invocation_services,
        }
    }
}

#[async_trait]
impl<F, G> RuntimeAdapter<F, G> for FirstPartyRuntimeAdapter
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, F, G>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let Some(handler) = self.registry.get(request.capability_id) else {
            if let Some(reservation) = request.resource_reservation
                && let Err(error) = request.governor.release(reservation.id)
            {
                tracing::warn!(
                    reservation_id = %reservation.id,
                    error = %error,
                    "failed to release prepared resource reservation after missing first-party handler"
                );
            }
            return Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
            });
        };

        let plan =
            plan_capability(request.descriptor, request.runtime_policy).map_err(|error| {
                if let Some(reservation) = &request.resource_reservation {
                    release_first_party_reservation(request.governor, reservation.id);
                }
                DispatchError::FirstParty {
                    kind: planner_error_kind(&error),
                }
            })?;
        let services = self
            .invocation_services
            .resolve(InvocationServicesResolutionRequest {
                plan: &plan,
                scope: &request.scope,
                mounts: request.mounts.as_ref(),
            })
            .map_err(|error| {
                if let Some(reservation) = &request.resource_reservation {
                    release_first_party_reservation(request.governor, reservation.id);
                }
                DispatchError::FirstParty { kind: error.kind() }
            })?;

        let reservation = match request.resource_reservation {
            Some(reservation) => reservation,
            None => request
                .governor
                .reserve(request.scope.clone(), request.estimate.clone())
                .map_err(|_| DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Resource,
                })?,
        };

        let result = match AssertUnwindSafe(handler.dispatch(FirstPartyCapabilityRequest {
            capability_id: request.capability_id.clone(),
            scope: request.scope.clone(),
            estimate: request.estimate,
            mounts: request.mounts,
            services,
            input: request.input,
        }))
        .catch_unwind()
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                account_or_release_failed_first_party_execution(
                    request.governor,
                    reservation.id,
                    error.usage(),
                )?;
                return Err(DispatchError::FirstParty { kind: error.kind() });
            }
            Err(_) => {
                release_first_party_reservation(request.governor, reservation.id);
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Backend,
                });
            }
        };

        let output_bytes = serde_json::to_vec(&result.output)
            .map(|bytes| bytes.len() as u64)
            .map_err(|_| DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::OutputDecode,
            })?;
        let mut usage = result.usage;
        usage.output_bytes = usage.output_bytes.max(output_bytes);
        let receipt = match request.governor.reconcile(reservation.id, usage.clone()) {
            Ok(receipt) => receipt,
            Err(_) => {
                if let Err(release_error) = request.governor.release(reservation.id) {
                    tracing::warn!(
                        reservation_id = %reservation.id,
                        error = %release_error,
                        "failed to release first-party resource reservation after reconcile failure"
                    );
                }
                return Err(DispatchError::FirstParty {
                    kind: RuntimeDispatchErrorKind::Resource,
                });
            }
        };

        Ok(RuntimeAdapterResult {
            output: result.output,
            usage,
            receipt,
            output_bytes,
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
    pub(crate) fn new(
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

    pub(crate) fn try_new(
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
        let Some(policy) = self.network_policy_store.get(scope, capability_id) else {
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
            WasmRuntimeHttpAdapter::new(egress, scope.clone(), capability_id.clone(), policy)
                .with_policy_discarder(Arc::new(NetworkPolicyDiscarder {
                    store: Arc::clone(&self.network_policy_store),
                }));
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

#[derive(Debug, Clone)]
struct NetworkPolicyDiscarder {
    store: Arc<NetworkObligationPolicyStore>,
}

impl WasmRuntimePolicyDiscarder for NetworkPolicyDiscarder {
    fn discard(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
        self.store.discard_for_capability(scope, capability_id);
    }
}

#[derive(Clone)]
struct RuntimeDispatchProcessExecutor {
    dispatcher: Arc<dyn CapabilityDispatcher>,
}

impl RuntimeDispatchProcessExecutor {
    pub(crate) fn new(dispatcher: Arc<dyn CapabilityDispatcher>) -> Self {
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
                mounts: Some(request.mounts),
                resource_reservation: request.resource_reservation,
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

fn account_or_release_failed_first_party_execution<G>(
    governor: &G,
    reservation_id: ResourceReservationId,
    usage: Option<&ResourceUsage>,
) -> Result<(), DispatchError>
where
    G: ResourceGovernor + ?Sized,
{
    let Some(usage) = usage else {
        release_first_party_reservation(governor, reservation_id);
        return Ok(());
    };
    if !has_accountable_effects(usage) {
        release_first_party_reservation(governor, reservation_id);
        return Ok(());
    }

    if governor.reconcile(reservation_id, usage.clone()).is_err() {
        release_first_party_reservation(governor, reservation_id);
        return Err(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::Resource,
        });
    }

    Ok(())
}

fn release_first_party_reservation<G>(governor: &G, reservation_id: ResourceReservationId)
where
    G: ResourceGovernor + ?Sized,
{
    let _ = governor.release(reservation_id);
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

fn release_adapter_reservation<G>(governor: &G, reservation_id: Option<ResourceReservationId>)
where
    G: ResourceGovernor + ?Sized,
{
    let Some(reservation_id) = reservation_id else {
        return;
    };
    if let Err(error) = governor.release(reservation_id) {
        tracing::warn!(
            reservation_id = %reservation_id,
            error = %error,
            "failed to release prepared resource reservation after runtime policy rejection"
        );
    }
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

fn dispatch_error_for_runtime(
    runtime: RuntimeKind,
    kind: RuntimeDispatchErrorKind,
) -> DispatchError {
    match runtime {
        RuntimeKind::Mcp => DispatchError::Mcp { kind },
        RuntimeKind::Script => DispatchError::Script { kind },
        RuntimeKind::Wasm => DispatchError::Wasm { kind },
        RuntimeKind::FirstParty | RuntimeKind::System => DispatchError::FirstParty { kind },
    }
}

fn planner_error_kind(error: &PlannerError) -> RuntimeDispatchErrorKind {
    match error {
        PlannerError::ProcessEffectsRequiredButProcessBackendIsNone { .. } => {
            RuntimeDispatchErrorKind::UnsupportedRunner
        }
        PlannerError::NetworkRequiredButNetworkModeIsDeny { .. } => {
            RuntimeDispatchErrorKind::NetworkDenied
        }
        PlannerError::SecretAccessRequiredButSecretModeIsDeny { .. } => {
            RuntimeDispatchErrorKind::SecretDenied
        }
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
        | DispatchError::Wasm { kind }
        | DispatchError::FirstParty { kind } => kind.event_kind(),
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
mod tests {
    use std::sync::{Arc, Mutex};

    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extensions::{
        ExtensionManifest, ExtensionPackage, ExtensionRegistry, ManifestSource,
    };
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityId, EffectKind, HostPortCatalog, InvocationId,
        NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, PermissionMode,
        ResourceEstimate, ResourceReceipt, ResourceScope, RuntimeCredentialInjection,
        RuntimeCredentialSource, RuntimeCredentialTarget, RuntimeHttpEgressRequest, RuntimeKind,
        SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
    };
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
    use ironclaw_resources::{
        InMemoryResourceGovernor, ResourceAccount, ResourceGovernor, ResourceTally,
    };
    use ironclaw_secrets::{InMemorySecretStore, SecretMaterial};
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn host_http_egress_borrows_staged_policy_for_repeated_invocation_requests() {
        let network = RecordingNetwork::ok();
        let recorded_requests = Arc::clone(&network.requests);
        let services = test_services()
            .with_secret_store(Arc::new(InMemorySecretStore::new()))
            .try_with_host_http_egress(network)
            .expect("host HTTP egress should wire with graph secret store");
        let scope = sample_scope();
        let capability_id = sample_capability_id();
        let staged_policy = staged_policy();
        services
            .network_policy_store
            .insert(&scope, &capability_id, staged_policy.clone());
        let egress = configured_egress(&services);

        egress
            .execute(request_without_credentials(
                scope.clone(),
                capability_id.clone(),
            ))
            .expect("first request should observe staged policy");
        egress
            .execute(request_without_credentials(scope, capability_id))
            .expect("second request in same invocation should observe borrowed staged policy");

        let requests = recorded_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].policy, staged_policy);
        assert_eq!(requests[1].policy, staged_policy);
    }

    #[test]
    fn host_http_egress_helper_injects_staged_credentials_from_handoff_store() {
        let scope = sample_scope();
        let capability_id = sample_capability_id();
        let handle = SecretHandle::new("api-token").unwrap();

        let network = RecordingNetwork::ok();
        let recorded_requests = Arc::clone(&network.requests);
        let services = test_services()
            .with_secret_store(Arc::new(InMemorySecretStore::new()))
            .try_with_host_http_egress(network)
            .expect("host HTTP egress should wire with graph secret store");
        services
            .network_policy_store
            .insert(&scope, &capability_id, staged_policy());
        services
            .secret_injection_store
            .insert(
                &scope,
                &capability_id,
                &handle,
                SecretMaterial::from("staged-secret"),
            )
            .expect("staged credential should be seeded");
        let egress = configured_egress(&services);

        egress
            .execute(request_with_staged_credential(scope, capability_id, handle))
            .expect("StagedObligation should inject from handoff store");

        let requests = recorded_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .iter()
                .find(|(name, _)| name == "authorization"),
            Some(&(
                "authorization".to_string(),
                "Bearer staged-secret".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn service_guard_releases_reservation_on_planner_denial() {
        let inner = Arc::new(RecordingRuntimeAdapter::default());
        let adapter = ServiceResolvedRuntimeAdapter::new(
            Arc::clone(&inner),
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let estimate = ResourceEstimate {
            process_count: Some(1),
            ..ResourceEstimate::default()
        };
        let reservation = governor
            .reserve(scope.clone(), estimate.clone())
            .expect("test reservation should be created");
        let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
        assert_eq!(governor.reserved_for(&tenant_account).process_count, 1);

        let package = test_package(SCRIPT_MANIFEST, "test-script");
        let descriptor = test_descriptor(RuntimeKind::Script, vec![EffectKind::ExecuteCode]);
        let policy = policy_with(
            FilesystemBackendKind::ScopedVirtual,
            ProcessBackendKind::None,
            NetworkMode::Deny,
            SecretMode::Deny,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: Some(reservation),
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::Script {
                kind: RuntimeDispatchErrorKind::UnsupportedRunner
            })
        ));
        assert_eq!(inner.call_count(), 0);
        assert_eq!(
            governor.reserved_for(&tenant_account),
            ResourceTally::default()
        );
    }

    #[tokio::test]
    async fn service_guard_rejects_resolution_before_wasm_dispatch() {
        let inner = Arc::new(RecordingRuntimeAdapter::default());
        let adapter = ServiceResolvedRuntimeAdapter::new(
            Arc::clone(&inner),
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let estimate = ResourceEstimate::default();
        let package = test_package(WASM_MANIFEST, "test-wasm");
        let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::Network]);
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: None,
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::NetworkDenied
            })
        ));
        assert_eq!(inner.call_count(), 0);
    }

    #[tokio::test]
    async fn service_guard_releases_reservation_on_invocation_service_resolution_denial() {
        let inner = Arc::new(RecordingRuntimeAdapter::default());
        let adapter = ServiceResolvedRuntimeAdapter::new(
            Arc::clone(&inner),
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let estimate = ResourceEstimate {
            network_egress_bytes: Some(1),
            ..ResourceEstimate::default()
        };
        let reservation = governor
            .reserve(scope.clone(), estimate.clone())
            .expect("test reservation should be created");
        let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
        assert_eq!(
            governor.reserved_for(&tenant_account).network_egress_bytes,
            1
        );

        let package = test_package(WASM_MANIFEST, "test-wasm");
        let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::Network]);
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: Some(reservation),
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::NetworkDenied
            })
        ));
        assert_eq!(inner.call_count(), 0);
        assert_eq!(
            governor.reserved_for(&tenant_account),
            ResourceTally::default()
        );
    }

    #[tokio::test]
    async fn service_guard_rejects_required_secret_without_secret_store_before_dispatch() {
        let inner = Arc::new(RecordingRuntimeAdapter::default());
        let adapter = ServiceResolvedRuntimeAdapter::new(
            Arc::clone(&inner),
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let estimate = ResourceEstimate::default();
        let package = test_package(WASM_MANIFEST, "test-wasm");
        let descriptor = test_descriptor(RuntimeKind::Wasm, vec![EffectKind::UseSecret]);
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Deny,
            SecretMode::ScrubbedEnv,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: None,
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::Wasm {
                kind: RuntimeDispatchErrorKind::SecretDenied
            })
        ));
        assert_eq!(inner.call_count(), 0);
    }

    #[tokio::test]
    async fn first_party_adapter_releases_reservation_when_invocation_service_resolution_denies() {
        let descriptor = test_descriptor(RuntimeKind::FirstParty, vec![EffectKind::Network]);
        let registry = Arc::new(
            FirstPartyCapabilityRegistry::new()
                .with_handler(descriptor.id.clone(), Arc::new(PanicFirstPartyHandler)),
        );
        let adapter = FirstPartyRuntimeAdapter::from_registry(
            registry,
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
        let estimate = ResourceEstimate {
            network_egress_bytes: Some(1),
            ..ResourceEstimate::default()
        };
        let reservation = governor
            .reserve(scope.clone(), estimate.clone())
            .expect("test reservation should be created");
        assert_eq!(
            governor.reserved_for(&tenant_account).network_egress_bytes,
            1
        );
        let package = test_package(WASM_MANIFEST, "test-wasm");
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: Some(reservation),
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::NetworkDenied
            })
        ));
        assert_eq!(
            governor.reserved_for(&tenant_account),
            ResourceTally::default()
        );
    }

    #[tokio::test]
    async fn first_party_adapter_releases_reservation_when_planner_denies() {
        let descriptor = test_descriptor(RuntimeKind::FirstParty, vec![EffectKind::Network]);
        let registry = Arc::new(
            FirstPartyCapabilityRegistry::new()
                .with_handler(descriptor.id.clone(), Arc::new(PanicFirstPartyHandler)),
        );
        let adapter = FirstPartyRuntimeAdapter::from_registry(
            registry,
            Arc::new(LocalInvocationServicesResolver::new(
                Arc::new(LocalFilesystem::new()),
                None,
                Arc::new(LocalHostProcessPort::new()),
                None,
            )),
        );
        let filesystem = LocalFilesystem::new();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let tenant_account = ResourceAccount::tenant(scope.tenant_id.clone());
        let estimate = ResourceEstimate {
            network_egress_bytes: Some(1),
            ..ResourceEstimate::default()
        };
        let reservation = governor
            .reserve(scope.clone(), estimate.clone())
            .expect("test reservation should be created");
        assert_eq!(
            governor.reserved_for(&tenant_account).network_egress_bytes,
            1
        );
        let package = test_package(WASM_MANIFEST, "test-wasm");
        let policy = policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::Deny,
            SecretMode::ScrubbedEnv,
        );

        let result = adapter
            .dispatch_json(RuntimeAdapterRequest {
                package: &package,
                descriptor: &descriptor,
                filesystem: &filesystem,
                governor: &governor,
                runtime_policy: &policy,
                capability_id: &descriptor.id,
                scope,
                estimate,
                mounts: None,
                resource_reservation: Some(reservation),
                input: json!({}),
            })
            .await;

        assert!(matches!(
            result,
            Err(DispatchError::FirstParty {
                kind: RuntimeDispatchErrorKind::NetworkDenied
            })
        ));
        assert_eq!(
            governor.reserved_for(&tenant_account),
            ResourceTally::default()
        );
    }

    fn test_services() -> HostRuntimeServices<
        LocalFilesystem,
        InMemoryResourceGovernor,
        InMemoryProcessStore,
        InMemoryProcessResultStore,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        )
    }

    fn configured_egress<
        F: RootFilesystem + 'static,
        G: ResourceGovernor + 'static,
        S: ProcessStore + 'static,
        R: ProcessResultStore + 'static,
    >(
        services: &HostRuntimeServices<F, G, S, R>,
    ) -> Arc<dyn RuntimeHttpEgress> {
        services
            .runtime_http_egress
            .lock()
            .unwrap()
            .as_ref()
            .expect("runtime HTTP egress should be configured")
            .clone()
    }

    fn test_package(manifest: &str, extension_id: &str) -> ExtensionPackage {
        let manifest = ExtensionManifest::parse(
            manifest,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("test manifest should parse");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new(format!("/system/extensions/{extension_id}")).unwrap(),
        )
        .expect("test package should build")
    }

    fn test_descriptor(runtime: RuntimeKind, effects: Vec<EffectKind>) -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::new("test.capability").unwrap(),
            provider: ironclaw_host_api::ExtensionId::new("test").unwrap(),
            runtime,
            trust_ceiling: TrustClass::UserTrusted,
            description: "test capability".to_string(),
            parameters_schema: serde_json::Value::Null,
            effects,
            default_permission: PermissionMode::Allow,
            resource_profile: None,
        }
    }

    fn policy_with(
        filesystem_backend: FilesystemBackendKind,
        process_backend: ProcessBackendKind,
        network_mode: NetworkMode,
        secret_mode: SecretMode,
    ) -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend,
            process_backend,
            network_mode,
            secret_mode,
            approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskDestructive,
            audit_mode: ironclaw_host_api::runtime_policy::AuditMode::LocalMinimal,
        }
    }

    #[derive(Default)]
    struct RecordingRuntimeAdapter {
        calls: Mutex<usize>,
    }

    impl RecordingRuntimeAdapter {
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for RecordingRuntimeAdapter {
        async fn dispatch_json(
            &self,
            request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
        ) -> Result<RuntimeAdapterResult, DispatchError> {
            *self.calls.lock().unwrap() += 1;
            let usage = ResourceUsage::default();
            let reservation = match request.resource_reservation {
                Some(reservation) => reservation,
                None => request
                    .governor
                    .reserve(request.scope, request.estimate)
                    .map_err(|_| DispatchError::Wasm {
                        kind: RuntimeDispatchErrorKind::Resource,
                    })?,
            };
            let receipt: ResourceReceipt = request
                .governor
                .reconcile(reservation.id, usage.clone())
                .map_err(|_| DispatchError::Wasm {
                    kind: RuntimeDispatchErrorKind::Resource,
                })?;
            Ok(RuntimeAdapterResult {
                output: Value::Null,
                usage,
                receipt,
                output_bytes: 0,
            })
        }
    }

    struct PanicFirstPartyHandler;

    #[async_trait]
    impl crate::FirstPartyCapabilityHandler for PanicFirstPartyHandler {
        async fn dispatch(
            &self,
            _request: crate::FirstPartyCapabilityRequest,
        ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
            panic!("service-resolution denial should happen before handler dispatch")
        }
    }

    fn request_without_credentials(
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) -> RuntimeHttpEgressRequest {
        RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            capability_id,
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: caller_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        }
    }

    fn request_with_staged_credential(
        scope: ResourceScope,
        capability_id: CapabilityId,
        handle: SecretHandle,
    ) -> RuntimeHttpEgressRequest {
        RuntimeHttpEgressRequest {
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id.clone(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            ..request_without_credentials(scope, capability_id)
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_capability_id() -> CapabilityId {
        CapabilityId::new("runtime.http").unwrap()
    }

    fn staged_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(4096),
        }
    }

    fn caller_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "caller.example.test".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: false,
            max_egress_bytes: Some(1),
        }
    }

    const SCRIPT_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test-script"
name = "Test Script"
version = "0.1.0"
description = "Script test extension"
trust = "untrusted"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "test-script.run"
description = "Run script"
effects = ["execute_code"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-script/run.input.v1.json"
output_schema_ref = "schemas/test-script/run.output.v1.json"
prompt_doc_ref = "prompts/test-script/run.md"
"#;

    const WASM_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test-wasm"
name = "Test Wasm"
version = "0.1.0"
description = "WASM test extension"
trust = "untrusted"

[runtime]
kind = "wasm"
module = "test.wasm"

[[capabilities]]
id = "test-wasm.run"
description = "Run WASM"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-wasm/run.input.v1.json"
output_schema_ref = "schemas/test-wasm/run.output.v1.json"
prompt_doc_ref = "prompts/test-wasm/run.md"
"#;

    #[derive(Clone)]
    struct RecordingNetwork {
        requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
    }

    impl RecordingNetwork {
        fn ok() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl NetworkHttpEgress for RecordingNetwork {
        fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.requests.lock().unwrap().push(request);
            Ok(NetworkHttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"ok":true}"#.to_vec(),
                usage: NetworkUsage {
                    request_bytes: 0,
                    response_bytes: 11,
                    resolved_ip: None,
                },
            })
        }
    }
}
