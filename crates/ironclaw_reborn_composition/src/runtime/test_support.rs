// Test-support accessors mirroring `build_reborn_runtime`'s approval/auth
// interaction-service wiring, for harnesses that build their own planned
// runtime and bypass `build_reborn_runtime` (W5-WEBUI-API-2).
//
// Lives under `crate::runtime` (not `factory.rs`) — the recipe needs
// module-private types only reachable from here.

use super::*;

fn build_approval_interaction_service_with_parts(
    parts: &InteractionServiceTestParts,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    turn_run_source: Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    let approval_turn_runs = Arc::new(SnapshotApprovalTurnRunLocator::new(turn_run_source));
    let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
        parts.approval_requests.clone(),
        approval_turn_runs,
    ));
    let approval_resolver = Arc::new(ApprovalResolverPort::new(
        parts.approval_requests.clone(),
        parts.capability_leases.clone(),
    ));
    let persistent_approval_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore> =
        parts.persistent_approval_policies.clone();
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
        parts.tool_permission_overrides.clone();

    Ok(Arc::new(
        DefaultApprovalInteractionService::new(
            approval_read_model,
            Arc::new(approval::PolicyApprovalLeaseTermsProvider::new(
                Arc::clone(&parts.builtin_capability_policy),
                Arc::clone(&parts.extension_registry),
                parts.workspace_mounts.clone(),
                parts.skill_mounts.clone(),
                parts.memory_mounts.clone(),
                parts.system_extensions_lifecycle_mounts.clone(),
                extension_surface::ExtensionCapabilitySurfaceSource::new(Some({
                    let mut facade = crate::extension_host::lifecycle::LifecycleFacade::new(
                        Arc::clone(&parts.skill_management),
                    )
                    .with_extension_management(Arc::clone(&parts.extension_management))
                    .with_admin_configuration_resolver(Arc::clone(
                        &parts.admin_configuration_resolver,
                    ))
                    .with_runtime_credential_accounts(
                        parts
                            .product_auth
                            .runtime_credential_account_selection_service(),
                    );
                    if let Some(egress) = parts.runtime_http_egress.as_ref() {
                        facade = facade.with_runtime_http_egress(Arc::clone(egress));
                    }
                    Arc::new(facade)
                })),
            )),
            approval_resolver,
            turn_coordinator,
        )
        .with_persistent_policy_store(persistent_approval_policies)
        .with_persistent_grantee_resolver(Arc::new(RegistryPersistentApprovalGranteeResolver::new(
            Arc::clone(&parts.extension_registry),
        )?))
        .with_tool_permission_override_store(tool_permission_overrides),
    ))
}

impl RebornRuntime {
    /// Real approval interaction service owned by this runtime.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError> {
        let Some(parts) = self.interaction_service_test_parts.as_ref() else {
            return Ok(Some(Arc::clone(&self.approval_interaction_service)));
        };
        build_approval_interaction_service_with_parts(
            parts,
            turn_coordinator,
            Arc::clone(&self.turn_run_snapshot_source),
        )
        .map(Some)
    }

    /// Auth-interaction service owned by this runtime.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_auth_interaction_service_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Option<Arc<dyn AuthInteractionService>> {
        Some(build_webui_auth_interaction_service_with_turn_run_source(
            self.product_auth.as_ref(),
            Arc::clone(&self.turn_run_snapshot_source),
            turn_coordinator,
        ))
    }

    /// Like [`local_dev_approval_interaction_service_for_test`], but lets
    /// the caller substitute the turn-run snapshot source the interaction
    /// service's approval locator reads from — for harnesses whose real runs
    /// live in a DIFFERENT `TurnStateStore` composition than this
    /// this runtime's own turn state (e.g.
    /// `RebornIntegrationGroup`, whose runs execute against its own
    /// `shared.turn_store` via a separate `build_default_planned_runtime`).
    /// Generic over `F` so any `FilesystemTurnStateRowStore<F>`-backed store can be
    /// passed directly, without this crate exposing `TurnRunSnapshotSource`
    /// outside itself.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in
    /// production builds.
    ///
    /// [`local_dev_approval_interaction_service_for_test`]: Self::local_dev_approval_interaction_service_for_test
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_with_turn_state_for_test<F>(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        turn_state: Arc<ironclaw_turns::FilesystemTurnStateRowStore<F>>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        let Some(parts) = self.interaction_service_test_parts.as_ref() else {
            return Ok(None);
        };
        build_approval_interaction_service_with_parts(
            parts,
            turn_coordinator,
            turn_state as Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>,
        )
        .map(Some)
    }

    /// Auth-side counterpart of
    /// [`local_dev_approval_interaction_service_with_turn_state_for_test`]. See
    /// that method's doc for why the turn-state override exists.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in
    /// production builds.
    ///
    /// [`local_dev_approval_interaction_service_with_turn_state_for_test`]: Self::local_dev_approval_interaction_service_with_turn_state_for_test
    #[cfg(feature = "test-support")]
    pub fn local_dev_auth_interaction_service_with_turn_state_for_test<F>(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        turn_state: Arc<ironclaw_turns::FilesystemTurnStateRowStore<F>>,
    ) -> Option<Arc<dyn AuthInteractionService>>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        Some(build_webui_auth_interaction_service_with_turn_run_source(
            self.product_auth.as_ref(),
            turn_state as Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>,
            turn_coordinator,
        ))
    }
}
