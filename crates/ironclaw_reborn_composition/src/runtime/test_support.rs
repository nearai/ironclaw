// Test-support accessors mirroring `build_reborn_runtime`'s approval/auth
// interaction-service wiring, for harnesses that build their own planned
// runtime and bypass `build_reborn_runtime` (W5-WEBUI-API-2).
//
// Lives under `crate::runtime` (not `factory.rs`) — the recipe needs
// module-private types only reachable from here.

use super::*;

impl RebornRuntime {
    /// Real approval interaction service owned by this runtime.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_for_test(
        &self,
        _turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError> {
        if self.approval_requests.is_none() {
            return Ok(None);
        }
        Ok(Some(Arc::clone(&self.approval_interaction_service)))
    }

    /// Auth-interaction service owned by this runtime.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_auth_interaction_service_for_test(
        &self,
        _turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Option<Arc<dyn AuthInteractionService>> {
        self.approval_requests.as_ref()?;
        Some(Arc::clone(&self.auth_interaction_service))
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
        _turn_coordinator: Arc<dyn TurnCoordinator>,
        _turn_state: Arc<ironclaw_turns::FilesystemTurnStateRowStore<F>>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        if self.approval_requests.is_none() {
            return Ok(None);
        }
        Ok(Some(Arc::clone(&self.approval_interaction_service)))
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
        _turn_coordinator: Arc<dyn TurnCoordinator>,
        _turn_state: Arc<ironclaw_turns::FilesystemTurnStateRowStore<F>>,
    ) -> Option<Arc<dyn AuthInteractionService>>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        self.approval_requests.as_ref()?;
        Some(Arc::clone(&self.auth_interaction_service))
    }
}
