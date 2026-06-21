//! Global "auto-approve eligible tools" setting for the Reborn settings surface
//! (#4776 / #4959).
//!
//! This is a per-(tenant, user) boolean toggle. When enabled, the dispatch
//! approval gate auto-approves a tool whose effective permission is the
//! default/seeded ask behaviour — while the hard floor (never-auto-approve
//! effects), explicit per-tool overrides, missing-grant denials, and trust
//! ceilings are all still enforced above it (see the authorizer in
//! `ironclaw_reborn_composition`).
//!
//! Scope is deliberately `(tenant, user)` only — agent/project/thread are
//! dropped — so a single user-level toggle applies across every agent and
//! project, and a value written by the settings UI resolves identically at
//! dispatch time regardless of the runtime's agent/project context.

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
    HostApiError, Principal, ResourceScope, ScopedPath, TenantId, Timestamp, UserId,
    sha256_digest_token,
};
use serde::{Deserialize, Serialize};

use crate::CapabilityPermissionStoreError as ToolPermissionStoreError;

const SETTING_PREFIX: &str = "/approvals/auto-approve";
const SETTING_CAS_RETRY_ATTEMPTS: usize = 3;

/// `(tenant, user)` identity for the global auto-approve toggle. Agent, project,
/// thread, and mission are intentionally excluded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AutoApproveSettingKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl AutoApproveSettingKey {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoApproveSettingRecord {
    pub key: AutoApproveSettingKey,
    pub enabled: bool,
    pub updated_by: Principal,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoApproveSettingInput {
    pub scope: ResourceScope,
    pub enabled: bool,
    pub updated_by: Principal,
}

#[async_trait]
pub trait AutoApproveSettingStore: Send + Sync {
    /// Create or update the per-(tenant, user) auto-approve toggle.
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, ToolPermissionStoreError>;

    /// Look up the stored toggle. `None` means the user has never set it.
    async fn get(
        &self,
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, ToolPermissionStoreError>;

    /// Convenience: the effective boolean, defaulting to `false` when unset.
    async fn is_enabled(&self, scope: &ResourceScope) -> Result<bool, ToolPermissionStoreError> {
        let key = AutoApproveSettingKey::from_resource_scope(scope);
        Ok(self.get(&key).await?.is_some_and(|record| record.enabled))
    }
}

#[derive(Debug, Default)]
pub struct InMemoryAutoApproveSettingStore {
    settings: RwLock<HashMap<AutoApproveSettingKey, AutoApproveSettingRecord>>,
}

impl InMemoryAutoApproveSettingStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AutoApproveSettingStore for InMemoryAutoApproveSettingStore {
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, ToolPermissionStoreError> {
        let key = AutoApproveSettingKey::from_resource_scope(&input.scope);
        let mut settings = self
            .settings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Utc::now();
        let created_at = settings
            .get(&key)
            .map_or(now, |existing| existing.created_at);
        let record = AutoApproveSettingRecord {
            key: key.clone(),
            enabled: input.enabled,
            updated_by: input.updated_by,
            created_at,
            updated_at: now,
        };
        settings.insert(key, record.clone());
        Ok(record)
    }

    async fn get(
        &self,
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, ToolPermissionStoreError> {
        Ok(self
            .settings
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned())
    }
}

pub struct FilesystemAutoApproveSettingStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    mutation_locks: Mutex<HashMap<AutoApproveSettingKey, Arc<tokio::sync::Mutex<()>>>>,
}

impl<F> FilesystemAutoApproveSettingStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            mutation_locks: Mutex::new(HashMap::new()),
        }
    }

    fn record_entry(record: &AutoApproveSettingRecord) -> Result<Entry, ToolPermissionStoreError> {
        Ok(Entry::bytes(serialize(record)?).with_content_type(ContentType::json()))
    }
}

