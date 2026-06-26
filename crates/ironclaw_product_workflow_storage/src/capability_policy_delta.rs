//! Durable filesystem-backed [`CapabilityPolicyDeltaStore`] (issue #5273).
//!
//! Mirrors [`FilesystemScopedLifecycleInstallationStore`] over the universal
//! [`RootFilesystem`] dispatch fabric: an admin upserts/deletes a per-`(tenant,
//! scope, capability)` delta, and the resolver lists the rows relevant to a
//! subject before folding them via `resolve_effective_policy`.
//!
//! [`ironclaw_capability_policy`] is intentionally storage-free, so the trait
//! lives there while this — the only durable implementation — lives in the
//! storage crate that already owns [`RootFilesystem`].
//!
//! Path scheme: a delta is a single leaf grouped by `(tenant, scope)` so a
//! single prefix query enumerates every capability under one scope without a
//! capability argument:
//!
//! ```text
//! <root>/tenants/<hex(tenant)>/deltas/<hex(scope_key)>/<hex(capability)>.json
//! ```
//!
//! [`FilesystemScopedLifecycleInstallationStore`]: crate::FilesystemScopedLifecycleInstallationStore

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_capability_policy::{
    CapabilityPolicyDelta, CapabilityPolicyDeltaStore, PolicyError, PolicyScope, PolicySubject,
};
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexValue, Page, RecordKind,
    RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{CapabilityId, TenantId, VirtualPath};

/// Default durable root. Like the scoped-lifecycle store's own default this is
/// under `/engine/...`, which is **unmounted** in the local-dev composite
/// filesystem; the composition build helper re-roots under the durable
/// `/tenants` mount (see `local_dev_capability_policy_delta_store`).
const DEFAULT_CAPABILITY_POLICY_DELTA_ROOT: &str = "/engine/capability_policy/deltas";
/// The single record kind for a stored delta leaf.
const CAPABILITY_POLICY_DELTA_RECORD_KIND: &str = "capability_policy_delta";

/// Durable [`CapabilityPolicyDeltaStore`] over a [`RootFilesystem`].
///
/// Mirrors [`FilesystemScopedLifecycleInstallationStore`](crate::FilesystemScopedLifecycleInstallationStore):
/// two fields (`filesystem`, `root`), two constructors (`new`, `with_root`).
pub struct FilesystemCapabilityPolicyDeltaStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
}

impl FilesystemCapabilityPolicyDeltaStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self {
            filesystem,
            root: default_capability_policy_delta_root(),
        }
    }

    pub fn with_root(filesystem: Arc<dyn RootFilesystem>, root: VirtualPath) -> Self {
        Self { filesystem, root }
    }

    /// Read every (non-tombstoned) delta under one `(tenant, scope)` prefix.
    /// A never-written scope directory yields an empty `Vec` — some backends
    /// surface a missing prefix as [`FilesystemError::NotFound`] rather than an
    /// empty query, so that case is mapped to empty (an empty store has simply
    /// never written the dir).
    async fn deltas_in_scope(
        &self,
        tenant_id: &TenantId,
        scope: &PolicyScope,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError> {
        let dir = tenant_scope_dir(&self.root, tenant_id, &scope_key(scope))?;
        let mut deltas = Vec::new();
        let mut offset = 0_u64;
        loop {
            let entries = match self
                .filesystem
                .query(&dir, &Filter::All, Page::new(offset, Page::MAX_LIMIT))
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => return Ok(deltas),
                Err(error) => return Err(filesystem_error("list capability policy deltas", error)),
            };
            let entry_count = entries.len();
            for entry in entries {
                deltas.push(parse_delta(&entry)?);
            }
            if entry_count < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.checked_add(Page::MAX_LIMIT as u64).ok_or_else(|| {
                internal("capability policy delta list page overflow".to_string())
            })?;
        }
        Ok(deltas)
    }
}

