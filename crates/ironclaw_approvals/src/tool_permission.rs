//! Per-tool permission overrides for the Reborn settings surface (#4776).
//!
//! Reborn already expresses an "always allow this capability" decision as a
//! durable [`PersistentApprovalPolicy`](crate::PersistentApprovalPolicy) grant,
//! which the dispatch approval gate honours through its existing
//! grant-matching path. The two states that grant model *cannot* express are
//! the explicit "keep asking" and "never run" user choices. This module owns
//! those — and only those — as per-(tenant, user, capability) override records.
//!
//! The resolved, three-state value surfaced to the WebUI is
//! [`ToolPermissionState`]; the persisted override is the two-state
//! [`ToolPermissionOverride`]. `always_allow` is intentionally absent from the
//! override store: it lives as a persistent approval grant so there is a single
//! source of truth for auto-run authority.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    CapabilityId, HostApiError, Principal, ResourceScope, ScopedPath, Timestamp,
    sha256_digest_token,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::PersistentApprovalScope;

const OVERRIDE_PREFIX: &str = "/approvals/tool-permissions";
const OVERRIDE_PATH_CACHE_MAX_ENTRIES: usize = 1024;
const OVERRIDE_CAS_RETRY_ATTEMPTS: usize = 3;

/// Resolved per-tool permission as surfaced to the WebUI settings/tools API.
///
/// Wire-stable: serialized as `always_allow` / `ask_each_time` / `disabled`.
/// `AlwaysAllow` is a *resolved* value (backed by a persistent approval grant),
/// not something this module persists directly — see [`ToolPermissionOverride`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionState {
    AlwaysAllow,
    AskEachTime,
    Disabled,
}

/// The explicit per-tool override a user can store. `always_allow` is excluded
/// by construction: it is represented by a persistent approval grant, so the
/// override store only ever holds the two "do not auto-run" decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionOverride {
    AskEachTime,
    Disabled,
}

impl ToolPermissionOverride {
    /// The resolved three-state value this override projects to.
    pub fn as_state(self) -> ToolPermissionState {
        match self {
            Self::AskEachTime => ToolPermissionState::AskEachTime,
            Self::Disabled => ToolPermissionState::Disabled,
        }
    }
}

#[derive(Debug, Error)]
pub enum ToolPermissionStoreError {
    #[error("tool permission override changed concurrently")]
    CasConflict,
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<FilesystemError> for ToolPermissionStoreError {
    fn from(error: FilesystemError) -> Self {
        if matches!(error, FilesystemError::VersionMismatch { .. }) {
            return Self::CasConflict;
        }
        Self::Filesystem(error.to_string())
    }
}

/// Identifies one override record: a capability within a persistent-approval
/// scope (tenant, user, optional agent/project). Reuses
/// [`PersistentApprovalScope`] so the override and always-allow legs share an
/// identical scoping rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolPermissionOverrideKey {
    pub scope: PersistentApprovalScope,
    pub capability_id: CapabilityId,
}