#[async_trait]
impl<F> AutoApproveSettingStore for FilesystemAutoApproveSettingStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, ToolPermissionStoreError> {
        let scope = input.scope.clone();
        let key = AutoApproveSettingKey::from_resource_scope(&scope);
        let path = setting_path(&key)?;
        let lock = self.mutation_lock(&key);
        let _guard = lock.lock().await;
        for _ in 0..SETTING_CAS_RETRY_ATTEMPTS {
            let existing = self.lookup_versioned(&key, &scope, &path).await?;
            let now = Utc::now();
            let (created_at, cas) = existing
                .as_ref()
                .map_or((now, CasExpectation::Absent), |(record, version)| {
                    (record.created_at, CasExpectation::Version(*version))
                });
            let record = AutoApproveSettingRecord {
                key: key.clone(),
                enabled: input.enabled,
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
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, ToolPermissionStoreError> {
        let scope = resource_scope_for_key(key);
        let path = setting_path(key)?;
        Ok(self
            .lookup_versioned(key, &scope, &path)
            .await?
            .map(|(record, _version)| record))
    }
}

impl<F> FilesystemAutoApproveSettingStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn lookup_versioned(
        &self,
        key: &AutoApproveSettingKey,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<(AutoApproveSettingRecord, RecordVersion)>, ToolPermissionStoreError> {
        let Some(versioned) = self.filesystem.get(scope, path).await? else {
            return Ok(None);
        };
        deserialize_versioned_record(key, versioned)
    }

    fn mutation_lock(&self, key: &AutoApproveSettingKey) -> Arc<tokio::sync::Mutex<()>> {
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
        record: &AutoApproveSettingRecord,
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
                    "auto-approve setting store does not support versioned CAS; falling back to unconditional write"
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
}

fn deserialize_versioned_record(
    key: &AutoApproveSettingKey,
    versioned: VersionedEntry,
) -> Result<Option<(AutoApproveSettingRecord, RecordVersion)>, ToolPermissionStoreError> {
    let record = deserialize::<AutoApproveSettingRecord>(&versioned.entry.body)?;
    if &record.key == key {
        Ok(Some((record, versioned.version)))
    } else {
        Err(ToolPermissionStoreError::Integrity(format!(
            "stored key {:?} does not match expected {:?}",
            record.key, key
        )))
    }
}

fn setting_path(key: &AutoApproveSettingKey) -> Result<ScopedPath, ToolPermissionStoreError> {
    ScopedPath::new(format!("{}/{}.json", SETTING_PREFIX, setting_digest(key)?))
        .map_err(invalid_path)
}

fn setting_digest(key: &AutoApproveSettingKey) -> Result<String, ToolPermissionStoreError> {
    let bytes = serde_json::to_vec(key).map_err(serialization)?;
    let digest = sha256_digest_token(&bytes);
    Ok(digest
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .to_string())
}

fn resource_scope_for_key(key: &AutoApproveSettingKey) -> ResourceScope {
    ResourceScope {
        tenant_id: key.tenant_id.clone(),
        user_id: key.user_id.clone(),
        agent_id: None,
        project_id: None,
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
    use ironclaw_filesystem::{
        ContentType, Entry, InMemoryBackend, RecordVersion, ScopedFilesystem,
    };
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, VirtualPath,
    };

    use super::*;

    fn scope(user: &str, agent: Option<&str>, project: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: agent.map(|id| AgentId::new(id).unwrap()),
            project_id: project.map(|id| ProjectId::new(id).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
    }

    fn input(scope: ResourceScope, enabled: bool) -> AutoApproveSettingInput {
        AutoApproveSettingInput {
            scope,
            enabled,
            updated_by: Principal::User(UserId::new("alice").unwrap()),
        }
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

    #[tokio::test]
    async fn in_memory_defaults_to_disabled() {
        let store = InMemoryAutoApproveSettingStore::new();
        assert!(!store.is_enabled(&scope("alice", None, None)).await.unwrap());
    }

    #[tokio::test]
    async fn in_memory_set_get_roundtrip_and_update() {
        let store = InMemoryAutoApproveSettingStore::new();
        let scope = scope("alice", None, None);

        let saved = store.set(input(scope.clone(), true)).await.unwrap();
        assert!(saved.enabled);
        assert!(store.is_enabled(&scope).await.unwrap());

        let updated = store.set(input(scope.clone(), false)).await.unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.created_at, saved.created_at);
        assert!(!store.is_enabled(&scope).await.unwrap());
    }

    #[tokio::test]
    async fn setting_is_agent_and_project_agnostic() {
        // A user-level toggle written without agent/project resolves the same
        // way at dispatch time when the runtime scope carries an agent/project.
        let store = InMemoryAutoApproveSettingStore::new();
        store
            .set(input(scope("alice", None, None), true))
            .await
            .unwrap();

        assert!(
            store
                .is_enabled(&scope("alice", Some("agent-x"), Some("project-y")))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn setting_isolates_users() {
        let store = InMemoryAutoApproveSettingStore::new();
        store
            .set(input(scope("alice", None, None), true))
            .await
            .unwrap();

        assert!(store.is_enabled(&scope("alice", None, None)).await.unwrap());
        assert!(!store.is_enabled(&scope("bob", None, None)).await.unwrap());
    }

    #[tokio::test]
    async fn filesystem_setting_survives_restart() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemAutoApproveSettingStore::new(Arc::clone(&scoped));
        let scope = scope("alice", None, None);

        store.set(input(scope.clone(), true)).await.unwrap();

        let reloaded = FilesystemAutoApproveSettingStore::new(scoped);
        assert!(reloaded.is_enabled(&scope).await.unwrap());
    }

    #[test]
    fn deserialize_versioned_record_rejects_key_mismatch() {
        let expected_key = AutoApproveSettingKey::from_resource_scope(&scope("alice", None, None));
        let stored_key = AutoApproveSettingKey::from_resource_scope(&scope("bob", None, None));
        let stored = AutoApproveSettingRecord {
            key: stored_key,
            enabled: true,
            updated_by: Principal::User(UserId::new("bob").unwrap()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let versioned = VersionedEntry {
            path: VirtualPath::new("/engine/record.json").unwrap(),
            entry: Entry::bytes(serialize(&stored).unwrap()).with_content_type(ContentType::json()),
            version: RecordVersion::from_backend(1),
        };

        let error = deserialize_versioned_record(&expected_key, versioned).unwrap_err();
        assert!(matches!(error, ToolPermissionStoreError::Integrity(_)));
    }
}
