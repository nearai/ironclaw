/// Test double substituting the production `LoopCapabilityPortFactory` wiring
/// (`RefreshingLoopCapabilityPortFactory` / `HostRuntimeLoopCapabilityPortFactory`)
/// for the Echo (`RecordingTestCapabilityPort`) backend.
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_loop_host::LoopCapabilityPortFactory;
use ironclaw_turns::run_profile::{AgentLoopHostError, LoopCapabilityPort, LoopRunContext};

use super::recording_test_capability_port::RecordingTestCapabilityPort;

pub(crate) struct HarnessCapabilityPortFactory {
    pub(crate) port: Arc<RecordingTestCapabilityPort>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(self.port.clone())
    }
}
