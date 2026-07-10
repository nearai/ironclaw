/// Test double substituting the production `LoopCapabilityPortFactory` wiring:
/// `LocalDevLoopCapabilityPortFactory` (`crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`)
/// and `HostRuntimeLoopCapabilityPortFactory` (`crates/ironclaw_loop_support/src/capability_port.rs`).
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_support::LoopCapabilityPortFactory;
use ironclaw_turns::run_profile::{AgentLoopHostError, LoopCapabilityPort, LoopRunContext};

use super::{super::harness::HostRuntimeCapabilityHarness, RecordingDelegatingCapabilityPort};

pub(crate) struct HostRuntimeHarnessCapabilityPortFactory {
    pub(crate) harness: Arc<HostRuntimeCapabilityHarness>,
    pub(crate) milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    pub(crate) inner: Option<Arc<dyn LoopCapabilityPortFactory>>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeHarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        if let Some(inner) = &self.inner {
            let port = inner.create_capability_port(run_context).await?;
            return Ok(Arc::new(RecordingDelegatingCapabilityPort {
                inner: port,
                invocations: self.harness.invocations_handle_for_test(),
            }));
        }
        self.harness
            .create_recording_capability_port(run_context, &self.milestone_sink)
            .await
    }
}
