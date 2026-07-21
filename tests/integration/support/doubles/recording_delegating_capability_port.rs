/// Test double substituting the production `LoopCapabilityPort` produced by
/// `HostRuntimeLoopCapabilityPortFactory` (`crates/ironclaw_loop_host/src/capability_port.rs`).
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{Resolution, ResolutionBatch};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityInvocation,
    LoopCapabilityPort, ProviderToolCall, ProviderToolDefinition, VisibleCapabilityRequest,
    VisibleCapabilitySurface,
};

pub(crate) struct RecordingDelegatingCapabilityPort {
    pub(crate) inner: Arc<dyn LoopCapabilityPort>,
    pub(crate) invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
}

#[async_trait]
impl LoopCapabilityPort for RecordingDelegatingCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: ironclaw_turns::run_profile::RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(request).await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.invocations.lock().unwrap().push(request.clone());
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        self.invocations
            .lock()
            .unwrap()
            .extend(request.invocations.iter().cloned());
        self.inner.invoke_capability_batch(request).await
    }
}
