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
use ironclaw_loop_support::HostManagedModelGateway;

/// Test-support mirror of production local-dev capability wiring. Exposes only
/// trait-object seams so integration harnesses can drive the same refreshing
/// capability port production uses without naming local-dev internals.
#[cfg(feature = "test-support")]
pub struct LocalDevCapabilityWiringForTest {
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

impl RebornServices {
    /// Real `DefaultApprovalInteractionService` wired like `build_reborn_runtime`, via the
    /// shared `build_local_dev_approval_interaction_service` recipe so the two never drift.
    /// `Ok(None)` without a local-dev runtime; `Err` surfaces a local-dev capability-policy
    /// or grantee-resolver construction failure instead of collapsing it into `None`. No
    /// audit sink threaded — production wires one for audit-log observability only, not
    /// correctness the test needs.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_interaction_service_for_test(
        &self,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError> {
        let Some(local_runtime) = self.local_runtime.as_ref() else {
            return Ok(None);
        };
        let local_dev_capability_policy =
            Arc::new(local_dev_capability_policy().map_err(|error| {
                RebornRuntimeError::InvalidArgument {
                    reason: format!("local-dev capability policy is invalid: {error}"),
                }
            })?);
        Ok(Some(build_local_dev_approval_interaction_service(
            local_runtime,
            local_dev_capability_policy,
            turn_coordinator,
            None,
        )?))
    }

    /// WebUI auth-interaction service via the same `build_webui_auth_interaction_service`
    /// helper `build_reborn_runtime` uses. `None` only without a local-dev runtime; falls
    /// back to `UnavailableAuthInteractionService` if `product_auth` has no flow-record source.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in production builds.
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

    /// Like [`local_dev_approval_interaction_service_for_test`], but lets
    /// the caller substitute the turn-run snapshot source the interaction
    /// service's approval locator reads from — for harnesses whose real runs
    /// live in a DIFFERENT `TurnStateStore` composition than this
    /// `RebornServices`' own `local_runtime.turn_state` (e.g.
    /// `RebornIntegrationGroup`, whose runs execute against its own
    /// `shared.turn_store` via a separate `build_default_planned_runtime`).
    /// Generic over `F` so any `FilesystemTurnStateStore<F>`-backed store can be
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
        turn_state: Arc<ironclaw_turns::FilesystemTurnStateStore<F>>,
    ) -> Result<Option<Arc<dyn ApprovalInteractionService>>, RebornRuntimeError>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        let Some(local_runtime) = self.local_runtime.as_ref() else {
            return Ok(None);
        };
        let local_dev_capability_policy =
            Arc::new(local_dev_capability_policy().map_err(|error| {
                RebornRuntimeError::InvalidArgument {
                    reason: format!("local-dev capability policy is invalid: {error}"),
                }
            })?);
        Ok(Some(
            build_local_dev_approval_interaction_service_with_turn_run_source(
                local_runtime,
                local_dev_capability_policy,
                turn_coordinator,
                None,
                turn_state as Arc<dyn TurnRunSnapshotSource>,
            )?,
        ))
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
        turn_state: Arc<ironclaw_turns::FilesystemTurnStateStore<F>>,
    ) -> Option<Arc<dyn AuthInteractionService>>
    where
        F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
    {
        self.local_runtime.as_ref()?;
        Some(build_webui_auth_interaction_service_with_turn_run_source(
            self.product_auth.as_deref(),
            turn_state as Arc<dyn TurnRunSnapshotSource>,
            turn_coordinator,
        ))
    }

    /// Build the same local-dev refreshing capability wiring
    /// `build_reborn_runtime` uses, but over a caller-owned thread service and
    /// model gateway. Integration groups use this to keep their shared planned
    /// runtime while exercising production `RefreshingLocalDevCapabilityPort`
    /// refreshes instead of the static harness allowlist path.
    ///
    /// For tests only -- gated behind `test-support`, ships zero bytes in
    /// production builds.
    #[cfg(feature = "test-support")]
    pub fn local_dev_capability_wiring_for_test(
        &self,
        thread_service: Arc<dyn SessionThreadService>,
        fallback_user_id: UserId,
        model_gateway: Arc<dyn HostManagedModelGateway>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Result<Option<LocalDevCapabilityWiringForTest>, RebornRuntimeError> {
        let Some(_) = self.local_runtime.as_ref() else {
            return Ok(None);
        };
        let local_dev_capability_policy =
            Arc::new(local_dev_capability_policy().map_err(|error| {
                RebornRuntimeError::InvalidArgument {
                    reason: format!("local-dev capability policy is invalid: {error}"),
                }
            })?);
        Ok(local_dev::capability_wiring(
            self,
            thread_service,
            fallback_user_id,
            local_dev_capability_policy,
            model_gateway,
            milestone_sink,
            None,
            None,
            None,
        )
        .map(|wiring| LocalDevCapabilityWiringForTest {
            capability_factory: wiring.capability_factory,
            capability_input_resolver: wiring.capability_input_resolver,
            capability_result_writer: wiring.capability_result_writer,
        }))
    }
}
