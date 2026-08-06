use std::sync::Arc;

use super::LibSqlRootFilesystem;
use super::PostgresRootFilesystem;
use super::{
    AgentTurnRuntimePort, ApprovalRequestStore, ApprovalRequestStorePort, AuditSink,
    CapabilityLeaseStorePort, CoalescingEventSink, DurableAuditLog, DurableAuditSink,
    DurableEventLog, DurableEventSink, EffectiveRuntimePolicy, EventBatchConfig, EventSink,
    FilesystemResourceGovernor, FirstPartyCapabilityRegistry, HostRuntimeServices, McpExecutor,
    NetworkHttpEgress, ProcessBackendKind, ProcessExecutor, ProcessInvocationStatePort,
    ProductionComponentType, ProductionImplementationReadiness, ProductionWiringComponent,
    ProductionWiringIssueKind, ProductionWiringReport, RebornEventStoreConfig,
    RebornEventStoreError, RebornEventStores, RebornProfile, ResourceGovernor, RootFilesystem,
    RunProfileResolver, RuntimeBackendHealth, RuntimeCredentialAccountResolver, RuntimeHttpEgress,
    RuntimeKind, RuntimeProcessPort, ScopedFilesystem, ScriptExecutor, SecretMode, SecretStorePort,
    SecurityAuditSink, SharedSecretStore, TenantSandboxProcessPort, TrustPolicy,
    TurnRunWakeNotifier, WasmError, WasmRuntimeAdapter, WasmRuntimeCredentialProvider,
    WasmStagedRuntimeCredentials, WitToolHost, WitToolRuntimeConfig, build_reborn_event_stores,
    production_wiring_report, set_runtime_http_egress, set_tool_call_http_egress,
};
use crate::HostProcessPort;
use crate::RuntimeHttpBodyStore;
use crate::http_body::UnsupportedRuntimeHttpBodyStore;
use crate::wasm_credentials::SharedHostWasmRuntimeCredentials;
use ironclaw_secrets::{CredentialAccountStore, CredentialSessionStore};