impl ToolPermissionOverrideKey {
    pub fn new(scope: &ResourceScope, capability_id: CapabilityId) -> Self {
        Self {
            scope: PersistentApprovalScope::from_resource_scope(scope),
            capability_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPermissionOverrideRecord {
    pub key: ToolPermissionOverrideKey,
    pub state: ToolPermissionOverride,
    pub updated_by: Principal,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionOverrideInput {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub state: ToolPermissionOverride,
    pub updated_by: Principal,
}

#[async_trait]
pub trait ToolPermissionOverrideStore: Send + Sync {
    /// Create or update the explicit override for a capability.
    async fn set(
        &self,
        input: ToolPermissionOverrideInput,
    ) -> Result<ToolPermissionOverrideRecord, ToolPermissionStoreError>;

    /// Look up the stored override, if any. `None` means the capability has no
    /// explicit override and the caller should fall back to the resolved
    /// default (persistent grant present → always-allow, else seeded default).
    async fn get(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<Option<ToolPermissionOverrideRecord>, ToolPermissionStoreError>;

    /// Remove the explicit override, reverting the capability to its default.
    /// Idempotent: clearing an absent override is a no-op.
    async fn clear(&self, key: &ToolPermissionOverrideKey) -> Result<(), ToolPermissionStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryToolPermissionOverrideStore {
    overrides: RwLock<HashMap<ToolPermissionOverrideKey, ToolPermissionOverrideRecord>>,
}

impl InMemoryToolPermissionOverrideStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ToolPermissionOverrideStore for InMemoryToolPermissionOverrideStore {
    async fn set(
        &self,
        input: ToolPermissionOverrideInput,
    ) -> Result<ToolPermissionOverrideRecord, ToolPermissionStoreError> {
        let key = ToolPermissionOverrideKey::new(&input.scope, input.capability_id);
        let mut overrides = self
            .overrides
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Utc::now();
        let created_at = overrides
            .get(&key)
            .map_or(now, |existing| existing.created_at);
        let record = ToolPermissionOverrideRecord {
            key: key.clone(),
            state: input.state,
            updated_by: input.updated_by,
            created_at,
            updated_at: now,
        };
        overrides.insert(key, record.clone());
        Ok(record)
    }

    async fn get(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<Option<ToolPermissionOverrideRecord>, ToolPermissionStoreError> {
        Ok(self
            .overrides
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned())
    }

    async fn clear(&self, key: &ToolPermissionOverrideKey) -> Result<(), ToolPermissionStoreError> {
        self.overrides
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(key);
        Ok(())
    }
}

pub struct FilesystemToolPermissionOverrideStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    path_cache: RwLock<HashMap<ToolPermissionOverrideKey, ScopedPath>>,
    mutation_locks: Mutex<HashMap<ToolPermissionOverrideKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl<F> FilesystemToolPermissionOverrideStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            path_cache: RwLock::new(HashMap::new()),
            mutation_locks: Mutex::new(HashMap::new()),
        }
    }

    fn record_entry(
        record: &ToolPermissionOverrideRecord,
    ) -> Result<Entry, ToolPermissionStoreError> {
        Ok(Entry::bytes(serialize(record)?).with_content_type(ContentType::json()))
    }
}

#[async_trait]
impl<F> ToolPermissionOverrideStore for FilesystemToolPermissionOverrideStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn set(
        &self,
        input: ToolPermissionOverrideInput,
    ) -> Result<ToolPermissionOverrideRecord, ToolPermissionStoreError> {
        let scope = input.scope.clone();
        let key = ToolPermissionOverrideKey::new(&scope, input.capability_id);
        let path = self.cached_override_path(&key)?;
        let lock = self.mutation_lock(&key);
        let _guard = lock.lock().await;
        for _ in 0..OVERRIDE_CAS_RETRY_ATTEMPTS {
            let existing = self.lookup_versioned(&key).await?;
            let now = Utc::now();
            let (created_at, cas) = existing
                .as_ref()
                .map_or((now, CasExpectation::Absent), |(record, version)| {
                    (record.created_at, CasExpectation::Version(*version))
                });
            let record = ToolPermissionOverrideRecord {
                key: key.clone(),
                state: input.state,
                updated_by: input.updated_by.clone(),
                created_at,
                updated_at: now,
            };
            match self.write_record_raw(&scope, &path, &record, cas).await {
                Ok(()) => return Ok(record),
                Err(ToolPermissionStoreError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(ToolPermissionStoreError::CasConflict)
    }

    async fn get(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<Option<ToolPermissionOverrideRecord>, ToolPermissionStoreError> {
        Ok(self
            .lookup_versioned(key)
            .await?
            .map(|(record, _version)| record))
    }

    async fn clear(&self, key: &ToolPermissionOverrideKey) -> Result<(), ToolPermissionStoreError> {
        let scope = resource_scope_for_override_key(key);
        let path = self.cached_override_path(key)?;
        let lock = self.mutation_lock(key);
        let _guard = lock.lock().await;
        if self.filesystem.get(&scope, &path).await?.is_none() {
            return Ok(());
        }
        self.filesystem.delete(&scope, &path).await?;
        Ok(())
    }
}

impl<F> FilesystemToolPermissionOverrideStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn lookup_versioned(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<Option<(ToolPermissionOverrideRecord, RecordVersion)>, ToolPermissionStoreError>
    {
        let path = self.cached_override_path(key)?;
        let scope = resource_scope_for_override_key(key);
        let Some(versioned) = self.filesystem.get(&scope, &path).await? else {
            return Ok(None);
        };
        deserialize_versioned_record(key, versioned)
    }

    fn mutation_lock(&self, key: &ToolPermissionOverrideKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .mutation_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| Arc::strong_count(lock) > 1);
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn write_record_raw(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        record: &ToolPermissionOverrideRecord,
        expectation: CasExpectation,
    ) -> Result<(), ToolPermissionStoreError> {
        let entry = Self::record_entry(record)?;
        match self
            .filesystem
            .put(scope, path, entry.clone(), expectation)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::Unsupported { .. }) => {
                tracing::warn!(
                    path = %path,
                    "tool permission override store does not support versioned CAS; falling back to unconditional write"
                );
                let opaque = Entry::bytes(entry.body).with_content_type(entry.content_type);
                self.filesystem
                    .put(scope, path, opaque, CasExpectation::Any)
                    .await
                    .map(|_| ())
                    .map_err(ToolPermissionStoreError::from)
            }
            Err(error) => Err(ToolPermissionStoreError::from(error)),
        }
    }

    fn cached_override_path(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<ScopedPath, ToolPermissionStoreError> {
        if let Some(path) = self
            .path_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
        {
            return Ok(path);
        }

        let path = override_path(key)?;
        let mut cache = self
            .path_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(path) = cache.get(key).cloned() {
            return Ok(path);
        }
        if cache.len() >= OVERRIDE_PATH_CACHE_MAX_ENTRIES
            && let Some(evicted) = cache.keys().next().cloned()
        {
            cache.remove(&evicted);
        }
        cache.insert(key.clone(), path.clone());
        Ok(path)
    }
}

fn deserialize_versioned_record(
    key: &ToolPermissionOverrideKey,
    versioned: VersionedEntry,
) -> Result<Option<(ToolPermissionOverrideRecord, RecordVersion)>, ToolPermissionStoreError> {
    let record = deserialize::<ToolPermissionOverrideRecord>(&versioned.entry.body)?;
    if &record.key == key {
        Ok(Some((record, versioned.version)))
    } else {
        tracing::error!(
            stored = ?record.key,
            expected = ?key,
            "tool permission override key mismatch"
        );
        Ok(None)
    }
}

fn override_path(key: &ToolPermissionOverrideKey) -> Result<ScopedPath, ToolPermissionStoreError> {
    ScopedPath::new(format!(
        "{}/{}/{}.json",
        OVERRIDE_PREFIX,
        within_tenant_scope(&key.scope),
        override_digest(key)?
    ))
    .map_err(invalid_path)
}

fn within_tenant_scope(scope: &PersistentApprovalScope) -> String {
    let mut segments = Vec::new();
    if let Some(agent_id) = &scope.agent_id {
        segments.push(format!("agents/{agent_id}"));
    }
    if let Some(project_id) = &scope.project_id {
        segments.push(format!("projects/{project_id}"));
    }
    if segments.is_empty() {
        "scope".to_string()
    } else {
        segments.join("/")
    }
}

fn override_digest(key: &ToolPermissionOverrideKey) -> Result<String, ToolPermissionStoreError> {
    let bytes = serde_json::to_vec(key).map_err(serialization)?;
    let digest = sha256_digest_token(&bytes);
    // Safety: sha256_digest_token always returns "sha256:<hex>".
    Ok(digest
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .to_string())
}

fn resource_scope_for_override_key(key: &ToolPermissionOverrideKey) -> ResourceScope {
    ResourceScope {
        tenant_id: key.scope.tenant_id.clone(),
        user_id: key.scope.user_id.clone(),
        agent_id: key.scope.agent_id.clone(),
        project_id: key.scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

fn serialize<T>(value: &T) -> Result<Vec<u8>, ToolPermissionStoreError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(serialization)
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ToolPermissionStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(serialization)
}

fn serialization(error: serde_json::Error) -> ToolPermissionStoreError {
    ToolPermissionStoreError::Serialization(error.to_string())
}

fn invalid_path(error: HostApiError) -> ToolPermissionStoreError {
    ToolPermissionStoreError::InvalidPath(error.to_string())
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
        ThreadId, UserId, VirtualPath,
    };

    use super::*;

    fn scope(project_id: Option<&str>, thread_id: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("alice").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
            project_id: project_id.map(|id| ProjectId::new(id).unwrap()),
            mission_id: None,
            thread_id: thread_id.map(|id| ThreadId::new(id).unwrap()),
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
    }

    fn input(scope: ResourceScope, state: ToolPermissionOverride) -> ToolPermissionOverrideInput {
        ToolPermissionOverrideInput {
            scope,
            capability_id: CapabilityId::new("builtin.shell").unwrap(),
            state,
            updated_by: Principal::User(UserId::new("alice").unwrap()),
        }
    }

    fn key_for(scope: &ResourceScope) -> ToolPermissionOverrideKey {
        ToolPermissionOverrideKey::new(scope, CapabilityId::new("builtin.shell").unwrap())
    }

    fn scoped_fs<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
    where
        F: RootFilesystem,
    {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/approvals").unwrap(),
            VirtualPath::new(format!("/engine/tenants/{tenant}/users/{user}/approvals")).unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    #[test]
    fn permission_state_wire_values_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&ToolPermissionState::AlwaysAllow).unwrap(),
            "\"always_allow\""
        );
        assert_eq!(
            serde_json::to_string(&ToolPermissionState::AskEachTime).unwrap(),
            "\"ask_each_time\""
        );
        assert_eq!(
            serde_json::to_string(&ToolPermissionState::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[test]
    fn override_projects_to_resolved_state() {
        assert_eq!(
            ToolPermissionOverride::AskEachTime.as_state(),
            ToolPermissionState::AskEachTime
        );
        assert_eq!(
            ToolPermissionOverride::Disabled.as_state(),
            ToolPermissionState::Disabled
        );
    }

    #[tokio::test]
    async fn in_memory_set_get_clear_roundtrip() {
        let store = InMemoryToolPermissionOverrideStore::new();
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        assert!(store.get(&key).await.unwrap().is_none());

        let saved = store
            .set(input(scope.clone(), ToolPermissionOverride::Disabled))
            .await
            .unwrap();
        assert_eq!(saved.state, ToolPermissionOverride::Disabled);
        assert_eq!(
            store.get(&key).await.unwrap().map(|record| record.state),
            Some(ToolPermissionOverride::Disabled)
        );

        // Updating keeps created_at, advances state.
        let updated = store
            .set(input(scope, ToolPermissionOverride::AskEachTime))
            .await
            .unwrap();
        assert_eq!(updated.state, ToolPermissionOverride::AskEachTime);
        assert_eq!(updated.created_at, saved.created_at);

        store.clear(&key).await.unwrap();
        assert!(store.get(&key).await.unwrap().is_none());
        // Clearing again is a no-op.
        store.clear(&key).await.unwrap();
    }

    #[tokio::test]
    async fn filesystem_override_survives_restart() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemToolPermissionOverrideStore::new(Arc::clone(&scoped));
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let saved = store
            .set(input(scope, ToolPermissionOverride::AskEachTime))
            .await
            .unwrap();

        let reloaded = FilesystemToolPermissionOverrideStore::new(scoped)
            .get(&key)
            .await
            .unwrap()
            .expect("override persisted across store instances");
        assert_eq!(reloaded, saved);
        assert_eq!(reloaded.state, ToolPermissionOverride::AskEachTime);
    }

    #[tokio::test]
    async fn filesystem_clear_removes_override() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemToolPermissionOverrideStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        store
            .set(input(scope, ToolPermissionOverride::Disabled))
            .await
            .unwrap();
        assert!(store.get(&key).await.unwrap().is_some());

        store.clear(&key).await.unwrap();
        assert!(store.get(&key).await.unwrap().is_none());
        // Idempotent.
        store.clear(&key).await.unwrap();
    }

    #[tokio::test]
    async fn override_scope_isolates_users() {
        let store = InMemoryToolPermissionOverrideStore::new();
        let alice = scope(None, Some("thread-a"));
        let bob = ResourceScope {
            user_id: UserId::new("bob").unwrap(),
            ..scope(None, Some("thread-a"))
        };

        store
            .set(input(alice.clone(), ToolPermissionOverride::Disabled))
            .await
            .unwrap();

        assert!(store.get(&key_for(&alice)).await.unwrap().is_some());
        assert!(store.get(&key_for(&bob)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn override_scope_is_thread_agnostic() {
        // Mirrors persistent-approval scoping: thread id is not part of the key.
        let store = InMemoryToolPermissionOverrideStore::new();
        let thread_a = scope(None, Some("thread-a"));
        let thread_b = scope(None, Some("thread-b"));

        store
            .set(input(thread_a, ToolPermissionOverride::Disabled))
            .await
            .unwrap();

        assert!(
            store.get(&key_for(&thread_b)).await.unwrap().is_some(),
            "override applies across threads in the same scope"
        );
    }
}
