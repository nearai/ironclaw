use async_trait::async_trait;

use crate::RunProfileResolver;

/// Pre-resolved run-profile resolver used to thread the resolver result
/// *into* the apply closure. The resolver future runs once per submit call
/// outside the CAS loop because resolving may issue I/O the closure shouldn't
/// carry; the resolution outcome is then constant for the retry loop.
#[derive(Clone)]
pub(super) struct PreResolvedRunProfileResolver {
    result: Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError>,
}

impl PreResolvedRunProfileResolver {
    pub(super) fn new(
        result: Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError>,
    ) -> Self {
        Self { result }
    }
}

#[async_trait]
impl RunProfileResolver for PreResolvedRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        _request: crate::RunProfileResolutionRequest,
    ) -> Result<crate::ResolvedRunProfile, crate::RunProfileResolutionError> {
        self.result.clone()
    }
}
