use std::any::type_name;

use std::sync::Arc;

#[cfg(feature = "libsql")]
use super::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use super::PostgresRootFilesystem;
use super::{
    ApprovalRequestStore, AuditSink, CapabilityLeaseStore, DurableAuditLog, DurableAuditSink,
    DurableEventLog, DurableEventSink, EffectiveRuntimePolicy, EventSink,
    FilesystemApprovalRequestStore, FilesystemResourceGovernorStore, FilesystemRunStateStore,
    FilesystemTurnStateStore, FirstPartyCapabilityRegistry, HostRuntimeServices, McpExecutor,
    NetworkHttpEgress, PersistentResourceGovernor, ProcessBackendKind, ProcessExecutor,
    ProcessObligationLifecycleStore, ProcessResultStore, ProcessStore, ProductionComponentType,
    ProductionImplementationReadiness, ProductionWiringComponent, ProductionWiringIssueKind,
    ProductionWiringReport, RebornEventStoreConfig, RebornEventStoreError, RebornEventStores,
    RebornProfile, ResourceGovernor, RootFilesystem, RunProfileResolver, RunStateApprovalStore,
    RunStateStore, RuntimeBackendHealth, RuntimeCredentialAccountResolver, RuntimeHttpEgress,
    RuntimeKind, RuntimeProcessPort, ScopedFilesystem, ScriptExecutor, SecretMode, SecretStore,
    SharedSecretStore, TenantSandboxProcessPort, TrustPolicy, TurnRunTransitionPort,
    TurnRunWakeNotifier, TurnStateStore, WasmError, WasmRuntimeAdapter,
    WasmRuntimeCredentialProvider, WasmStagedRuntimeCredentials, WitToolHost, WitToolRuntimeConfig,
    build_reborn_event_stores, production_wiring_report, set_runtime_http_egress,
};
use crate::LocalHostProcessPort;
use crate::RuntimeHttpBodyStore;
use crate::http_body::UnsupportedRuntimeHttpBodyStore;
use crate::wasm_credentials::SharedHostWasmRuntimeCredentials;
use ironclaw_secrets::{CredentialAccountStore, CredentialSessionStore};

