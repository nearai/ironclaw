use std::sync::Arc;

use super::{
    DefaultHostRuntime, DefaultTurnCoordinator, HostRuntimeServices, ProcessBackendKind,
    ProcessResultStore, ProcessStore, ProductionComponentType, ProductionEventStoreWiringError,
    ProductionImplementationReadiness, ProductionWiringComponent, ProductionWiringConfig,
    ProductionWiringIssue, ProductionWiringIssueKind, ProductionWiringReport,
    RebornEventStoreConfig, RebornProfile, ResourceGovernor, RootFilesystem, RuntimeKind,
    TurnRunExecutor, TurnRunScheduler, TurnRunSchedulerConfig, TurnStateStore, component_name,
    local_only_runtime_policy_reason, production_wiring_report, runtime_http_egress_is_configured,
};

impl<F, G, S, R> HostRuntimeServices<F, G, S, R>
where
    F: RootFilesystem + 'static,
    G: ResourceGovernor + 'static,
    S: ProcessStore + 'static,
    R: ProcessResultStore + 'static,
{
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
            ProductionWiringComponent::PersistentApprovalPolicies,
            self.persistent_approval_policies.is_some(),
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
        if config.require_credential_broker {
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::CredentialAccountStore,
                self.component_types.credential_account_store.is_some(),
            );
            self.push_missing(
                &mut issues,
                ProductionWiringComponent::CredentialSessionStore,
                self.component_types.credential_session_store.is_some(),
            );
        }

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
            ProductionWiringComponent::PersistentApprovalPolicies,
            self.component_types.persistent_approval_policies,
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
            ProductionWiringComponent::CredentialAccountStore,
            self.component_types.credential_account_store,
        );
        self.push_local_only(
            &mut issues,
            ProductionWiringComponent::CredentialSessionStore,
            self.component_types.credential_session_store,
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
}
