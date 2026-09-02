//! Test-only production external-tool decorator wiring for integration groups.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopCapabilityPort, LoopRunContext,
};
use ironclaw_loop_host::{
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
    wrap_external_tools,
};

pub(crate) struct ExternalToolCapabilityPortFactory {
    pub(crate) inner: Arc<dyn LoopCapabilityPortFactory>,
    pub(crate) catalog: Arc<dyn ironclaw_turns::ExternalToolCatalog>,
    pub(crate) specs: Vec<ironclaw_turns::ExternalToolSpec>,
    pub(crate) input_resolver: Arc<dyn LoopCapabilityInputResolver>,
    pub(crate) result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

impl ExternalToolCapabilityPortFactory {
    async fn wrap_for_run(
        &self,
        run_context: &LoopRunContext,
        inner: Arc<dyn LoopCapabilityPort>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        self.catalog
            .register(run_context.run_id, self.specs.clone())
            .await
            .map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!("external-tool test registration failed: {error}"),
                )
            })?;
        Ok(wrap_external_tools(
            inner,
            run_context.clone(),
            Arc::clone(&self.input_resolver),
            Arc::clone(&self.result_writer),
            Arc::clone(&self.catalog),
        ))
    }
}

#[async_trait]
impl LoopCapabilityPortFactory for ExternalToolCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let inner = self.inner.create_capability_port(run_context).await?;
        self.wrap_for_run(run_context, inner).await
    }

    async fn create_capability_port_with_surface_policy(
        &self,
        run_context: &LoopRunContext,
        surface_policy: Arc<CapabilitySurfacePolicy>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let inner = self
            .inner
            .create_capability_port_with_surface_policy(run_context, surface_policy)
            .await?;
        self.wrap_for_run(run_context, inner).await
    }
}
