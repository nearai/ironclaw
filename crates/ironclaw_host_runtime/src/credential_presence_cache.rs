//! Short-TTL cache for capability-credential presence checks.
//!
//! [`CredentialPresenceCache`] avoids re-querying the secret store / product-auth
//! account resolver on every `visible_capabilities` render. The host rebuilds
//! the capability surface at roughly every LLM step within a turn, but
//! credential presence (is an account connected / is a secret stored) changes
//! far less often than that — so the naive approach would issue N redundant
//! presence lookups per step for no benefit.
//!
//! Only CONCLUSIVE presence results are cached (`Ok(true)`/`Ok(false)` from the
//! secret store, or `AuthRequired`-vs-resolved from the product-auth account
//! resolver). A backend-error/indeterminate outcome is never cached, so a
//! transient blip cannot wedge a stale wrong answer for the whole TTL window —
//! the next lookup simply re-probes live.
//!
//! ## Cache key: owner scope, not full [`ResourceScope`]
//!
//! The key intentionally narrows `ResourceScope` to its owner-identity axes
//! (tenant/user/agent/project) via [`CredentialOwnerScope`], dropping
//! `invocation_id`/`thread_id`/`mission_id`. Credential presence is an
//! account-level property, not an invocation-scoped one. `ResourceScope`
//! carries a fresh `invocation_id` on every request (see
//! `ExecutionContext`/`ResourceScope` in `ironclaw_host_api`), so keying the
//! cache on the full scope would mean every lookup gets a fresh cache key and
//! the cache would never hit — defeating its purpose entirely.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use ironclaw_host_api::{
    AgentId, ExtensionId, ProjectId, ResourceScope, RuntimeCredentialAccountProviderId,
    SecretHandle, TenantId, UserId,
};

/// Default TTL for cached credential-presence answers. Short enough that a
/// user connecting/disconnecting an account is reflected within one heartbeat
/// of surface renders; long enough to collapse the N-per-step lookups a
/// single turn's LLM steps would otherwise repeat against the same backend.
pub(crate) const DEFAULT_CREDENTIAL_PRESENCE_CACHE_TTL: Duration = Duration::from_secs(10);

/// Owner-identity projection of [`ResourceScope`] used as the scope component
/// of a cache key. Deliberately excludes `invocation_id`/`thread_id`/
/// `mission_id` — see the module doc for why.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CredentialOwnerScope {
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: Option<AgentId>,
    project_id: Option<ProjectId>,
}

impl CredentialOwnerScope {
    pub(crate) fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }
}

/// Cache key for one credential presence answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CredentialPresenceKey {
    /// Generic secret-handle credential, keyed by owner scope + handle.
    Secret(CredentialOwnerScope, SecretHandle),
    /// Product-auth account credential, keyed by owner scope + provider +
    /// requesting extension + requested provider scopes (the account
    /// identity axes — see `RuntimeCredentialAuthRequirement`).
    ///
    /// The scopes component is REQUIRED, not cosmetic: presence is
    /// scope-dependent (`account_has_provider_scopes` filters configured
    /// accounts by requested scopes), and a single provider/extension pair
    /// commonly backs multiple capabilities with different required scopes
    /// (e.g. gmail.readonly / gmail.send / gmail.modify all under
    /// `provider="google"`, `requester_extension="gmail"`). Omitting scopes
    /// from the key would let the first-checked capability's answer alias
    /// onto every other capability sharing the same provider/extension in
    /// the same render — reintroducing the #5416 false-"connected" bug (or a
    /// spurious sign-in prompt) depending on check order. Callers must build
    /// this variant with scopes sorted (mirrors `stable_auth_gate_id`'s
    /// scope-sort) so key equality does not depend on manifest declaration
    /// order.
    ProductAuth(
        CredentialOwnerScope,
        RuntimeCredentialAccountProviderId,
        ExtensionId,
        Vec<String>,
    ),
}

struct CacheEntry {
    present: bool,
    inserted_at: Instant,
}

/// Short-TTL, presence-only cache. Never authoritative — a miss or an expired
/// entry simply means the caller re-probes the live backend.
pub(crate) struct CredentialPresenceCache {
    entries: Mutex<HashMap<CredentialPresenceKey, CacheEntry>>,
    ttl: Duration,
}

