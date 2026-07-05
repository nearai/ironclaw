//! Test-only accessor for the real Slack host-beta pairing/binding store
//! (W5-SLACK-PAIR Enabler §1).
//!
//! `FilesystemSlackHostState` is the one production implementation of
//! `SlackPersonalBindingPairingChallengeStore` / `RebornUserIdentityBindingStore`
//! / `RebornUserIdentityLookup` (see `slack_host_beta.rs`'s wiring, which clones
//! one instance as all three trait objects). Its only prior constructors were
//! the crate-tier `state()`/`state_with_root()` test helpers in
//! `slack_host_state.rs`, both private to that file. This accessor mirrors
//! them so int-tier tests outside this crate (`tests/integration/slack_pairing_*.rs`)
//! can drive the real store behind `SlackPersonalUserBindingService` /
//! `SlackPersonalBindingPairingService` / `SlackPairingActorResolver` instead
//! of a fake, without duplicating the CAS/pairing/binding wiring.
//!
//! Callers own the `ScopedFilesystem`'s mount view — a single covering
//! `/tenant-shared` grant (read/write/list/delete) is sufficient, matching the
//! crate-tier `state_with_backend` helper's own fixed view; production's
//! multi-grant `slack_host_state_mount_view` splits permissions further for
//! reasons unrelated to `FilesystemSlackHostState`'s own path usage.
//!
//! Gated behind `test-support`; ships zero bytes in production.

/// Construct the real `FilesystemSlackHostState` over an already-mounted
/// `filesystem`. Returns the concrete type (not a boxed trait object) so a
/// test can upcast to whichever of `SlackPersonalBindingPairingChallengeStore`
/// / `RebornUserIdentityBindingStore` / `RebornUserIdentityLookup` it needs via
/// `.clone() as Arc<dyn _>`, exactly as production composition wiring does at
/// `slack_host_beta.rs`.
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
/// TTL, for tests that must exercise real expiry (e.g. mint a code with a
/// millisecond TTL, sleep past it, then redeem) without waiting out the
/// production 10-minute default. Mirrors the crate-tier
/// `filesystem_slack_host_state_rejects_expired_pairing_code` test's own
/// `.with_pairing_ttl(..)` use.
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
