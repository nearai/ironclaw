//! Per-capability permission overrides for the Reborn settings surface.
//!
//! Reborn already expresses an "always allow this capability" decision as a
//! durable [`PersistentApprovalPolicy`](crate::PersistentApprovalPolicy) grant,
//! which the dispatch approval gate honours through its existing
//! grant-matching path. The two states that grant model *cannot* express are
//! the explicit "keep asking" and "never run" user choices. This module owns
//! those — and only those — as per-(tenant, user, capability) override records.
//!
//! The resolved, three-state value surfaced to the WebUI is
//! [`CapabilityPermissionState`]; the persisted override is the two-state
//! [`CapabilityPermissionOverride`]. `always_allow` is intentionally absent from the
//! override store: it lives as a persistent approval grant so there is a single
//! source of truth for auto-run authority.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, FilesystemError, RecordVersion, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{
    CapabilityId, HostApiError, Principal, ResourceScope, ScopedPath, Timestamp,
    sha256_digest_token,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PersistentApprovalScope, cas_record::FilesystemCasRecordStore};

const OVERRIDE_PREFIX: &str = "/approvals/capability-permissions";
const OVERRIDE_PATH_CACHE_MAX_ENTRIES: usize = 1024;
const OVERRIDE_CAS_RETRY_ATTEMPTS: usize = 3;

/// Resolved per-capability permission as surfaced to the WebUI settings/tools API.
///
/// Wire-stable: serialized as `always_allow` / `ask_each_time` / `disabled`.
/// `AlwaysAllow` is a *resolved* value (backed by a persistent approval grant),
/// not something this module persists directly — see [`CapabilityPermissionOverride`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPermissionState {
    AlwaysAllow,
    AskEachTime,
    Disabled,
}

/// The explicit per-capability override a user can store. `always_allow` is excluded
/// by construction: it is represented by a persistent approval grant, so the
/// override store only ever holds the two "do not auto-run" decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPermissionOverride {
    AskEachTime,
    Disabled,
}

impl CapabilityPermissionOverride {
    /// The resolved three-state value this override projects to.
    pub fn as_state(self) -> CapabilityPermissionState {
        match self {
            Self::AskEachTime => CapabilityPermissionState::AskEachTime,
            Self::Disabled => CapabilityPermissionState::Disabled,
        }
    }
}

