/// Test double substituting the production `LoopCapabilityPort` produced by
/// `HostRuntimeLoopCapabilityPortFactory` (`crates/loop/ironclaw_loop_host/src/capability_port.rs`).
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::resolution::{Resolution, ResolutionBatch};
use ironclaw_loop_contracts::{
    AgentLoopHostError, CapabilityCallCandidate, LoopCapabilityPort, LoopRequest, LoopRequestBatch,
    ProviderToolCall, ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
};

pub(crate) struct RecordingDelegatingCapabilityPort {
    pub(crate) inner: Arc<dyn LoopCapabilityPort>,
    pub(crate) invocations: Arc<Mutex<Vec<LoopRequest>>>,
}

#[async_trait]
impl LoopCapabilityPort for RecordingDelegatingCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ironclaw_loop_contracts::ProviderToolCallCapabilityIds, AgentLoopHostError> {
        // MUST delegate to inner. The `LoopCapabilityPort` default resolves a
        // call by searching `self.tool_definitions()` (the disclosed/advertised
        // surface), which rejects every name that resolves only at the port
        // boundary — deferred/disclosed tools, synthetic capabilities, and
        // other inner-resolvable names — with "outside the visible capability
        // surface" before the inner snapshot can resolve it. This is the model
        // gateway's resolvability pre-check, so it must reach inner (same
        // reason the surface-tracking wrapper in `ironclaw_turn_runner`
        // delegates).
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        request: ironclaw_loop_contracts::RegisterProviderToolCallRequest,
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
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.invocations.lock().unwrap().push(request.clone());
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        self.invocations
            .lock()
            .unwrap()
            .extend(request.invocations.iter().cloned());
        self.inner.invoke_capability_batch(request).await
    }
}
