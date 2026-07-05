//! Test-only accessor for the real Slack host-beta pairing/binding store
//! (W5-SLACK-PAIR Enabler §1) — mirrors the crate-private `state()`/
//! `state_with_root()` helpers so int-tier tests outside this crate can
//! drive the real store instead of a fake. Gated behind `test-support`;
//! ships zero bytes in production.

/// Construct the real `FilesystemSlackHostState` over an already-mounted
/// `filesystem`. Returns the concrete type so callers can upcast to whichever
/// trait (`...ChallengeStore`/`...BindingStore`/`...Lookup`) they need, as
/// production wiring does at `slack_host_beta.rs`.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
pub fn slack_host_state_for_test<F>(
    filesystem: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    user_id: ironclaw_host_api::UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
) -> std::sync::Arc<crate::slack_host_state::FilesystemSlackHostState<F>>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    std::sync::Arc::new(crate::slack_host_state::FilesystemSlackHostState::new(
        filesystem, tenant_id, user_id, agent_id, project_id,
    ))
}

/// Same as [`slack_host_state_for_test`] but with a caller-chosen pairing-code
/// TTL, for tests exercising real expiry without the production 10-minute
/// default.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
pub fn slack_host_state_for_test_with_pairing_ttl<F>(
    filesystem: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    user_id: ironclaw_host_api::UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
    pairing_ttl: std::time::Duration,
) -> std::sync::Arc<crate::slack_host_state::FilesystemSlackHostState<F>>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    std::sync::Arc::new(
        crate::slack_host_state::FilesystemSlackHostState::new(
            filesystem, tenant_id, user_id, agent_id, project_id,
        )
        .with_pairing_ttl(pairing_ttl),
    )
}