#[derive(Debug, Error)]
pub enum CapabilityPermissionStoreError {
    #[error("capability permission override changed concurrently")]
    CasConflict,
    #[error("capability permission override integrity error: {0}")]
    Integrity(String),
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<FilesystemError> for CapabilityPermissionStoreError {
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
pub struct CapabilityPermissionOverrideKey {
    pub scope: PersistentApprovalScope,
    pub capability_id: CapabilityId,
}

impl CapabilityPermissionOverrideKey {
    pub fn new(scope: &ResourceScope, capability_id: CapabilityId) -> Self {
        Self {
            scope: PersistentApprovalScope::from_resource_scope(scope),
            capability_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPermissionOverrideRecord {
    pub key: CapabilityPermissionOverrideKey,
    pub state: CapabilityPermissionOverride,
    pub updated_by: Principal,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPermissionOverrideInput {
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub state: CapabilityPermissionOverride,
    pub updated_by: Principal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredCapabilityPermissionOverrideRecord {
    key: CapabilityPermissionOverrideKey,
    state: Option<CapabilityPermissionOverride>,
    updated_by: Principal,
    created_at: Timestamp,
    updated_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cleared_at: Option<Timestamp>,
}

impl StoredCapabilityPermissionOverrideRecord {
    fn active(record: &CapabilityPermissionOverrideRecord) -> Self {
        Self {
            key: record.key.clone(),
            state: Some(record.state),
            updated_by: record.updated_by.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
            cleared_at: None,
        }
    }

    fn tombstone_from(existing: &Self, now: Timestamp) -> Self {
        Self {
            key: existing.key.clone(),
            state: None,
            updated_by: existing.updated_by.clone(),
            created_at: existing.created_at,
            updated_at: now,
            cleared_at: Some(now),
        }
    }

    fn into_active(self) -> Option<CapabilityPermissionOverrideRecord> {
        self.state.map(|state| CapabilityPermissionOverrideRecord {
            key: self.key,
            state,
            updated_by: self.updated_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[async_trait]
pub trait CapabilityPermissionOverrideStore: Send + Sync {
    /// Create or update the explicit override for a capability.
    async fn set(
        &self,
        input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError>;

    /// Look up the stored override, if any. `None` means the capability has no
    /// explicit override and the caller should fall back to the resolved
    /// default (persistent grant present → always-allow, else seeded default).
    async fn get(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError>;

    /// Remove the explicit override, reverting the capability to its default.
    /// Idempotent: clearing an absent override is a no-op.
    async fn clear(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError>;
}

#[derive(Debug, Default)]
pub struct InMemoryCapabilityPermissionOverrideStore {
    overrides: RwLock<HashMap<CapabilityPermissionOverrideKey, CapabilityPermissionOverrideRecord>>,
}

impl InMemoryCapabilityPermissionOverrideStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CapabilityPermissionOverrideStore for InMemoryCapabilityPermissionOverrideStore {
    async fn set(
        &self,
        input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError> {
        let key = CapabilityPermissionOverrideKey::new(&input.scope, input.capability_id);
        let mut overrides = self
            .overrides
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Utc::now();
        let created_at = overrides
            .get(&key)
            .map_or(now, |existing| existing.created_at);
        let record = CapabilityPermissionOverrideRecord {
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
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        Ok(self
            .overrides
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned())
    }

    async fn clear(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError> {
        self.overrides
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(key);
        Ok(())
    }
}

pub struct FilesystemCapabilityPermissionOverrideStore<F>
where
    F: RootFilesystem,
{
    records: FilesystemCasRecordStore<F, CapabilityPermissionOverrideKey>,
}

impl<F> FilesystemCapabilityPermissionOverrideStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            records: FilesystemCasRecordStore::new(filesystem, OVERRIDE_PATH_CACHE_MAX_ENTRIES),
        }
    }
}

#[async_trait]
impl<F> CapabilityPermissionOverrideStore for FilesystemCapabilityPermissionOverrideStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn set(
        &self,
        input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError> {
        let scope = input.scope.clone();
        let key = CapabilityPermissionOverrideKey::new(&scope, input.capability_id);
        let path = self.cached_override_path(&key)?;
        let lock = self.records.mutation_lock(&key);
        let _guard = lock.lock().await;
        for _ in 0..OVERRIDE_CAS_RETRY_ATTEMPTS {
            let existing = self.lookup_versioned(&key).await?;
            let now = Utc::now();
            let (created_at, cas) =
                existing
                    .as_ref()
                    .map_or((now, CasExpectation::Absent), |(record, version)| {
                        let created_at = if record.state.is_some() {
                            record.created_at
                        } else {
                            now
                        };
                        (created_at, CasExpectation::Version(*version))
                    });
            let record = CapabilityPermissionOverrideRecord {
                key: key.clone(),
                state: input.state,
                updated_by: input.updated_by.clone(),
                created_at,
                updated_at: now,
            };
            match self.write_record_raw(&scope, &path, &record, cas).await {
                Ok(()) => return Ok(record),
                Err(CapabilityPermissionStoreError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(CapabilityPermissionStoreError::CasConflict)
    }

    async fn get(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        Ok(self
            .lookup_versioned(key)
            .await?
            .and_then(|(record, _version)| record.into_active()))
    }

    async fn clear(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError> {
        let scope = resource_scope_for_override_key(key);
        let path = self.cached_override_path(key)?;
        let lock = self.records.mutation_lock(key);
        let _guard = lock.lock().await;
        let Some((existing, version)) = self.lookup_versioned(key).await? else {
            return Ok(());
        };
        if existing.state.is_none() {
            return Ok(());
        }
        let tombstone =
            StoredCapabilityPermissionOverrideRecord::tombstone_from(&existing, Utc::now());
        self.write_stored_record_raw(&scope, &path, &tombstone, CasExpectation::Version(version))
            .await
    }
}

impl<F> FilesystemCapabilityPermissionOverrideStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn lookup_versioned(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<
        Option<(StoredCapabilityPermissionOverrideRecord, RecordVersion)>,
        CapabilityPermissionStoreError,
    > {
        let path = self.cached_override_path(key)?;
        let scope = resource_scope_for_override_key(key);
        let Some(versioned) = self.records.get(&scope, &path).await? else {
            return Ok(None);
        };
        deserialize_versioned_record(key, versioned)
    }

    async fn write_record_raw(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        record: &CapabilityPermissionOverrideRecord,
        expectation: CasExpectation,
    ) -> Result<(), CapabilityPermissionStoreError> {
        let stored = StoredCapabilityPermissionOverrideRecord::active(record);
        self.write_stored_record_raw(scope, path, &stored, expectation)
            .await
    }

    async fn write_stored_record_raw(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        record: &StoredCapabilityPermissionOverrideRecord,
        expectation: CasExpectation,
    ) -> Result<(), CapabilityPermissionStoreError> {
        self.records
            .put_json(scope, path, serialize(record)?, expectation)
            .await
    }

    fn cached_override_path(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<ScopedPath, CapabilityPermissionStoreError> {
        self.records.cached_path(key, override_path)
    }
}

fn deserialize_versioned_record(
    key: &CapabilityPermissionOverrideKey,
    versioned: VersionedEntry,
) -> Result<
    Option<(StoredCapabilityPermissionOverrideRecord, RecordVersion)>,
    CapabilityPermissionStoreError,
> {
    let record = deserialize::<StoredCapabilityPermissionOverrideRecord>(&versioned.entry.body)?;
    if &record.key != key {
        Err(CapabilityPermissionStoreError::Integrity(format!(
            "stored key {:?} does not match expected {:?}",
            record.key, key
        )))
    } else if record.state.is_some() == record.cleared_at.is_some() {
        Err(CapabilityPermissionStoreError::Integrity(
            "stored override must be active with no cleared_at or tombstoned with cleared_at"
                .to_string(),
        ))
    } else {
        Ok(Some((record, versioned.version)))
    }
}

fn override_path(
    key: &CapabilityPermissionOverrideKey,
) -> Result<ScopedPath, CapabilityPermissionStoreError> {
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

fn override_digest(
    key: &CapabilityPermissionOverrideKey,
) -> Result<String, CapabilityPermissionStoreError> {
    let bytes = serde_json::to_vec(key).map_err(serialization)?;
    let digest = sha256_digest_token(&bytes);
    // Safety: sha256_digest_token always returns "sha256:<hex>".
    Ok(digest
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .to_string())
}

fn resource_scope_for_override_key(key: &CapabilityPermissionOverrideKey) -> ResourceScope {
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

fn serialize<T>(value: &T) -> Result<Vec<u8>, CapabilityPermissionStoreError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(serialization)
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, CapabilityPermissionStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(serialization)
}

fn serialization(error: serde_json::Error) -> CapabilityPermissionStoreError {
    CapabilityPermissionStoreError::Serialization(error.to_string())
}

fn invalid_path(error: HostApiError) -> CapabilityPermissionStoreError {
    CapabilityPermissionStoreError::InvalidPath(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use ironclaw_filesystem::{
        BackendCapabilities, ContentType, DirEntry, Entry, FileStat, Filter, InMemoryBackend,
        IndexSpec, Page, ScopedFilesystem,
    };
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

    fn input(
        scope: ResourceScope,
        state: CapabilityPermissionOverride,
    ) -> CapabilityPermissionOverrideInput {
        CapabilityPermissionOverrideInput {
            scope,
            capability_id: CapabilityId::new("builtin.shell").unwrap(),
            state,
            updated_by: Principal::User(UserId::new("alice").unwrap()),
        }
    }

    fn key_for(scope: &ResourceScope) -> CapabilityPermissionOverrideKey {
        CapabilityPermissionOverrideKey::new(scope, CapabilityId::new("builtin.shell").unwrap())
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

    struct VersionMismatchOnceBackend {
        inner: Arc<InMemoryBackend>,
        injected: AtomicBool,
    }

    impl VersionMismatchOnceBackend {
        fn new(inner: Arc<InMemoryBackend>) -> Self {
            Self {
                inner,
                injected: AtomicBool::new(false),
            }
        }

        fn injected(&self) -> bool {
            self.injected.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RootFilesystem for VersionMismatchOnceBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            if matches!(cas, CasExpectation::Version(_))
                && !self.injected.swap(true, Ordering::SeqCst)
            {
                return Err(FilesystemError::VersionMismatch {
                    path: path.clone(),
                    expected: None,
                    found: None,
                });
            }
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }
    }

    struct ClearRacingBackend {
        inner: Arc<InMemoryBackend>,
        replacement: std::sync::Mutex<Option<Entry>>,
        injected: AtomicBool,
    }

    impl ClearRacingBackend {
        fn new(inner: Arc<InMemoryBackend>) -> Self {
            Self {
                inner,
                replacement: std::sync::Mutex::new(None),
                injected: AtomicBool::new(false),
            }
        }

        fn arm(&self, replacement: Entry) {
            *self
                .replacement
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(replacement);
        }

        fn injected(&self) -> bool {
            self.injected.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RootFilesystem for ClearRacingBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            let replacement = if matches!(cas, CasExpectation::Version(_))
                && !self.injected.swap(true, Ordering::SeqCst)
            {
                self.replacement
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
            } else {
                None
            };
            if let Some(replacement) = replacement {
                let _ = self
                    .inner
                    .put(path, replacement, CasExpectation::Any)
                    .await?;
            }
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }
    }

    #[test]
    fn permission_state_wire_values_are_snake_case() {
        assert_eq!(
            serde_json::to_string(&CapabilityPermissionState::AlwaysAllow).unwrap(),
            "\"always_allow\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityPermissionState::AskEachTime).unwrap(),
            "\"ask_each_time\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityPermissionState::Disabled).unwrap(),
            "\"disabled\""
        );

        assert_eq!(
            serde_json::from_str::<CapabilityPermissionState>("\"always_allow\"").unwrap(),
            CapabilityPermissionState::AlwaysAllow
        );
        assert_eq!(
            serde_json::from_str::<CapabilityPermissionState>("\"ask_each_time\"").unwrap(),
            CapabilityPermissionState::AskEachTime
        );
        assert_eq!(
            serde_json::from_str::<CapabilityPermissionState>("\"disabled\"").unwrap(),
            CapabilityPermissionState::Disabled
        );
    }

    #[test]
    fn override_wire_values_exclude_always_allow() {
        assert_eq!(
            serde_json::to_string(&CapabilityPermissionOverride::AskEachTime).unwrap(),
            "\"ask_each_time\""
        );
        assert_eq!(
            serde_json::to_string(&CapabilityPermissionOverride::Disabled).unwrap(),
            "\"disabled\""
        );
        assert_eq!(
            serde_json::from_str::<CapabilityPermissionOverride>("\"ask_each_time\"").unwrap(),
            CapabilityPermissionOverride::AskEachTime
        );
        assert_eq!(
            serde_json::from_str::<CapabilityPermissionOverride>("\"disabled\"").unwrap(),
            CapabilityPermissionOverride::Disabled
        );
        assert!(serde_json::from_str::<CapabilityPermissionOverride>("\"always_allow\"").is_err());
    }

    #[test]
    fn override_projects_to_resolved_state() {
        assert_eq!(
            CapabilityPermissionOverride::AskEachTime.as_state(),
            CapabilityPermissionState::AskEachTime
        );
        assert_eq!(
            CapabilityPermissionOverride::Disabled.as_state(),
            CapabilityPermissionState::Disabled
        );
    }

    #[tokio::test]
    async fn in_memory_set_get_clear_roundtrip() {
        let store = InMemoryCapabilityPermissionOverrideStore::new();
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        assert!(store.get(&key).await.unwrap().is_none());

        let saved = store
            .set(input(scope.clone(), CapabilityPermissionOverride::Disabled))
            .await
            .unwrap();
        assert_eq!(saved.state, CapabilityPermissionOverride::Disabled);
        assert_eq!(
            store.get(&key).await.unwrap().map(|record| record.state),
            Some(CapabilityPermissionOverride::Disabled)
        );

        // Updating keeps created_at, advances state.
        let updated = store
            .set(input(scope, CapabilityPermissionOverride::AskEachTime))
            .await
            .unwrap();
        assert_eq!(updated.state, CapabilityPermissionOverride::AskEachTime);
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
        let store = FilesystemCapabilityPermissionOverrideStore::new(Arc::clone(&scoped));
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let saved = store
            .set(input(scope, CapabilityPermissionOverride::AskEachTime))
            .await
            .unwrap();

        let reloaded = FilesystemCapabilityPermissionOverrideStore::new(scoped)
            .get(&key)
            .await
            .unwrap()
            .expect("override persisted across store instances");
        assert_eq!(reloaded, saved);
        assert_eq!(reloaded.state, CapabilityPermissionOverride::AskEachTime);
    }

    #[tokio::test]
    async fn filesystem_clear_removes_override() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemCapabilityPermissionOverrideStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        store
            .set(input(scope.clone(), CapabilityPermissionOverride::Disabled))
            .await
            .unwrap();
        assert!(store.get(&key).await.unwrap().is_some());

        store.clear(&key).await.unwrap();
        assert!(store.get(&key).await.unwrap().is_none());
        // Idempotent.
        store.clear(&key).await.unwrap();

        let path = override_path(&key).unwrap();
        let stored = store
            .records
            .filesystem
            .get(&resource_scope_for_override_key(&key), &path)
            .await
            .unwrap()
            .expect("clear leaves a versioned tombstone");
        let tombstone =
            deserialize::<StoredCapabilityPermissionOverrideRecord>(&stored.entry.body).unwrap();
        assert!(tombstone.state.is_none());
        assert!(tombstone.cleared_at.is_some());

        let reset = store
            .set(input(scope, CapabilityPermissionOverride::AskEachTime))
            .await
            .unwrap();
        assert_eq!(reset.state, CapabilityPermissionOverride::AskEachTime);
        assert!(
            reset.created_at >= tombstone.updated_at,
            "set after clear should behave like a fresh explicit override"
        );
    }

    #[tokio::test]
    async fn filesystem_clear_preserves_newer_override_on_version_mismatch() {
        let inner = Arc::new(InMemoryBackend::new());
        let backend = Arc::new(ClearRacingBackend::new(inner));
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemCapabilityPermissionOverrideStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        store
            .set(input(
                scope.clone(),
                CapabilityPermissionOverride::AskEachTime,
            ))
            .await
            .unwrap();
        let newer = CapabilityPermissionOverrideRecord {
            key: key.clone(),
            state: CapabilityPermissionOverride::Disabled,
            updated_by: Principal::User(UserId::new("alice").unwrap()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let newer_entry = Entry::bytes(
            serialize(&StoredCapabilityPermissionOverrideRecord::active(&newer)).unwrap(),
        )
        .with_content_type(ContentType::json());
        backend.arm(newer_entry);

        let error = store.clear(&key).await.unwrap_err();

        assert!(backend.injected());
        assert!(matches!(error, CapabilityPermissionStoreError::CasConflict));
        assert_eq!(
            store.get(&key).await.unwrap().map(|record| record.state),
            Some(CapabilityPermissionOverride::Disabled)
        );
    }

    #[tokio::test]
    async fn filesystem_set_retries_after_concurrent_version_mismatch() {
        let inner = Arc::new(InMemoryBackend::new());
        let backend = Arc::new(VersionMismatchOnceBackend::new(inner));
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemCapabilityPermissionOverrideStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        store
            .set(input(
                scope.clone(),
                CapabilityPermissionOverride::AskEachTime,
            ))
            .await
            .unwrap();
        let saved = store
            .set(input(scope, CapabilityPermissionOverride::Disabled))
            .await
            .unwrap();

        assert!(backend.injected());
        assert_eq!(saved.state, CapabilityPermissionOverride::Disabled);
        assert_eq!(
            store.get(&key).await.unwrap().map(|record| record.state),
            Some(CapabilityPermissionOverride::Disabled)
        );
    }

    #[tokio::test]
    async fn filesystem_project_scoped_override_matches_in_new_thread_after_reload() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemCapabilityPermissionOverrideStore::new(Arc::clone(&scoped));

        let saved = store
            .set(input(
                scope(Some("project-a"), Some("thread-1")),
                CapabilityPermissionOverride::Disabled,
            ))
            .await
            .unwrap();

        let new_thread_key = key_for(&scope(Some("project-a"), Some("thread-2")));
        let reloaded = FilesystemCapabilityPermissionOverrideStore::new(scoped)
            .get(&new_thread_key)
            .await
            .unwrap()
            .expect("project-scoped override still matches in a new thread");

        assert_eq!(reloaded, saved);
    }

    #[tokio::test]
    async fn filesystem_get_returns_serialization_error_for_corrupt_override_record() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemCapabilityPermissionOverrideStore::new(Arc::clone(&scoped));
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);
        let path = override_path(&key).unwrap();

        scoped
            .put(
                &resource_scope_for_override_key(&key),
                &path,
                Entry::bytes(b"{not valid json".to_vec()).with_content_type(ContentType::json()),
                CasExpectation::Absent,
            )
            .await
            .unwrap();

        let error = store.get(&key).await.unwrap_err();
        assert!(matches!(
            error,
            CapabilityPermissionStoreError::Serialization(_)
        ));
    }

    #[test]
    fn deserialize_versioned_record_rejects_key_mismatch() {
        let expected_key = key_for(&scope(None, Some("thread-a")));
        let stored_key = CapabilityPermissionOverrideKey::new(
            &scope(None, Some("thread-a")),
            CapabilityId::new("builtin.http").unwrap(),
        );
        let stored = StoredCapabilityPermissionOverrideRecord {
            key: stored_key,
            state: Some(CapabilityPermissionOverride::Disabled),
            updated_by: Principal::User(UserId::new("alice").unwrap()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            cleared_at: None,
        };
        let versioned = VersionedEntry {
            path: VirtualPath::new("/engine/record.json").unwrap(),
            entry: Entry::bytes(serialize(&stored).unwrap()).with_content_type(ContentType::json()),
            version: RecordVersion::from_backend(1),
        };

        let error = deserialize_versioned_record(&expected_key, versioned).unwrap_err();

        assert!(matches!(
            error,
            CapabilityPermissionStoreError::Integrity(_)
        ));
    }

    #[test]
    fn deserialize_versioned_record_rejects_malformed_tombstone_shapes() {
        let key = key_for(&scope(None, Some("thread-a")));
        let base = StoredCapabilityPermissionOverrideRecord {
            key: key.clone(),
            state: Some(CapabilityPermissionOverride::Disabled),
            updated_by: Principal::User(UserId::new("alice").unwrap()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            cleared_at: None,
        };

        for malformed in [
            StoredCapabilityPermissionOverrideRecord {
                state: None,
                cleared_at: None,
                ..base.clone()
            },
            StoredCapabilityPermissionOverrideRecord {
                cleared_at: Some(Utc::now()),
                ..base
            },
        ] {
            let versioned = VersionedEntry {
                path: VirtualPath::new("/engine/record.json").unwrap(),
                entry: Entry::bytes(serialize(&malformed).unwrap())
                    .with_content_type(ContentType::json()),
                version: RecordVersion::from_backend(1),
            };

            let error = deserialize_versioned_record(&key, versioned).unwrap_err();

            assert!(matches!(
                error,
                CapabilityPermissionStoreError::Integrity(_)
            ));
        }
    }

    #[tokio::test]
    async fn override_scope_isolates_users() {
        let store = InMemoryCapabilityPermissionOverrideStore::new();
        let alice = scope(None, Some("thread-a"));
        let bob = ResourceScope {
            user_id: UserId::new("bob").unwrap(),
            ..scope(None, Some("thread-a"))
        };

        store
            .set(input(alice.clone(), CapabilityPermissionOverride::Disabled))
            .await
            .unwrap();

        assert!(store.get(&key_for(&alice)).await.unwrap().is_some());
        assert!(store.get(&key_for(&bob)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn override_scope_is_thread_agnostic() {
        // Mirrors persistent-approval scoping: thread id is not part of the key.
        let store = InMemoryCapabilityPermissionOverrideStore::new();
        let thread_a = scope(None, Some("thread-a"));
        let thread_b = scope(None, Some("thread-b"));

        store
            .set(input(thread_a, CapabilityPermissionOverride::Disabled))
            .await
            .unwrap();

        assert!(
            store.get(&key_for(&thread_b)).await.unwrap().is_some(),
            "override applies across threads in the same scope"
        );
    }
}
