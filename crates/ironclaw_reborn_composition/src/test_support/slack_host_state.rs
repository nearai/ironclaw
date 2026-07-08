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

/// A manually-advanceable clock for pairing-TTL tests. The production TTL
/// check (`slack_host_state::active_pairing_challenge`, read through
/// `FilesystemSlackHostState::now`) compares against real
/// `chrono::Utc::now()`, which `tokio::time::pause`'s virtual clock never
/// advances — this clock lets a test push `now()` past expiry directly
/// instead of racing a sleep against real time.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
#[derive(Clone)]
pub struct SlackPairingTestClock {
    now: std::sync::Arc<std::sync::Mutex<chrono::DateTime<chrono::Utc>>>,
}

#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
impl Default for SlackPairingTestClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
impl SlackPairingTestClock {
    pub fn new() -> Self {
        Self {
            now: std::sync::Arc::new(std::sync::Mutex::new(chrono::Utc::now())),
        }
    }

    /// Advances the clock by `delta`, deterministically pushing any TTL
    /// comparison read through this clock past expiry.
    pub fn advance(&self, delta: std::time::Duration) {
        let mut guard = Self::lock(&self.now);
        *guard += chrono::Duration::from_std(delta).unwrap_or_else(|_| chrono::Duration::zero());
    }

    fn lock(
        mutex: &std::sync::Mutex<chrono::DateTime<chrono::Utc>>,
    ) -> std::sync::MutexGuard<'_, chrono::DateTime<chrono::Utc>> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn as_now_fn(&self) -> std::sync::Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync> {
        let now = std::sync::Arc::clone(&self.now);
        std::sync::Arc::new(move || *Self::lock(&now))
    }
}

/// Same as [`slack_host_state_for_test_with_pairing_ttl`] but also injects a
/// [`SlackPairingTestClock`] in place of the real wall clock, for
/// deterministic expiry tests.
#[cfg(all(feature = "test-support", feature = "slack-v2-host-beta"))]
pub fn slack_host_state_for_test_with_pairing_ttl_and_clock<F>(
    filesystem: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    user_id: ironclaw_host_api::UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
    pairing_ttl: std::time::Duration,
    clock: &SlackPairingTestClock,
) -> SlackHostStateTestParts
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    let store = std::sync::Arc::new(
        crate::slack_host_state::FilesystemSlackHostState::new(
            filesystem, tenant_id, user_id, agent_id, project_id,
        )
        .with_pairing_ttl(pairing_ttl)
        .with_clock(clock.as_now_fn()),
    );
    SlackHostStateTestParts {
        challenges: store.clone(),
        bindings: store.clone(),
        lookup: store,
    }
}