#[async_trait]
impl CapabilityPolicyDeltaStore for FilesystemCapabilityPolicyDeltaStore {
    async fn upsert_delta(
        &self,
        tenant_id: &TenantId,
        delta: CapabilityPolicyDelta,
    ) -> Result<(), PolicyError> {
        let path = delta_leaf_path(&self.root, tenant_id, &delta.scope, &delta.capability)?;
        // Upsert == replace-at-key, so `CasExpectation::Any` matches the trait's
        // "replacing any existing delta at that exact key" contract; no
        // read-modify-write is needed.
        self.filesystem
            .put(&path, entry_for_delta(&delta)?, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(|error| filesystem_error("upsert capability policy delta", error))
    }

    async fn delete_delta(
        &self,
        tenant_id: &TenantId,
        scope: &PolicyScope,
        capability: &CapabilityId,
    ) -> Result<(), PolicyError> {
        let path = delta_leaf_path(&self.root, tenant_id, scope, capability)?;
        match self.filesystem.delete(&path).await {
            Ok(()) => Ok(()),
            // Idempotent revoke: removing an absent delta is a no-op.
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(filesystem_error("delete capability policy delta", error)),
        }
    }

    async fn deltas_for(
        &self,
        subject: &PolicySubject,
        capability: &CapabilityId,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError> {
        let mut found = Vec::new();
        for scope in subject_scopes(subject) {
            for delta in self.deltas_in_scope(&subject.tenant_id, &scope).await? {
                // Defense-in-depth: the path scheme already isolates by scope,
                // but re-check capability + subject applicability before
                // returning a row to the resolver.
                if &delta.capability == capability
                    && scope_applies_to_subject(&delta.scope, subject)
                {
                    found.push(delta);
                }
            }
        }
        Ok(found)
    }

    async fn list_subject_deltas(
        &self,
        subject: &PolicySubject,
    ) -> Result<Vec<CapabilityPolicyDelta>, PolicyError> {
        let mut found = Vec::new();
        for scope in subject_scopes(subject) {
            for delta in self.deltas_in_scope(&subject.tenant_id, &scope).await? {
                if scope_applies_to_subject(&delta.scope, subject) {
                    found.push(delta);
                }
            }
        }
        Ok(found)
    }
}

fn default_capability_policy_delta_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_CAPABILITY_POLICY_DELTA_ROOT)
        .expect("DEFAULT_CAPABILITY_POLICY_DELTA_ROOT is a valid virtual path")
}

/// The scopes that can carry a delta visible to `subject`: the tenant-wide row
/// and the subject's own user row. (Project scope is dormant in v1 — the
/// subject carries no project id to match.)
fn subject_scopes(subject: &PolicySubject) -> [PolicyScope; 2] {
    [
        PolicyScope::Tenant,
        PolicyScope::User {
            user_id: subject.user_id.clone(),
        },
    ]
}

/// Stable per-row key component for a scope. Re-derived **verbatim** from
/// `ironclaw_capability_policy::store::scope_key` (private there). If that
/// definition changes (e.g. project scope goes live) this copy must change with
/// it — see the regression note in the module tests.
fn scope_key(scope: &PolicyScope) -> String {
    match scope {
        PolicyScope::Tenant => "tenant".to_string(),
        PolicyScope::Project { project_id } => format!("project:{}", project_id.as_str()),
        PolicyScope::User { user_id } => format!("user:{}", user_id.as_str()),
    }
}

/// `true` when a delta at `scope` applies to `subject`. Re-derived **verbatim**
/// from `ironclaw_capability_policy::store::scope_applies_to_subject` (private
/// there).
fn scope_applies_to_subject(scope: &PolicyScope, subject: &PolicySubject) -> bool {
    match scope {
        PolicyScope::Tenant => true,
        PolicyScope::User { user_id } => user_id == &subject.user_id,
        // Project scope is dormant in v1 (default project == tenant) and the
        // subject carries no project id to match against.
        PolicyScope::Project { .. } => false,
    }
}

fn hex_component(value: &str) -> String {
    hex::encode(value)
}

/// The `(tenant, scope)` directory holding one leaf per capability.
fn tenant_scope_dir(
    root: &VirtualPath,
    tenant_id: &TenantId,
    scope_component: &str,
) -> Result<VirtualPath, PolicyError> {
    let path = format!(
        "{}/tenants/{}/deltas/{}",
        root.as_str().trim_end_matches('/'),
        hex_component(tenant_id.as_str()),
        hex_component(scope_component)
    );
    VirtualPath::new(path).map_err(|error| internal(error.to_string()))
}

/// The leaf for one `(tenant, scope, capability)` delta.
fn delta_leaf_path(
    root: &VirtualPath,
    tenant_id: &TenantId,
    scope: &PolicyScope,
    capability: &CapabilityId,
) -> Result<VirtualPath, PolicyError> {
    let dir = tenant_scope_dir(root, tenant_id, &scope_key(scope))?;
    let path = format!(
        "{}/{}.json",
        dir.as_str(),
        hex_component(capability.as_str())
    );
    VirtualPath::new(path).map_err(|error| internal(error.to_string()))
}

fn entry_for_delta(delta: &CapabilityPolicyDelta) -> Result<Entry, PolicyError> {
    let kind = RecordKind::new(CAPABILITY_POLICY_DELTA_RECORD_KIND)
        .map_err(|error| internal(error.to_string()))?;
    let payload = serde_json::to_value(delta).map_err(|error| internal(error.to_string()))?;
    let entry = Entry::record(kind, &payload).map_err(|error| internal(error.to_string()))?;
    // Forward-compat indexed projections: a future indexed query can filter on
    // capability without re-parsing the body. The prefix-listing reads above
    // do not depend on them.
    let entry = entry
        .with_indexed(
            index_key("capability")?,
            IndexValue::Text(delta.capability.as_str().to_string()),
        )
        .with_indexed(
            index_key("scope")?,
            IndexValue::Text(scope_key(&delta.scope)),
        );
    Ok(entry)
}

fn parse_delta(entry: &VersionedEntry) -> Result<CapabilityPolicyDelta, PolicyError> {
    entry
        .entry
        .parse_json::<CapabilityPolicyDelta>()
        .map_err(|error| internal(error.to_string()))
}

