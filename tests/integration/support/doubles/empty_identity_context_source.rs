use async_trait::async_trait;
use ironclaw_loop_host::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
};
use ironclaw_turns::run_profile::{LoopRunContext, PromptMode};

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
