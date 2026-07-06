//! Test-only accessor for the real Slack host-beta pairing/binding store
//! (W5-SLACK-PAIR Enabler §1) — mirrors the crate-private `state()`/
//! `state_with_root()` helpers so int-tier tests outside this crate can
//! drive the real store instead of a fake. Gated behind `test-support`;
//! ships zero bytes in production.
//!
//! Returns [`SlackHostStateTestParts`] rather than the concrete
//! `FilesystemSlackHostState` — that type stays `pub(crate)` (production
//! wiring at `slack_host_beta.rs` never needs to name it outside this crate
//! either, it upcasts to trait objects the same way). Splitting into the
//! three trait facets here, once, means callers never see the storage
//! implementation type and never hand-roll the same `.clone()` fan-out.

/// The three trait facets of one real `FilesystemSlackHostState` that Slack
/// pairing/actor-resolution tests need — the concrete storage type is not
/// nameable outside this crate, so tests hold only these trait objects, as
/// production wiring does at `slack_host_beta.rs`.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
pub struct SlackHostStateTestParts {
    pub challenges: std::sync::Arc<
        dyn crate::slack_personal_binding_pairing::SlackPersonalBindingPairingChallengeStore,
    >,
    pub bindings: std::sync::Arc<dyn crate::slack_personal_binding::RebornUserIdentityBindingStore>,
    pub lookup: std::sync::Arc<dyn crate::slack_actor_identity::RebornUserIdentityLookup>,
}

/// Construct the real `FilesystemSlackHostState` over an already-mounted
/// `filesystem` and split it into its [`SlackHostStateTestParts`] trait
/// facets.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
pub fn slack_host_state_for_test<F>(
    filesystem: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    user_id: ironclaw_host_api::UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
) -> SlackHostStateTestParts
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    let store = std::sync::Arc::new(crate::slack_host_state::FilesystemSlackHostState::new(
        filesystem, tenant_id, user_id, agent_id, project_id,
    ));
    SlackHostStateTestParts {
        challenges: store.clone(),
        bindings: store.clone(),
        lookup: store,
    }
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
) -> SlackHostStateTestParts
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    let store = std::sync::Arc::new(
        crate::slack_host_state::FilesystemSlackHostState::new(
            filesystem, tenant_id, user_id, agent_id, project_id,
        )
        .with_pairing_ttl(pairing_ttl),
    );
    SlackHostStateTestParts {
        challenges: store.clone(),
        bindings: store.clone(),
        lookup: store,
    }
}