impl CredentialPresenceCache {
    pub(crate) fn new() -> Self {
        Self::with_ttl(DEFAULT_CREDENTIAL_PRESENCE_CACHE_TTL)
    }

    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Returns the cached presence if a conclusive entry exists and has not
    /// expired; otherwise `None` (cache miss — including "expired", which is
    /// treated identically to "never cached").
    ///
    /// Lock poisoning (a prior holder panicked mid-mutation) is recovered via
    /// `into_inner` rather than propagated: this cache is never an authority —
    /// the worst case of trusting a possibly-torn map is a spurious cache miss
    /// (falls through to a live re-probe), never a wrong presence answer, since
    /// entries are only ever inserted after a conclusive backend result.
    pub(crate) fn get(&self, key: &CredentialPresenceKey) -> Option<bool> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match entries.get(key) {
            Some(entry) if entry.inserted_at.elapsed() < self.ttl => Some(entry.present),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    /// Records a conclusive presence result. Callers must never insert an
    /// indeterminate/backend-error outcome.
    ///
    /// Opportunistically sweeps every expired entry first (not just the key
    /// being inserted). Without this, a key that stops being queried after
    /// expiry (e.g. a capability that scrolls out of a manifest, or a scope
    /// combination checked once and never again) lingers in the map for the
    /// process lifetime — unbounded growth on a long-running multi-user host.
    /// `get` only ever reclaims the single key it was asked about, so it
    /// cannot bound the map on its own.
    pub(crate) fn insert(&self, key: CredentialPresenceKey, present: bool) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ttl = self.ttl;
        entries.retain(|_, entry| entry.inserted_at.elapsed() < ttl);
        entries.insert(
            key,
            CacheEntry {
                present,
                inserted_at: Instant::now(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_scope(tenant: &str, user: &str) -> CredentialOwnerScope {
        CredentialOwnerScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: None,
        }
    }

    fn secret_key(tenant: &str, user: &str, handle: &str) -> CredentialPresenceKey {
        CredentialPresenceKey::Secret(
            owner_scope(tenant, user),
            SecretHandle::new(handle).unwrap(),
        )
    }

    fn product_auth_key(
        tenant: &str,
        user: &str,
        provider: &str,
        extension: &str,
        scopes: &[&str],
    ) -> CredentialPresenceKey {
        let mut scopes: Vec<String> = scopes.iter().map(|scope| scope.to_string()).collect();
        scopes.sort();
        CredentialPresenceKey::ProductAuth(
            owner_scope(tenant, user),
            RuntimeCredentialAccountProviderId::new(provider).unwrap(),
            ExtensionId::new(extension).unwrap(),
            scopes,
        )
    }

    #[test]
    fn hit_within_ttl_returns_cached_presence() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_secs(60));
        let key = secret_key("tenant", "user", "handle");

        cache.insert(key.clone(), true);

        assert_eq!(cache.get(&key), Some(true));
    }

    #[test]
    fn miss_after_expiry_returns_none() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_millis(10));
        let key = secret_key("tenant", "user", "handle");

        cache.insert(key.clone(), false);
        std::thread::sleep(Duration::from_millis(40));

        assert_eq!(cache.get(&key), None);
    }

    #[test]
    fn never_cached_indeterminate_result_is_a_miss() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_secs(60));
        let key = secret_key("tenant", "user", "handle");

        // Simulates a backend-error/indeterminate outcome: the caller never
        // calls `insert` for it. The cache must report a miss, not a stale or
        // fabricated answer.
        assert_eq!(cache.get(&key), None);
    }

    #[test]
    fn distinct_owner_scopes_do_not_collide() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_secs(60));
        let key_a = secret_key("tenant-a", "user", "handle");
        let key_b = secret_key("tenant-b", "user", "handle");

        cache.insert(key_a.clone(), true);

        assert_eq!(cache.get(&key_a), Some(true));
        assert_eq!(cache.get(&key_b), None);
    }

    #[test]
    fn product_auth_key_is_distinct_from_secret_key_with_overlapping_scope() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_secs(60));
        let scope = owner_scope("tenant", "user");
        let secret_key =
            CredentialPresenceKey::Secret(scope.clone(), SecretHandle::new("h").unwrap());
        let product_auth_key = CredentialPresenceKey::ProductAuth(
            scope,
            RuntimeCredentialAccountProviderId::new("google").unwrap(),
            ExtensionId::new("gmail").unwrap(),
            Vec::new(),
        );

        cache.insert(secret_key.clone(), true);

        assert_eq!(cache.get(&secret_key), Some(true));
        assert_eq!(cache.get(&product_auth_key), None);
    }

    /// #5416 Phase 2 Fix B (BLOCKER) regression: two product-auth
    /// requirements that share `(owner_scope, provider, requester_extension)`
    /// but request different provider scopes (e.g. gmail.readonly vs.
    /// gmail.send under the same `provider="google"`,
    /// `requester_extension="gmail"`) must not alias onto the same cache
    /// entry. Before the fix, `CredentialPresenceKey::ProductAuth` omitted
    /// scopes entirely, so caching the readonly capability's answer would be
    /// wrongly reused for the send capability's lookup in the same render.
    #[test]
    fn product_auth_keys_with_different_scopes_do_not_collide() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_secs(60));
        let readonly_key =
            product_auth_key("tenant", "user", "google", "gmail", &["gmail.readonly"]);
        let send_key = product_auth_key("tenant", "user", "google", "gmail", &["gmail.send"]);

        cache.insert(readonly_key.clone(), true);

        assert_eq!(
            cache.get(&readonly_key),
            Some(true),
            "the scope this entry was inserted for must retain its own answer"
        );
        assert_eq!(
            cache.get(&send_key),
            None,
            "a different scope combination under the same provider/extension must not \
             alias onto the readonly answer"
        );
    }

    /// #5416 Phase 2 Fix C (MEDIUM) regression: `insert` must sweep every
    /// expired entry, not just the key being inserted — otherwise a key that
    /// stops being queried after expiry lingers in the map for the process
    /// lifetime. Confirmed by inspecting the underlying map length directly
    /// (a `get`-only check would pass even without the sweep, since `get`
    /// already treats an expired entry as a miss without evicting anything
    /// but the queried key).
    #[test]
    fn insert_sweeps_expired_entries_for_unrelated_keys() {
        let cache = CredentialPresenceCache::with_ttl(Duration::from_millis(10));
        let stale_key = secret_key("tenant", "user", "stale-handle");
        cache.insert(stale_key.clone(), true);
        std::thread::sleep(Duration::from_millis(40));

        let fresh_key = secret_key("tenant", "user", "fresh-handle");
        cache.insert(fresh_key.clone(), false);

        let entries = cache
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            entries.len(),
            1,
            "insert must opportunistically sweep expired entries, not just answer \
             misses lazily on re-query"
        );
        assert!(entries.contains_key(&fresh_key));
        assert!(!entries.contains_key(&stale_key));
    }
}
