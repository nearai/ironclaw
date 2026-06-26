//! Persistent storage + a store-backed [`PolicyResolver`] for capability policy
//! deltas (issue #5273).
//!
//! [`resolve_effective_policy`](crate::resolve_effective_policy) is the pure
//! fold; this module adds the I/O seam around it:
//!
//! - [`CapabilityPolicyDeltaStore`] holds the per-`(tenant, scope, capability)`
//!   deltas an admin sets — the configuration / identity / approval dimensions
//!   layered on top of the capability default. The admin REST surface (#5268)
//!   writes here; the resolver reads.
//! - [`StoreBackedPolicyResolver`] combines a
//!   [`CapabilityDefaultPolicySource`] (the capability default, #5263) with the
//!   subject's stored deltas into an [`EffectivePolicy`].
//!
//! Availability in the resulting [`EffectivePolicy`] is the *policy* view
//! (default + deltas). The dispatch allow-set's *installation* view (#4544 /
//! #5267) is combined with it at the enforcement layer — a capability is
//! available iff it is installed for the subject **and** not hidden by policy.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, TenantId};

use crate::{
    CapabilityDefaultPolicySource, CapabilityPolicyDelta, EffectivePolicy, PolicyError,
    PolicyResolver, PolicyScope, PolicySubject, resolve_effective_policy,
};

/// Durable store of per-`(tenant, scope, capability)` policy deltas. Mirrors the
/// #4544 scoped-lifecycle store shape: an admin upserts/deletes a delta, and the
/// resolver lists the rows relevant to a subject before folding them.
///
/// [`PolicyScope`] carries no tenant, so the tenant is an explicit key here —
/// deltas never leak across tenants.
#[async_trait]
pub trait CapabilityPolicyDeltaStore: Send + Sync {
    /// Upsert the delta at its `(scope, capability)` within `tenant_id`,
    /// replacing any existing delta at that exact key.
    async fn upsert_delta(
        &self,
        tenant_id: &TenantId,
        delta: CapabilityPolicyDelta,
    ) -> Result<(), PolicyError>;

    /// Remove the delta at `(scope, capability)` within `tenant_id`, if present.
    /// Removing an absent delta is a no-op (idempotent revoke).
    async fn delete_delta(
        &self,
        tenant_id: &TenantId,
        scope: &PolicyScope,
        capability: &CapabilityId,
    ) -> Result<(), PolicyError>;

    /// The deltas relevant to `subject` for `capability`: the tenant-wide row
    /// plus the subject's own user row. Pre-filtered for
    /// [`resolve_effective_policy`]. (Project-scope rows are not matched yet —
    /// [`PolicySubject`] carries no project; the default project == the tenant
    /// in v1.)
    async fn deltas_for(
        &self,
        subject: &PolicySubject,
        capability: &CapabilityId,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError>;

    /// Every delta visible to `subject` across capabilities (admin read-back /
    /// the `GET …/capabilities` listing).
    async fn list_subject_deltas(
        &self,
        subject: &PolicySubject,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError>;
}

/// Stable per-row key component for a scope: `tenant` / `project:<id>` /
/// `user:<id>`. Matches the store's `(tenant, scope, capability)` keying so an
/// upsert replaces the delta at the same scope+capability.
fn scope_key(scope: &PolicyScope) -> String {
    match scope {
        PolicyScope::Tenant => "tenant".to_string(),
        PolicyScope::Project { project_id } => format!("project:{}", project_id.as_str()),
        PolicyScope::User { user_id } => format!("user:{}", user_id.as_str()),
    }
}

/// `true` when a delta at `scope` applies to `subject`: the tenant-wide row, or
/// the subject's own user row.
fn scope_applies_to_subject(scope: &PolicyScope, subject: &PolicySubject) -> bool {
    match scope {
        PolicyScope::Tenant => true,
        PolicyScope::User { user_id } => user_id == &subject.user_id,
        // Project scope is dormant in v1 (default project == tenant) and the
        // subject carries no project id to match against.
        PolicyScope::Project { .. } => false,
    }
}

/// Map key for a stored delta: `(tenant, scope_key, capability)`.
type DeltaKey = (String, String, String);
/// Stored value: the delta plus its owning tenant (carried so reads can filter
/// by tenant — `PolicyScope` itself has no tenant).
type StoredDelta = (TenantId, CapabilityPolicyDelta);

/// In-memory [`CapabilityPolicyDeltaStore`] for tests and local-dev. Keyed by
/// `(tenant, scope, capability)` so an upsert replaces the prior delta at that
/// key. Durable filesystem / libSQL backends mirror this contract (follow-on).
#[derive(Default)]
pub struct InMemoryCapabilityPolicyDeltaStore {
    deltas: RwLock<HashMap<DeltaKey, StoredDelta>>,
}

impl InMemoryCapabilityPolicyDeltaStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(tenant_id: &TenantId, scope: &PolicyScope, capability: &CapabilityId) -> DeltaKey {
        (
            tenant_id.as_str().to_string(),
            scope_key(scope),
            capability.as_str().to_string(),
        )
    }
}

#[async_trait]
impl CapabilityPolicyDeltaStore for InMemoryCapabilityPolicyDeltaStore {
    async fn upsert_delta(
        &self,
        tenant_id: &TenantId,
        delta: CapabilityPolicyDelta,
    ) -> Result<(), PolicyError> {
        let key = Self::key(tenant_id, &delta.scope, &delta.capability);
        let mut deltas = self.deltas.write().map_err(|_| PolicyError::Internal {
            reason: "capability policy delta store lock poisoned".to_string(),
        })?;
        deltas.insert(key, (tenant_id.clone(), delta));
        Ok(())
    }

