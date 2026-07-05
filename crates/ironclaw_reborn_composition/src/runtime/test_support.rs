// Test-support accessors mirroring the WebUI approval/auth interaction-service
// assembly above (`build_reborn_runtime`'s `approval_interaction_service` /
// `auth_interaction_service` locals). A harness that builds its own planned
// runtime directly (e.g. via `build_default_planned_runtime`, bypassing
// `build_reborn_runtime`) has no other way to get a REAL
// `DefaultApprovalInteractionService` / auth-interaction-service pair instead
// of the default `Rejecting*InteractionService` — these two methods close that
// gap for RESOLVE_GATE coverage on both gate kinds (W5-WEBUI-API-2).
//
// `turn_coordinator` is a caller-supplied parameter, never `self.turn_coordinator`:
// a `RebornServices` built by `build_reborn_services` alone carries the earlier
// coordinator minted inside `build_local_runtime` (`factory.rs:1587`), which is
// NOT the same instance as whatever planned-runtime coordinator the caller's own
// turns actually run against. Pass the exact `Arc<dyn TurnCoordinator>` your
// harness's turns are driven by.
//
// This file lives inside the `crate::runtime` module tree (declared via
// `#[path = "runtime/test_support.rs"]` in `runtime.rs`) rather than in
// `factory.rs`, because the recipe it mirrors depends on module-private types
// (`LocalDevApprovalTurnRunLocator`, `RegistryPersistentApprovalGranteeResolver`,
// `approval::LocalDevApprovalLeaseTermsProvider`,
// `local_dev::extension_surface::LocalDevExtensionSurfaceSource`,
// `build_webui_auth_interaction_service`) that are only reachable from code
// inside `crate::runtime` and its descendant modules — see the enabler plan's
// §0 for the full module-privacy trace.

use super::*;

impl RebornServices {
    /// Test-support access to a real `DefaultApprovalInteractionService` wired
    /// exactly as `build_reborn_runtime` wires it, over this `RebornServices`'s
    /// own local-dev parts. Returns `None` for production-profile compositions
    /// without a local-dev runtime (mirrors every other `local_dev_..._for_test`
    /// accessor in `factory.rs`). Does not thread an audit sink (production
    /// wires one purely for audit-log observability, not correctness — a test
    /// asserting resolve/deny behavior doesn't need it; add one later if a test
    /// needs to assert on the audit trail).
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

    /// Test-support access to the WebUI auth-interaction service, built via the
    /// same `build_webui_auth_interaction_service` helper `build_reborn_runtime`
    /// uses (reused, not reimplemented). Returns `None` only when this
    /// composition has no local-dev runtime at all; if a local-dev runtime IS
    /// present but `product_auth` has no wired flow-record source, the returned
    /// service is `UnavailableAuthInteractionService` (matches
    /// `build_webui_auth_interaction_service`'s own fail-closed shape — a real,
    /// if inert, trait object, not a sentinel the caller has to special-case).
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
