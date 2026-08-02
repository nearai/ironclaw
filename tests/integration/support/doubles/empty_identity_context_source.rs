use async_trait::async_trait;
use ironclaw_loop_contracts::{LoopRunContext, PromptMode};
use ironclaw_loop_host::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
};

pub(crate) struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}
