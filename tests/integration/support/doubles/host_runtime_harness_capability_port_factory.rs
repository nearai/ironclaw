/// Test double substituting the production `LoopCapabilityPortFactory` wiring:
/// `RefreshingLoopCapabilityPortFactory` (`crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`)
/// and `HostRuntimeLoopCapabilityPortFactory` (`crates/ironclaw_loop_host/src/capability_port.rs`).
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_host::LoopCapabilityPortFactory;
use ironclaw_turns::run_profile::{AgentLoopHostError, LoopCapabilityPort, LoopRunContext};

use super::super::harness::HostRuntimeCapabilityHarness;

pub(crate) struct HostRuntimeHarnessCapabilityPortFactory {
    pub(crate) harness: Arc<HostRuntimeCapabilityHarness>,
    pub(crate) milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeHarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.harness
            .create_recording_capability_port(run_context, &self.milestone_sink)
            .await
    }
}
