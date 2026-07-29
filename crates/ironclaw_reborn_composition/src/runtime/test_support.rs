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
    turn_run_source: Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = TurnError>>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    let approval_turn_runs = Arc::new(ProcessGateApprovalTurnRunLocator::new(turn_run_source));
    let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
        parts.approval_requests.clone(),
        approval_turn_runs,
    ));
    let approval_resolver = Arc::new(ApprovalResolverPort::new(
        parts.approval_requests.clone(),
        parts.capability_leases.clone(),
    ));
    let persistent_approval_policies: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
    > = parts.persistent_approval_policies.clone();
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
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
                local_dev::extension_surface::ExtensionCapabilitySurfaceSource::new(Some(
                    Arc::clone(&parts.extension_management),
                )),
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
            Arc::clone(&self._process_gate_query_source),
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
            Arc::clone(&self._process_gate_query_source),
            turn_coordinator,
        ))
    }

    /// Like [`local_dev_approval_interaction_service_for_test`], but lets
    /// harnesses substitute the process gate source that owns their runs.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in
    /// production builds.
    ///
    /// [`local_dev_approval_interaction_service_for_test`]: Self::local_dev_approval_interaction_service_for_test
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_with_process_gates_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        process_gates: Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = TurnError>>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError> {
        let Some(parts) = self.interaction_service_test_parts.as_ref() else {
            return Ok(None);
        };
        build_approval_interaction_service_with_parts(parts, turn_coordinator, process_gates)
            .map(Some)
    }

    /// Auth-side counterpart of
    /// [`local_dev_approval_interaction_service_with_process_gates_for_test`].
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in
    /// production builds.
    ///
    /// [`local_dev_approval_interaction_service_with_process_gates_for_test`]: Self::local_dev_approval_interaction_service_with_process_gates_for_test
    #[cfg(feature = "test-support")]
    pub fn local_dev_auth_interaction_service_with_process_gates_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
        process_gates: Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = TurnError>>,
    ) -> Option<Arc<dyn AuthInteractionService>> {
        Some(build_webui_auth_interaction_service_with_turn_run_source(
            self.product_auth.as_ref(),
            process_gates,
            turn_coordinator,
        ))
    }
}
