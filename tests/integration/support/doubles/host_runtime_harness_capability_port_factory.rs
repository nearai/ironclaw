/// Test double substituting the production `LoopCapabilityPortFactory` wiring:
/// `RefreshingLoopCapabilityPortFactory` (`crates/ironclaw_composition/src/runtime/standalone.rs`)
/// and `HostRuntimeLoopCapabilityPortFactory` (`crates/ironclaw_loop_host/src/capability_port.rs`).
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_loop_contracts::{AgentLoopHostError, LoopCapabilityPort, LoopRunContext};
use ironclaw_loop_host::LoopCapabilityPortFactory;

use super::super::harness::HostRuntimeCapabilityHarness;

pub(crate) struct HostRuntimeHarnessCapabilityPortFactory {
    pub(crate) harness: Arc<HostRuntimeCapabilityHarness>,
    pub(crate) milestone_sink: Arc<ironclaw_loop_contracts::InMemoryLoopHostMilestoneSink>,
    pub(crate) trajectory_observer: Option<Arc<dyn ironclaw_composition::RebornTrajectoryObserver>>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeHarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.harness
            .create_recording_capability_port(
                run_context,
                &self.milestone_sink,
                self.trajectory_observer.clone(),
                CapabilitySurfacePolicy::allow_all(),
            )
            .await
    }

    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.harness
            .create_recording_capability_port(
                run_context,
                &self.milestone_sink,
                self.trajectory_observer.clone(),
                surface_policy.as_ref().clone(),
            )
            .await
    }
}
