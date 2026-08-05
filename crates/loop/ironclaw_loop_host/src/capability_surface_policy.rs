use async_trait::async_trait;
use ironclaw_host_api::capability_surface::CapabilitySurfacePolicy;
use ironclaw_loop_contracts::LoopRunContext;
use thiserror::Error;

#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CapabilityResolveError {
    #[error("capability surface profile is unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("capability surface profile could not be resolved: {reason}")]
    Internal { reason: String },
}

impl CapabilityResolveError {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub fn internal(reason: impl Into<String>) -> Self {
        Self::Internal {
            reason: reason.into(),
        }
    }
}

#[async_trait]
pub trait CapabilitySurfaceProfileResolver: Send + Sync {
    async fn resolve(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<CapabilitySurfacePolicy, CapabilityResolveError>;
}