impl<F, G> HostRuntimeServices<F, G>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
{
    fn with_root_filesystem<T>(self, filesystem: Arc<T>) -> HostRuntimeServices<T, G>
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
            invocation_state,
            approval_requests,
            capability_leases,
            persistent_approval_policies,
            event_sink,
            audit_sink,
            security_audit_sink,
            secret_store,
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            tool_call_http_egress,
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
            turn_run_wake_notifier,
            extension_tool_resolver,
            post_edit_check,
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
            invocation_state,
            approval_requests,
            capability_leases,
            persistent_approval_policies,
            event_sink,
            audit_sink,
            security_audit_sink,
            secret_store,
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            tool_call_http_egress,
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
            turn_run_wake_notifier,
            extension_tool_resolver,
            post_edit_check,
            component_types,
        }
    }

    pub fn with_postgres_root_filesystem(
        self,
        filesystem: Arc<PostgresRootFilesystem>,
    ) -> HostRuntimeServices<PostgresRootFilesystem, G> {
        self.with_root_filesystem(filesystem)
    }

    pub fn with_libsql_root_filesystem(
        self,
        filesystem: Arc<LibSqlRootFilesystem>,
    ) -> HostRuntimeServices<LibSqlRootFilesystem, G> {
        self.with_root_filesystem(filesystem)
    }

    pub fn with_resource_governor<T>(self, governor: Arc<T>) -> HostRuntimeServices<F, T>
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
            invocation_state,
            approval_requests,
            capability_leases,
            persistent_approval_policies,
            event_sink,
            audit_sink,
            security_audit_sink,
            secret_store,
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            tool_call_http_egress,
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
            turn_run_wake_notifier,
            extension_tool_resolver,
            post_edit_check,
            mut component_types,
        } = self;
        let lifecycle_governor: Arc<dyn ResourceGovernor> = governor.clone();
        process_lifecycle_store.set_resource_governor(lifecycle_governor);
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
            invocation_state,
            approval_requests,
            capability_leases,
            persistent_approval_policies,
            event_sink,
            audit_sink,
            security_audit_sink,
            secret_store,
            credential_account_store,
            credential_session_store,
            runtime_credential_account_resolver,
            network_policy_store,
            secret_injection_store,
            process_lifecycle_store,
            runtime_http_egress,
            tool_call_http_egress,
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
            turn_run_wake_notifier,
            extension_tool_resolver,
            post_edit_check,
            component_types,
        }
    }

    /// Replace the in-memory governor with the journaled filesystem-backed
    /// [`FilesystemResourceGovernor`] over the supplied [`ScopedFilesystem`].
    /// Backend choice (libSQL, Postgres, in-memory, local disk) is a property
    /// of the underlying [`RootFilesystem`](ironclaw_filesystem::RootFilesystem);
    /// see `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
    pub fn with_filesystem_resource_governor<FsBackend>(
        self,
        scoped_filesystem: Arc<ScopedFilesystem<FsBackend>>,
    ) -> HostRuntimeServices<F, FilesystemResourceGovernor<FsBackend>>
    where
        FsBackend: RootFilesystem + 'static,
    {
        self.with_resource_governor(Arc::new(FilesystemResourceGovernor::new(scoped_filesystem)))
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

    pub fn with_invocation_state<T>(mut self, invocation_state: Arc<T>) -> Self
    where
        T: ProcessInvocationStatePort + 'static,
    {
        self.component_types.invocation_state = Some(ProductionComponentType::of::<T>());
        self.invocation_state = Some(invocation_state);
        self
    }

    pub fn with_approval_requests<T>(mut self, approval_requests: Arc<T>) -> Self
    where
        T: ApprovalRequestStorePort + 'static,
    {
        self.component_types.approval_requests = Some(ProductionComponentType::of::<T>());
        self.approval_requests = Some(approval_requests);
        self
    }

    /// Builds and attaches journal-backed invocation state plus a
    /// filesystem-backed approval-request store.
    ///
    /// The process journal owns invocation lifecycle. The scoped filesystem is
    /// used only for approval records. The backend choice
    /// (`LibSqlRootFilesystem`, `PostgresRootFilesystem`,
    /// `InMemoryBackend`, …) happens at the `RootFilesystem` layer, not here.
    ///
    /// CapabilityHost uses the two-step
    /// `ApprovalRequestStorePort::save_pending` then
    /// `ProcessInvocationStatePort::block_approval` path in
    /// `ironclaw_capabilities::host`, with rollback if the process transition
    /// fails.
    pub fn with_process_journal_invocation_state<FsBackend>(
        self,
        process_runtime: Arc<dyn ironclaw_processes::ProcessRuntimePort>,
        scoped_filesystem: Arc<ScopedFilesystem<FsBackend>>,
    ) -> Self
    where
        FsBackend: RootFilesystem + 'static,
    {
        let invocation_state = Arc::new(ironclaw_processes::ProcessInvocationStore::new(
            process_runtime,
        ));
        let approval_requests = Arc::new(ApprovalRequestStore::new(scoped_filesystem));
        self.with_invocation_state(invocation_state)
            .with_approval_requests(approval_requests)
    }

    pub fn with_capability_leases<T>(mut self, capability_leases: Arc<T>) -> Self
    where
        T: CapabilityLeaseStorePort + 'static,
    {
        self.component_types.capability_leases = Some(ProductionComponentType::of::<T>());
        self.capability_leases = Some(capability_leases);
        self
    }

    pub fn with_persistent_approval_policies<T>(mut self, policies: Arc<T>) -> Self
    where
        T: ironclaw_approvals::PersistentApprovalPolicyStorePort + 'static,
    {
        self.component_types.persistent_approval_policies =
            Some(ProductionComponentType::of::<T>());
        self.persistent_approval_policies = Some(policies);
        self
    }

    pub fn with_turn_state<T>(mut self, turn_state: Arc<T>) -> Self
    where
        T: AgentTurnRuntimePort + 'static,
    {
        self.component_types.turn_state = Some(ProductionComponentType::of::<T>());
        self.turn_state = Some(turn_state);
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
    pub fn with_turn_run_wake_notifier<T>(mut self, notifier: Arc<T>) -> Self
    where
        T: TurnRunWakeNotifier + 'static,
    {
        self.component_types.turn_run_wake_notifier = Some(ProductionComponentType::of::<T>());
        self.turn_run_wake_notifier = Some(notifier);
        self
    }

    pub fn with_turn_run_wake_notifier_dyn(
        mut self,
        notifier: Arc<dyn TurnRunWakeNotifier>,
    ) -> Self {
        self.component_types.turn_run_wake_notifier = Some(ProductionComponentType::named(
            "dyn TurnRunWakeNotifier",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
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

    /// Wire in a [`SecurityAuditSink`] so security-boundary decisions inside
    /// the built-in obligation handler are recorded instead of dropped.
    pub fn with_security_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: SecurityAuditSink + 'static,
    {
        self.security_audit_sink = Some(sink);
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
    /// `ironclaw_event_store`, then this method adapts the durable logs
    /// into the live sink traits consumed by runtime services.
    pub fn with_reborn_event_stores(self, stores: RebornEventStores) -> Self {
        self.with_reborn_event_stores_verified(stores, false)
    }

    /// Attaches pre-built Reborn durable event/audit stores after the caller
    /// has already enforced production profile restrictions.
    pub fn with_production_reborn_event_stores(self, stores: RebornEventStores) -> Self {
        self.with_reborn_event_stores_verified(stores, true)
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
            // Prebuilt/Standalone/Test stores are useful for tests and lower-level
            // composition, but must not silently satisfy production guardrails.
            self.component_types.event_sink =
                Some(ProductionComponentType::of::<DurableEventSink>());
            self.component_types.audit_sink =
                Some(ProductionComponentType::of::<DurableAuditSink>());
        }
        // Runtime events are best-effort observability whose append cursor is
        // discarded at the sink, so route them through the write-behind
        // coalescing sink: a per-turn burst of single-row INSERTs collapses to
        // one multi-row INSERT per stream per drain window. The compliance
        // audit log stays synchronous.
        self.event_sink = Some(Arc::new(CoalescingEventSink::new(
            stores.events,
            EventBatchConfig::default(),
        )));
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
        T: SecretStorePort + 'static,
    {
        self.component_types.secret_store = Some(ProductionComponentType::of::<T>());
        self.secret_store = Some(secret_store);
        self
    }

    pub fn with_secret_store_dyn(mut self, secret_store: Arc<dyn SecretStorePort>) -> Self {
        self.component_types.secret_store = Some(ProductionComponentType::named(
            "dyn SecretStorePort",
            ProductionImplementationReadiness::ProductionCandidate,
        ));
        self.secret_store = Some(secret_store);
        self
    }

    pub(crate) fn with_credential_account_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: CredentialAccountStore + 'static,
    {
        self.component_types.credential_account_store = Some(ProductionComponentType::of::<T>());
        self.credential_account_store = store;
        self
    }

    pub(crate) fn with_credential_session_store<T>(mut self, store: Arc<T>) -> Self
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

    /// Attaches strict runtime HTTP egress only.
    ///
    /// This port keeps generic [`RuntimeHttpEgress`] response-limit semantics:
    /// response body limit overruns remain errors. First-party `builtin.http`
    /// inline output also needs [`crate::ToolCallHttpEgress`]; use
    /// [`Self::with_first_party_http_egress`] when one service should satisfy
    /// both ports.
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

    /// Attaches one HTTP service to both the strict runtime and model-visible
    /// first-party tool-call egress ports.
    ///
    /// This is the intended test/local composition helper for `builtin.http`:
    /// strict callers still use [`RuntimeHttpEgress`], while inline tool output
    /// goes through [`crate::ToolCallHttpEgress`] for sanitized partial response
    /// handling.
    pub fn with_first_party_http_egress<T>(self, http_egress: Arc<T>) -> Self
    where
        T: RuntimeHttpEgress + crate::ToolCallHttpEgress + 'static,
    {
        self.with_runtime_http_egress(Arc::clone(&http_egress))
            .with_tool_call_http_egress(http_egress)
    }

    /// Attaches model-visible HTTP egress for first-party tool calls.
    ///
    /// Use this when the tool-call path intentionally differs from the strict
    /// runtime HTTP path, such as tests that assert `builtin.http.save` does not
    /// route through model-visible output handling.
    pub fn with_tool_call_http_egress<T>(self, tool_call_http_egress: Arc<T>) -> Self
    where
        T: crate::ToolCallHttpEgress + 'static,
    {
        let tool_call_http_egress: Arc<dyn crate::ToolCallHttpEgress> = tool_call_http_egress;
        set_tool_call_http_egress(&self.tool_call_http_egress, tool_call_http_egress);
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

    /// Configure the operator post-edit check appended to successful
    /// `builtin.write_file` / `builtin.apply_patch` output. Composition
    /// resolves the config once (see `PostEditCheckConfig::from_env`) and
    /// threads it here; the feature stays off when this is never called.
    pub fn with_post_edit_check(mut self, post_edit_check: crate::PostEditCheckConfig) -> Self {
        self.post_edit_check = Some(post_edit_check);
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
    pub(crate) fn with_host_http_egress_service<N, SecretBackend>(
        mut self,
        runtime_http_egress: Arc<crate::HostHttpEgressService<N, SecretBackend>>,
    ) -> Self
    where
        N: NetworkHttpEgress + 'static,
        SecretBackend: SecretStorePort + 'static,
    {
        self.component_types.runtime_http_egress = Some(ProductionComponentType::of::<
            crate::HostHttpEgressService<N, SecretBackend>,
        >());
        self.component_types.runtime_http_egress_verified = runtime_http_egress
            .is_production_wired_with(&self.network_policy_store, &self.secret_injection_store);
        let tool_call_http_egress: Arc<dyn crate::ToolCallHttpEgress> = runtime_http_egress.clone();
        let runtime_http_egress: Arc<dyn RuntimeHttpEgress> = runtime_http_egress;
        set_runtime_http_egress(&self.runtime_http_egress, runtime_http_egress);
        set_tool_call_http_egress(&self.tool_call_http_egress, tool_call_http_egress);
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
            ProductionComponentType::of::<HostProcessPort>();
        self.process_port = if matches!(policy.secret_mode, SecretMode::InheritedEnv) {
            tracing::warn!(
                host_access = "full-local",
                "runtime policy selected inherited local host process environment"
            );
            Arc::new(HostProcessPort::new_inherited_env())
        } else {
            Arc::new(HostProcessPort::new())
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
                ProductionWiringComponent::SecretStorePort,
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
            let mut provider = SharedHostWasmRuntimeCredentials::new((*self.registry).clone());
            if let (Some(secret_store), Some(account_resolver)) = (
                self.secret_store.clone(),
                self.runtime_credential_account_resolver.clone(),
            ) {
                provider = provider.with_product_auth_restaging(
                    secret_store,
                    Arc::clone(&self.secret_injection_store),
                    account_resolver,
                );
            }
            let provider = Arc::new(provider);
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
