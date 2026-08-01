use async_trait::async_trait;
use ironclaw_loop_contracts::LoopRunContext;
use ironclaw_loop_host::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};

pub(crate) struct StaticCapabilitySurfaceProfileResolver {
    pub(crate) allow_set: CapabilityAllowSet,
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceProfileResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(self.allow_set.clone())
    }
}