impl<F, G, S, R> HostRuntimeServices<F, G, S, R>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
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
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            managed_process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            process_sandbox_executor,
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
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            managed_process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            process_sandbox_executor,
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
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store: _,
            runtime_http_egress,
            process_port,
            managed_process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            process_sandbox_executor,
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
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            process_port,
            managed_process_port,
            tenant_sandbox_process_port,
            wasm_credential_provider,
            runtime_health,
            runtime_policy,
            process_sandbox_executor,
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

    pub fn with_secret_store_dyn(mut self, secret_store: Arc<dyn SecretStore>) -> Self {
        self.component_types.secret_store = Some(ProductionComponentType::named(
            "dyn SecretStore",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
        self.secret_store = Some(secret_store);
        self
    }

    pub fn with_credential_account_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: CredentialAccountStore + 'static,
    {
        self.component_types.credential_account_store = Some(ProductionComponentType::of::<T>());
        self.credential_account_store = store;
        self
    }

    pub fn with_credential_session_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: CredentialSessionStore + 'static,
    {
        self.component_types.credential_session_store = Some(ProductionComponentType::of::<T>());
        self.credential_session_store = store;
        self
    }

    pub fn with_runtime_credential_account_resolver<T>(mut self, resolver: Arc<T>) -> Self
    where
        T: RuntimeCredentialAccountResolver + 'static,
    {
        let resolver: Arc<dyn RuntimeCredentialAccountResolver> = resolver;
        self.runtime_credential_account_resolver = Some(resolver);
        self
    }

    pub fn with_credential_broker<T>(self, broker: Arc<T>) -> Self
    where
        T: CredentialAccountStore + CredentialSessionStore + 'static,
    {
        self.with_credential_account_store(Arc::clone(&broker))
            .with_credential_session_store(broker)
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
        self.managed_process_port = false;
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
        self.managed_process_port = false;
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

    pub fn with_production_tenant_sandbox_process_port(
        mut self,
        process_port: Arc<TenantSandboxProcessPort>,
    ) -> Self {
        self.component_types.tenant_sandbox_process_port = Some(ProductionComponentType::named(
            "TenantSandboxProcessPort",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
        self.tenant_sandbox_process_port = Some(process_port);
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

    pub fn with_process_sandbox_executor<T>(mut self, executor: Arc<T>) -> Self
    where
        T: ProcessExecutor + 'static,
    {
        self.process_sandbox_executor = Some(executor);
        self
    }

    pub fn with_runtime_policy(mut self, policy: EffectiveRuntimePolicy) -> Self {
        self.apply_local_process_policy(&policy);
        self.runtime_policy = Some(policy);
        self
    }

    fn apply_local_process_policy(&mut self, policy: &EffectiveRuntimePolicy) {
        if !self.managed_process_port {
            return;
        }
        if !matches!(policy.process_backend, ProcessBackendKind::LocalHost) {
            return;
        }
        self.component_types.runtime_process_port =
            ProductionComponentType::of::<LocalHostProcessPort>();
        self.process_port = if matches!(policy.secret_mode, SecretMode::InheritedEnv) {
            tracing::warn!(
                host_access = "full-local",
                "runtime policy selected inherited local host process environment"
            );
            Arc::new(LocalHostProcessPort::new_inherited_env())
        } else {
            Arc::new(LocalHostProcessPort::new())
        };
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

    fn with_manifest_wasm_runtime_credentials(
        mut self,
        provider: Arc<SharedHostWasmRuntimeCredentials>,
        has_current_manifest_credentials: bool,
    ) -> Self {
        self.component_types.wasm_credential_provider = Some(ProductionComponentType::of::<
            SharedHostWasmRuntimeCredentials,
        >());
        self.component_types.wasm_credential_provider_verified = has_current_manifest_credentials;
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
        self.try_with_host_http_egress_internal(network, Arc::new(UnsupportedRuntimeHttpBodyStore))
    }

    pub fn try_with_host_http_egress_with_body_store<N, T>(
        self,
        network: N,
        body_store: Arc<T>,
    ) -> Result<Self, ProductionWiringReport>
    where
        N: NetworkHttpEgress + 'static,
        T: RuntimeHttpBodyStore + 'static,
    {
        let body_store: Arc<dyn RuntimeHttpBodyStore> = body_store;
        self.try_with_host_http_egress_internal(network, body_store)
    }

    fn try_with_host_http_egress_internal<N>(
        self,
        network: N,
        body_store: Arc<dyn RuntimeHttpBodyStore>,
    ) -> Result<Self, ProductionWiringReport>
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
        let service = crate::HostHttpEgressService::production(
            network,
            SharedSecretStore(secret_store),
            Arc::clone(&self.network_policy_store),
            Arc::clone(&self.secret_injection_store),
            body_store,
        )
        .with_unsafe_raw_diagnostics_allowed(
            crate::runtime_policy_allows_unsafe_raw_http_diagnostics(self.runtime_policy.as_ref()),
        );
        let runtime_http_egress = Arc::new(service);
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
        mut self,
        config: WitToolRuntimeConfig,
        host: WitToolHost,
    ) -> Result<Self, WasmError> {
        if self.wasm_credential_provider.is_none() {
            let registry = self.registry.snapshot();
            let has_current_manifest_credentials = registry.capabilities().any(|descriptor| {
                descriptor.runtime == RuntimeKind::Wasm
                    && !descriptor.runtime_credentials.is_empty()
            });
            let provider = Arc::new(SharedHostWasmRuntimeCredentials::new(
                (*self.registry).clone(),
            ));
            self = self
                .with_manifest_wasm_runtime_credentials(provider, has_current_manifest_credentials);
        }
        let adapter = Arc::new(WasmRuntimeAdapter::try_new(
            config,
            host,
            Arc::clone(&self.network_policy_store),
            Arc::clone(&self.runtime_http_egress),
            self.wasm_credential_provider.clone(),
        )?);
        Ok(self.with_wasm_runtime(adapter))
    }

    pub fn try_with_default_wasm_runtime(self) -> Result<Self, WasmError> {
        self.try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    }
}
