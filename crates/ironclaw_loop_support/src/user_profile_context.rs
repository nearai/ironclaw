//! Host source for the per-user agent-context profile, read at loop start.
//!
//! Mirrors `HostIdentityContextSource`: the trait lives here so the loop driver
//! depends only on a neutral port, while the concrete implementation (which
//! reads `context/profile.json` from the memory backend) lives in
//! `ironclaw_host_runtime`, keeping `ironclaw_memory` out of `ironclaw_reborn`.

use async_trait::async_trait;
use ironclaw_turns::run_profile::{LoopRunContext, UserProfileContext};

/// Resolves the per-user agent-context profile for a run. Returns the validated
/// `UserProfileContext` (timezone/locale/location), or `None` when no profile is
/// set or it cannot be resolved. Implementations must never fabricate values
/// (e.g. a guessed timezone) — fail to `None` instead.
#[async_trait]
pub trait HostUserProfileSource: Send + Sync {
    async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext>;
}

/// Default no-op source: always `None`. Used when no profile source is wired.
#[derive(Debug, Default, Clone)]
pub struct EmptyUserProfileSource;

#[async_trait]
impl HostUserProfileSource for EmptyUserProfileSource {
    async fn resolve_user_profile(
        &self,
        _run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        None
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, RunProfileResolutionRequest},
    };

    use super::*;

    /// Build a sample `LoopRunContext` the same way `identity_context.rs` tests do —
    /// using `InMemoryRunProfileResolver` and constructing a minimal `TurnScope`.
    async fn sample_run_context() -> LoopRunContext {
        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        let scope = TurnScope::new(
            TenantId::new("tenant-user-profile-test").unwrap(),
            None,
            None,
            ThreadId::new("thread-user-profile-test").unwrap(),
        );
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    #[tokio::test]
    async fn empty_source_returns_none() {
        let run_context = sample_run_context().await;
        assert!(
            EmptyUserProfileSource
                .resolve_user_profile(&run_context)
                .await
                .is_none()
        );
    }
}