    async fn delete_delta(
        &self,
        tenant_id: &TenantId,
        scope: &PolicyScope,
        capability: &CapabilityId,
    ) -> Result<(), PolicyError> {
        let key = Self::key(tenant_id, scope, capability);
        let mut deltas = self.deltas.write().map_err(|_| PolicyError::Internal {
            reason: "capability policy delta store lock poisoned".to_string(),
        })?;
        deltas.remove(&key);
        Ok(())
    }

    async fn deltas_for(
        &self,
        subject: &PolicySubject,
        capability: &CapabilityId,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError> {
        let deltas = self.deltas.read().map_err(|_| PolicyError::Internal {
            reason: "capability policy delta store lock poisoned".to_string(),
        })?;
        Ok(deltas
            .values()
            .filter(|(tenant_id, delta)| {
                tenant_id == &subject.tenant_id
                    && &delta.capability == capability
                    && scope_applies_to_subject(&delta.scope, subject)
            })
            .map(|(_, delta)| delta.clone())
            .collect())
    }

    async fn list_subject_deltas(
        &self,
        subject: &PolicySubject,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError> {
        let deltas = self.deltas.read().map_err(|_| PolicyError::Internal {
            reason: "capability policy delta store lock poisoned".to_string(),
        })?;
        Ok(deltas
            .values()
            .filter(|(tenant_id, delta)| {
                tenant_id == &subject.tenant_id && scope_applies_to_subject(&delta.scope, subject)
            })
            .map(|(_, delta)| delta.clone())
            .collect())
    }
}

/// A [`PolicyResolver`] that folds a [`CapabilityDefaultPolicySource`] (the
/// capability default, #5263) with the subject's stored
/// [`CapabilityPolicyDelta`] rows into an [`EffectivePolicy`] via
/// [`resolve_effective_policy`]. Resolution is **live** — every call reads the
/// store, never a cached snapshot (architecture doc §8).
pub struct StoreBackedPolicyResolver<D, S> {
    defaults: D,
    deltas: S,
}

impl<D, S> StoreBackedPolicyResolver<D, S> {
    pub fn new(defaults: D, deltas: S) -> Self {
        Self { defaults, deltas }
    }
}

#[async_trait]
impl<D, S> PolicyResolver for StoreBackedPolicyResolver<D, S>
where
    D: CapabilityDefaultPolicySource,
    S: CapabilityPolicyDeltaStore,
{
    async fn resolve(
        &self,
        subject: &PolicySubject,
        capability: &CapabilityId,
    ) -> Result<EffectivePolicy, PolicyError> {
        let default = self.defaults.default_for(capability);
        let deltas = self.deltas.deltas_for(subject, capability).await?;
        Ok(resolve_effective_policy(&default, &deltas))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_host_api::{CapabilityId, PermissionMode, TenantId, UserId};
    use serde_json::json;

    use crate::{
        Availability, CapabilityDefaultPolicy, IdentityMode, StaticCapabilityDefaultPolicySource,
    };

    const TENANT: &str = "tenant:acme";
    const OTHER_TENANT: &str = "tenant:other";

    fn tenant() -> TenantId {
        TenantId::from_trusted(TENANT.to_string())
    }

    fn cap() -> CapabilityId {
        CapabilityId::new("nearai.web_search").expect("cap")
    }

    fn subject(tenant: &str, user: &str) -> PolicySubject {
        PolicySubject {
            tenant_id: TenantId::from_trusted(tenant.to_string()),
            user_id: UserId::from_trusted(user.to_string()),
        }
    }

    fn tenant_delta() -> CapabilityPolicyDelta {
        CapabilityPolicyDelta {
            scope: PolicyScope::Tenant,
            capability: cap(),
            availability: Some(Availability::Available),
            identity: Some(IdentityMode::AdminKeyed),
            approval: Some(PermissionMode::Allow),
            config_patch: Some(json!({ "workspace": "acme" })),
        }
    }

    fn user_delta(user: &str) -> CapabilityPolicyDelta {
        CapabilityPolicyDelta {
            scope: PolicyScope::User {
                user_id: UserId::from_trusted(user.to_string()),
            },
            capability: cap(),
            availability: None,
            identity: None,
            approval: Some(PermissionMode::Deny),
            config_patch: Some(json!({ "verbose": true })),
        }
    }

    #[tokio::test]
    async fn upsert_then_read_back_and_delete() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        store
            .upsert_delta(&tenant(), tenant_delta())
            .await
            .expect("upsert");

        let found = store
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert_eq!(found.len(), 1, "tenant delta is visible to a tenant user");

        store
            .delete_delta(&tenant(), &PolicyScope::Tenant, &cap())
            .await
            .expect("delete");
        let after = store
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert!(after.is_empty(), "deleted delta no longer returned");
    }

    #[tokio::test]
    async fn tenant_delta_visible_to_all_users_user_delta_only_to_owner() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();
        store
            .upsert_delta(&tenant(), user_delta("user:bob"))
            .await
            .unwrap();

        let bob = store
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert_eq!(bob.len(), 2, "Bob sees the tenant row + his own user row");

        let carol = store
            .deltas_for(&subject(TENANT, "user:carol"), &cap())
            .await
            .unwrap();
        assert_eq!(
            carol.len(),
            1,
            "Carol sees only the tenant row, not Bob's user row"
        );
        assert_eq!(carol[0].scope, PolicyScope::Tenant);
    }

    #[tokio::test]
    async fn deltas_do_not_leak_across_tenants() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();

        let other = store
            .deltas_for(&subject(OTHER_TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert!(other.is_empty(), "a different tenant sees no deltas");
    }

    #[tokio::test]
    async fn resolver_folds_default_then_tenant_then_user() {
        // Default: hidden / no credential / ask.
        let defaults = StaticCapabilityDefaultPolicySource::new(
            CapabilityDefaultPolicy::conservative_fallback(),
        );
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();
        store
            .upsert_delta(&tenant(), user_delta("user:bob"))
            .await
            .unwrap();
        let resolver = StoreBackedPolicyResolver::new(defaults, store);

        // Bob: tenant makes it available + admin-keyed + always-allow with
        // `{workspace}` config; his user row overrides approval to Deny and
        // deep-merges `{verbose}`.
        let bob = resolver
            .resolve(&subject(TENANT, "user:bob"), &cap())
            .await
            .expect("resolve");
        assert!(bob.available, "tenant delta made it available");
        assert_eq!(bob.identity, IdentityMode::AdminKeyed);
        assert_eq!(
            bob.approval,
            PermissionMode::Deny,
            "user row wins on approval"
        );
        assert_eq!(bob.config, json!({ "workspace": "acme", "verbose": true }));

        // Carol: only the tenant row applies → always-allow, no user override.
        let carol = resolver
            .resolve(&subject(TENANT, "user:carol"), &cap())
            .await
            .expect("resolve");
        assert_eq!(carol.approval, PermissionMode::Allow);
        assert_eq!(carol.config, json!({ "workspace": "acme" }));
    }

    #[tokio::test]
    async fn resolver_returns_default_when_no_deltas() {
        let defaults = StaticCapabilityDefaultPolicySource::new(
            CapabilityDefaultPolicy::conservative_fallback(),
        );
        let resolver =
            StoreBackedPolicyResolver::new(defaults, InMemoryCapabilityPolicyDeltaStore::new());

        let eff = resolver
            .resolve(&subject(TENANT, "user:bob"), &cap())
            .await
            .expect("resolve");
        assert!(!eff.available, "no deltas → conservative default (hidden)");
        assert_eq!(eff.approval, PermissionMode::Ask);
    }

    #[tokio::test]
    async fn list_subject_deltas_scopes_to_subject() {
        let store = InMemoryCapabilityPolicyDeltaStore::new();
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();
        store
            .upsert_delta(&tenant(), user_delta("user:bob"))
            .await
            .unwrap();

        let bob = store
            .list_subject_deltas(&subject(TENANT, "user:bob"))
            .await
            .unwrap();
        assert_eq!(bob.len(), 2);
        let carol = store
            .list_subject_deltas(&subject(TENANT, "user:carol"))
            .await
            .unwrap();
        assert_eq!(carol.len(), 1);
    }
}
