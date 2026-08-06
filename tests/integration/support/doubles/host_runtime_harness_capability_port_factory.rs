/// Test double substituting the production `LoopCapabilityPortFactory` wiring:
/// `RefreshingLoopCapabilityPortFactory` (`crates/app/ironclaw_composition/src/runtime/capability_host.rs`)
/// and `HostRuntimeLoopCapabilityPortFactory` (`crates/loop/ironclaw_loop_host/src/capability_port.rs`).
use std::sync::Arc;

use async_trait::async_trait;
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
            )
            .await
    }
}
