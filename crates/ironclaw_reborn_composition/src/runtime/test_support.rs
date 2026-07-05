// Test-support accessors mirroring `build_reborn_runtime`'s approval/auth
// interaction-service wiring, for harnesses that build their own planned
// runtime and bypass `build_reborn_runtime` (W5-WEBUI-API-2).
//
// `turn_coordinator` is caller-supplied, never `self.turn_coordinator`: a
// `RebornServices` from `build_reborn_services` alone carries the coordinator
// minted in `build_local_runtime`, not the caller's own planned-runtime one.
//
// Lives under `crate::runtime` (not `factory.rs`) — the recipe needs
// module-private types only reachable from here.

use super::*;

impl RebornServices {
    /// Real `DefaultApprovalInteractionService` wired like `build_reborn_runtime`.
    /// `None` without a local-dev runtime. No audit sink threaded — production wires
    /// one for audit-log observability only, not correctness the test needs.
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Option<Arc<dyn ApprovalInteractionService>> {
        let local_runtime = self.local_runtime.as_ref()?;
        let local_dev_capability_policy = Arc::new(local_dev_capability_policy().ok()?);
        let approval_turn_runs = Arc::new(LocalDevApprovalTurnRunLocator::new(Arc::clone(
            &local_runtime.turn_state,
        )));
        let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
            local_runtime.approval_requests.clone(),
            approval_turn_runs,
        ));
        let approval_resolver = Arc::new(ApprovalResolverPort::new(
            local_runtime.approval_requests.clone(),
            local_runtime.capability_leases.clone(),
        ));
        Some(Arc::new(
            DefaultApprovalInteractionService::new(
                approval_read_model,
                Arc::new(approval::LocalDevApprovalLeaseTermsProvider::new(
                    local_dev_capability_policy,
                    Arc::clone(&local_runtime.extension_registry),
                    local_runtime.workspace_mounts.clone(),
                    local_runtime.skill_mounts.clone(),
                    local_runtime.memory_mounts.clone(),
                    local_runtime.system_extensions_lifecycle_mounts.clone(),
                    local_dev::extension_surface::LocalDevExtensionSurfaceSource::new(
                        local_runtime.extension_management.clone(),
                    ),
                )),
                approval_resolver,
                turn_coordinator,
            )
            .with_persistent_policy_store(local_runtime.persistent_approval_policies.clone())
            .with_persistent_grantee_resolver(Arc::new(
                RegistryPersistentApprovalGranteeResolver::new(Arc::clone(
                    &local_runtime.extension_registry,
                ))
                .ok()?,
            ))
            .with_tool_permission_override_store(local_runtime.tool_permission_overrides.clone()),
        ))
    }

    /// WebUI auth-interaction service via the same `build_webui_auth_interaction_service`
    /// helper `build_reborn_runtime` uses. `None` only without a local-dev runtime; falls
    /// back to `UnavailableAuthInteractionService` if `product_auth` has no flow-record source.
    #[cfg(feature = "test-support")]
    pub fn local_dev_auth_interaction_service_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Option<Arc<dyn AuthInteractionService>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(build_webui_auth_interaction_service(
            self.product_auth.as_deref(),
            Arc::clone(&local_runtime.turn_state),
            turn_coordinator,
        ))
    }
}
