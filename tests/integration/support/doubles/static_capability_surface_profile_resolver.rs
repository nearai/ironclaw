use async_trait::async_trait;
use ironclaw_loop_contracts::LoopRunContext;
use ironclaw_loop_host::{
    CapabilityResolveError, CapabilitySurfacePolicy, CapabilitySurfaceProfileResolver,
};

pub(crate) struct StaticCapabilitySurfaceProfileResolver {
    pub(crate) policy: CapabilitySurfacePolicy,
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceProfileResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilitySurfacePolicy, CapabilityResolveError> {
        Ok(self.policy.clone())
    }
}