fn index_key(value: &'static str) -> Result<IndexKey, PolicyError> {
    IndexKey::new(value).map_err(|error| internal(error.to_string()))
}

/// Map a [`FilesystemError`] to a sanitized [`PolicyError`], logging the real
/// cause first (per `.claude/rules/error-handling.md` — never drop the cause).
/// [`FilesystemError`]'s `Display` is host-path-safe (it carries virtual/scoped
/// paths, never raw host paths), so it is safe to log in full.
fn filesystem_error(operation: &'static str, error: FilesystemError) -> PolicyError {
    tracing::error!(
        operation,
        error = %error,
        "capability policy delta store filesystem operation failed"
    );
    PolicyError::Unavailable {
        reason: format!("capability policy delta store failed to {operation}"),
    }
}

fn internal(reason: String) -> PolicyError {
    PolicyError::Internal { reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_capability_policy::{
        Availability, CapabilityDefaultPolicy, IdentityMode, PolicyResolver,
        StaticCapabilityDefaultPolicySource, StoreBackedPolicyResolver,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{PermissionMode, UserId};
    use serde_json::json;

    const TENANT: &str = "tenant:acme";
    const OTHER_TENANT: &str = "tenant:other";

    fn backend() -> Arc<dyn RootFilesystem> {
        Arc::new(InMemoryBackend::new())
    }

    fn store(filesystem: Arc<dyn RootFilesystem>) -> FilesystemCapabilityPolicyDeltaStore {
        // Tests mount the in-memory backend at root, so the default `/engine`
        // root is reachable.
        FilesystemCapabilityPolicyDeltaStore::new(filesystem)
    }

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
        let store = store(backend());
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
    async fn delete_absent_delta_is_idempotent() {
        let store = store(backend());
        store
            .delete_delta(&tenant(), &PolicyScope::Tenant, &cap())
            .await
            .expect("deleting an absent delta is a no-op");
    }

    #[tokio::test]
    async fn upsert_replaces_at_key() {
        let store = store(backend());
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();
        let mut replacement = tenant_delta();
        replacement.approval = Some(PermissionMode::Deny);
        store
            .upsert_delta(&tenant(), replacement)
            .await
            .expect("re-upsert replaces in place");

        let found = store
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert_eq!(found.len(), 1, "upsert replaces, never duplicates");
        assert_eq!(found[0].approval, Some(PermissionMode::Deny));
    }

    #[tokio::test]
    async fn tenant_delta_visible_to_all_users_user_delta_only_to_owner() {
        let store = store(backend());
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
        let store = store(backend());
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();

        let other = store
            .deltas_for(&subject(OTHER_TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert!(other.is_empty(), "a different tenant sees no deltas");
    }

    #[tokio::test]
    async fn list_subject_deltas_scopes_to_subject() {
        let store = store(backend());
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

    #[tokio::test]
    async fn resolver_folds_default_then_tenant_then_user() {
        let store = store(backend());
        store.upsert_delta(&tenant(), tenant_delta()).await.unwrap();
        store
            .upsert_delta(&tenant(), user_delta("user:bob"))
            .await
            .unwrap();
        let defaults = StaticCapabilityDefaultPolicySource::new(
            CapabilityDefaultPolicy::conservative_fallback(),
        );
        let resolver = StoreBackedPolicyResolver::new(defaults, store);

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

        let carol = resolver
            .resolve(&subject(TENANT, "user:carol"), &cap())
            .await
            .expect("resolve");
        assert_eq!(carol.approval, PermissionMode::Allow);
        assert_eq!(carol.config, json!({ "workspace": "acme" }));
    }

    /// Durability: write through one instance, read through a SECOND instance
    /// over the SAME backend + root. Proves the rows are on the backend, not in
    /// per-instance memory.
    #[tokio::test]
    async fn second_instance_over_same_backend_reads_persisted_deltas() {
        let backend = backend();
        let writer = store(Arc::clone(&backend));
        writer
            .upsert_delta(&tenant(), tenant_delta())
            .await
            .unwrap();
        writer
            .upsert_delta(&tenant(), user_delta("user:bob"))
            .await
            .unwrap();
        drop(writer);

        let reader = store(backend);
        let bob = reader
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert_eq!(
            bob.len(),
            2,
            "a fresh store instance reads deltas a prior instance persisted"
        );
        let carol = reader
            .list_subject_deltas(&subject(TENANT, "user:carol"))
            .await
            .unwrap();
        assert_eq!(carol.len(), 1, "tenant row survives across instances");
    }

    /// An empty store (never-written scope directory) returns `[]`, not an
    /// error — guards the missing-prefix path on every backend.
    #[tokio::test]
    async fn empty_store_returns_no_deltas() {
        let store = store(backend());
        let found = store
            .deltas_for(&subject(TENANT, "user:bob"), &cap())
            .await
            .unwrap();
        assert!(found.is_empty());
        let listed = store
            .list_subject_deltas(&subject(TENANT, "user:bob"))
            .await
            .unwrap();
        assert!(listed.is_empty());
    }
}
