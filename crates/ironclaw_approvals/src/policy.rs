use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, FileType, FilesystemError, RecordVersion, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{
    Action, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId, GrantConstraints,
    HostApiError, PermissionMode, Principal, ProjectId, ResourceScope, ScopedPath, SystemServiceId,
    TenantId, UserId, sha256_digest_token,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cas_record::FilesystemCasRecordStore;

const POLICY_PREFIX: &str = "/approvals/persistent";
const POLICY_PATH_CACHE_MAX_ENTRIES: usize = 1024;
const POLICY_CAS_RETRY_ATTEMPTS: usize = 3;
const PERSISTENT_APPROVAL_GRANT_ISSUER: &str = "persistent-approval";

/// Returns whether an extension manifest permission mode may be upgraded by an
/// explicit "always allow" user decision.
///
/// `Ask` is eligible because it is the prompting posture for user-mediated
/// approval. Extension authors that require mandatory per-invocation consent
/// must use a separate gate that does not offer persistent approval.
pub fn permission_mode_allows_persistent_approval(permission: PermissionMode) -> bool {
    matches!(permission, PermissionMode::Allow | PermissionMode::Ask)
}

pub fn persistent_approval_grant_issuer() -> Principal {
    Principal::System(SystemServiceId::from_trusted(
        PERSISTENT_APPROVAL_GRANT_ISSUER.to_string(),
    ))
}

#[derive(Debug, Error)]
pub enum PersistentApprovalPolicyError {
    #[error("unknown persistent approval policy")]
    UnknownPolicy,
    #[error("persistent approval policy changed concurrently")]
    CasConflict,
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<FilesystemError> for PersistentApprovalPolicyError {
    fn from(error: FilesystemError) -> Self {
        if matches!(error, FilesystemError::VersionMismatch { .. }) {
            return Self::CasConflict;
        }
        Self::Filesystem(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistentApprovalAction {
    Dispatch,
    SpawnCapability,
}

impl PersistentApprovalAction {
    pub fn from_action(action: &Action) -> Option<(Self, CapabilityId)> {
        match action {
            Action::Dispatch { capability, .. } => Some((Self::Dispatch, capability.clone())),
            Action::SpawnCapability { capability, .. } => {
                Some((Self::SpawnCapability, capability.clone()))
            }
            _ => None,
        }
    }

    fn as_path_segment(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::SpawnCapability => "spawn_capability",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentApprovalScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<ironclaw_host_api::AgentId>,
    pub project_id: Option<ProjectId>,
}

impl PersistentApprovalScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersistentApprovalPolicyKey {
    pub scope: PersistentApprovalScope,
    pub action: PersistentApprovalAction,
    pub capability_id: CapabilityId,
    pub grantee: Principal,
}

impl PersistentApprovalPolicyKey {
    pub fn new(
        scope: &ResourceScope,
        action: PersistentApprovalAction,
        capability_id: CapabilityId,
        grantee: Principal,
    ) -> Self {
        Self {
            scope: PersistentApprovalScope::from_resource_scope(scope),
            action,
            capability_id,
            grantee,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentApprovalPolicy {
    pub key: PersistentApprovalPolicyKey,
    #[serde(default)]
    pub grant_id: CapabilityGrantId,
    pub approved_by: Principal,
    pub constraints: GrantConstraints,
    pub source_approval_request_id: Option<ApprovalRequestId>,
    pub created_at: ironclaw_host_api::Timestamp,
    pub updated_at: ironclaw_host_api::Timestamp,
    pub revoked_at: Option<ironclaw_host_api::Timestamp>,
}

impl PersistentApprovalPolicy {
    pub fn active_grant(&self) -> Option<CapabilityGrant> {
        if self.revoked_at.is_some()
            || self
                .constraints
                .expires_at
                .is_some_and(|expires_at| expires_at <= Utc::now())
        {
            return None;
        }
        Some(CapabilityGrant {
            id: self.grant_id,
            capability: self.key.capability_id.clone(),
            grantee: self.key.grantee.clone(),
            issued_by: persistent_approval_grant_issuer(),
            constraints: self.constraints.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentApprovalPolicyInput {
    pub scope: ResourceScope,
    pub action: PersistentApprovalAction,
    pub capability_id: CapabilityId,
    pub grantee: Principal,
    pub approved_by: Principal,
    pub constraints: GrantConstraints,
    pub source_approval_request_id: Option<ApprovalRequestId>,
}

#[async_trait]
pub trait PersistentApprovalPolicyStore: Send + Sync {
    /// Creates or refreshes a reusable persistent approval policy.
    ///
    /// `max_invocations` is always cleared; persistent policies are
    /// unlimited-use by design.
    async fn allow(
        &self,
        input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError>;

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError>;

    fn supports_scope_listing(&self) -> bool {
        false
    }

    async fn list_for_scope_action(
        &self,
        _scope: &ResourceScope,
        _action: PersistentApprovalAction,
    ) -> Result<Vec<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(Vec::new())
    }

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError>;

    /// Revokes only when the current policy came from the provided approval
    /// request. Returns `Ok(None)` when the policy is absent or has a different
    /// source request.
    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError>;
}

pub struct FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem,
{
    records: FilesystemCasRecordStore<F, PersistentApprovalPolicyKey>,
}

impl<F> FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            records: FilesystemCasRecordStore::new(filesystem, POLICY_PATH_CACHE_MAX_ENTRIES),
        }
    }
}

#[async_trait]
impl<F> PersistentApprovalPolicyStore for FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn allow(
        &self,
        mut input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        input.constraints.max_invocations = None;
        let scope = input.scope.clone();
        let key = PersistentApprovalPolicyKey::new(
            &scope,
            input.action,
            input.capability_id,
            input.grantee,
        );
        let path = self.cached_policy_path(&key)?;
        let lock = self.records.mutation_lock(&key);
        let _guard = lock.lock().await;
        for _ in 0..POLICY_CAS_RETRY_ATTEMPTS {
            let existing = self.lookup_versioned(&key).await?;
            let now = Utc::now();
            let (created_at, grant_id, cas) = existing.as_ref().map_or(
                (now, CapabilityGrantId::new(), CasExpectation::Absent),
                |(policy, version)| {
                    (
                        policy.created_at,
                        policy.grant_id,
                        CasExpectation::Version(*version),
                    )
                },
            );
            let policy = PersistentApprovalPolicy {
                key: key.clone(),
                grant_id,
                approved_by: input.approved_by.clone(),
                constraints: input.constraints.clone(),
                source_approval_request_id: input.source_approval_request_id,
                created_at,
                updated_at: now,
                revoked_at: None,
            };
            match self.write_policy_raw(&scope, &path, &policy, cas).await {
                Ok(()) => return Ok(policy),
                Err(PersistentApprovalPolicyError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(PersistentApprovalPolicyError::CasConflict)
    }

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(self
            .lookup_versioned(key)
            .await?
            .map(|(policy, _version)| policy))
    }

    fn supports_scope_listing(&self) -> bool {
        true
    }

    async fn list_for_scope_action(
        &self,
        scope: &ResourceScope,
        action: PersistentApprovalAction,
    ) -> Result<Vec<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        let target_scope = PersistentApprovalScope::from_resource_scope(scope);
        let root = policy_scope_action_path(&target_scope, action)?;
        let entries = match self.records.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        let mut policies = Vec::new();
        for entry in entries {
            if entry.file_type != FileType::File || !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_child_path(&root, &entry.name)?;
            let Some(versioned) = self.records.get(scope, &path).await? else {
                continue;
            };
            let policy = deserialize::<PersistentApprovalPolicy>(&versioned.entry.body)?;
            if policy.key.scope != target_scope || policy.key.action != action {
                tracing::error!(
                    stored_scope = ?policy.key.scope,
                    expected_scope = ?target_scope,
                    stored_action = ?policy.key.action,
                    expected_action = ?action,
                    "persistent approval policy scope/action mismatch while listing"
                );
                continue;
            }
            policies.push(policy);
        }
        Ok(policies)
    }

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        let scope = resource_scope_for_policy_key(key);
        let path = self.cached_policy_path(key)?;
        let lock = self.records.mutation_lock(key);
        let _guard = lock.lock().await;
        for _ in 0..POLICY_CAS_RETRY_ATTEMPTS {
            let (mut policy, version) = self
                .lookup_versioned(key)
                .await?
                .ok_or(PersistentApprovalPolicyError::UnknownPolicy)?;
            let now = Utc::now();
            policy.revoked_at = Some(now);
            policy.updated_at = now;
            match self
                .write_policy_raw(&scope, &path, &policy, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(policy),
                Err(PersistentApprovalPolicyError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(PersistentApprovalPolicyError::CasConflict)
    }

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        let scope = resource_scope_for_policy_key(key);
        let path = self.cached_policy_path(key)?;
        let lock = self.records.mutation_lock(key);
        let _guard = lock.lock().await;
        for _ in 0..POLICY_CAS_RETRY_ATTEMPTS {
            let Some((mut policy, version)) = self.lookup_versioned(key).await? else {
                return Ok(None);
            };
            if policy.source_approval_request_id != Some(source_approval_request_id) {
                return Ok(None);
            }
            let now = Utc::now();
            policy.revoked_at = Some(now);
            policy.updated_at = now;
            match self
                .write_policy_raw(&scope, &path, &policy, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(Some(policy)),
                Err(PersistentApprovalPolicyError::CasConflict) => continue,
                Err(error) => return Err(error),
            }
        }
        Err(PersistentApprovalPolicyError::CasConflict)
    }
}

impl<F> FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn lookup_versioned(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<(PersistentApprovalPolicy, RecordVersion)>, PersistentApprovalPolicyError>
    {
        let path = self.cached_policy_path(key)?;
        let scope = resource_scope_for_policy_key(key);
        self.lookup_versioned_at(key, &scope, &path).await
    }

    async fn lookup_versioned_at(
        &self,
        key: &PersistentApprovalPolicyKey,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<(PersistentApprovalPolicy, RecordVersion)>, PersistentApprovalPolicyError>
    {
        let Some(versioned) = self.records.get(scope, path).await? else {
            return Ok(None);
        };
        deserialize_versioned_policy(key, versioned)
    }

    async fn write_policy_raw(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        policy: &PersistentApprovalPolicy,
        expectation: CasExpectation,
    ) -> Result<(), PersistentApprovalPolicyError> {
        self.records
            .put_json(scope, path, serialize(policy)?, expectation)
            .await
    }

    fn cached_policy_path(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<ScopedPath, PersistentApprovalPolicyError> {
        self.records.cached_path(key, policy_path)
    }
}

fn deserialize_versioned_policy(
    key: &PersistentApprovalPolicyKey,
    versioned: VersionedEntry,
) -> Result<Option<(PersistentApprovalPolicy, RecordVersion)>, PersistentApprovalPolicyError> {
    let policy = deserialize::<PersistentApprovalPolicy>(&versioned.entry.body)?;
    if &policy.key == key {
        Ok(Some((policy, versioned.version)))
    } else {
        tracing::error!(
            stored = ?policy.key,
            expected = ?key,
            "persistent approval policy key mismatch"
        );
        Ok(None)
    }
}

fn policy_path(
    key: &PersistentApprovalPolicyKey,
) -> Result<ScopedPath, PersistentApprovalPolicyError> {
    ScopedPath::new(format!(
        "{}/{}/{}/{}.json",
        POLICY_PREFIX,
        within_tenant_scope(&key.scope),
        key.action.as_path_segment(),
        policy_digest(key)?
    ))
    .map_err(invalid_path)
}

fn policy_scope_action_path(
    scope: &PersistentApprovalScope,
    action: PersistentApprovalAction,
) -> Result<ScopedPath, PersistentApprovalPolicyError> {
    ScopedPath::new(format!(
        "{}/{}/{}",
        POLICY_PREFIX,
        within_tenant_scope(scope),
        action.as_path_segment()
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

fn scoped_child_path(
    parent: &ScopedPath,
    child: &str,
) -> Result<ScopedPath, PersistentApprovalPolicyError> {
    ScopedPath::new(format!(
        "{}/{}",
        parent.as_str().trim_end_matches('/'),
        child
    ))
    .map_err(invalid_path)
}

fn policy_digest(
    key: &PersistentApprovalPolicyKey,
) -> Result<String, PersistentApprovalPolicyError> {
    let bytes = serde_json::to_vec(key).map_err(serialization)?;
    let digest = sha256_digest_token(&bytes);
    // Safety: sha256_digest_token always returns "sha256:<hex>".
    Ok(digest
        .strip_prefix("sha256:")
        .unwrap_or(digest.as_str())
        .to_string())
}

fn resource_scope_for_policy_key(key: &PersistentApprovalPolicyKey) -> ResourceScope {
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

fn serialize<T>(value: &T) -> Result<Vec<u8>, PersistentApprovalPolicyError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(serialization)
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, PersistentApprovalPolicyError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(serialization)
}

fn serialization(error: serde_json::Error) -> PersistentApprovalPolicyError {
    PersistentApprovalPolicyError::Serialization(error.to_string())
}

fn invalid_path(error: HostApiError) -> PersistentApprovalPolicyError {
    PersistentApprovalPolicyError::InvalidPath(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{DiskFilesystem, InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, EffectKind, GrantConstraints, HostPath, InvocationId, MountAlias, MountGrant,
        MountPermissions, MountView, NetworkPolicy, ProjectId, ThreadId, VirtualPath,
    };

    use super::*;

    // The single production store, exercised over the in-memory filesystem
    // backend — the seam that replaced the deleted
    // `InMemoryPersistentApprovalPolicyStore`.
    fn memory_store() -> FilesystemPersistentApprovalPolicyStore<InMemoryBackend> {
        FilesystemPersistentApprovalPolicyStore::new(scoped_fs(
            Arc::new(InMemoryBackend::new()),
            "tenant-a",
            "alice",
        ))
    }

    #[test]
    fn permission_modes_allowed_for_persistent_approval_are_explicit() {
        assert!(permission_mode_allows_persistent_approval(
            PermissionMode::Allow
        ));
        assert!(permission_mode_allows_persistent_approval(
            PermissionMode::Ask
        ));
        assert!(!permission_mode_allows_persistent_approval(
            PermissionMode::Deny
        ));
    }

    #[tokio::test]
    async fn policy_revoke_removes_active_grant() {
        let store = memory_store();
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let policy = store.allow(input(scope)).await.expect("allow policy");
        assert!(policy.active_grant().is_some());

        let revoked = store.revoke(&key).await.expect("revoke policy");
        assert!(revoked.active_grant().is_none());
        assert!(
            store
                .lookup(&key)
                .await
                .expect("lookup")
                .expect("policy")
                .active_grant()
                .is_none()
        );
    }

    #[tokio::test]
    async fn filesystem_policy_store_survives_restart() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(Arc::clone(&scoped));
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let saved = store.allow(input(scope)).await.expect("allow policy");
        let reloaded = FilesystemPersistentApprovalPolicyStore::new(scoped)
            .lookup(&key)
            .await
            .expect("lookup")
            .expect("policy");

        assert_eq!(reloaded, saved);
        assert!(reloaded.active_grant().is_some());
    }

    #[tokio::test]
    async fn filesystem_policy_store_caches_policy_paths() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        store.allow(input(scope)).await.expect("allow policy");
        assert_eq!(store.records.path_cache_len(), 1);

        store.lookup(&key).await.expect("lookup policy");
        assert_eq!(store.records.path_cache_len(), 1);
    }

    #[tokio::test]
    async fn filesystem_policy_store_bounds_policy_path_cache() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(scoped);

        for index in 0..(POLICY_PATH_CACHE_MAX_ENTRIES + 2) {
            store
                .allow(input(scope(None, Some(&format!("thread-{index}")))))
                .await
                .expect("allow policy");
        }

        assert!(store.records.path_cache_len() <= POLICY_PATH_CACHE_MAX_ENTRIES);
    }

    #[tokio::test]
    async fn filesystem_policy_store_rejects_versioned_mutation_on_byte_only_backend() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut backend = DiskFilesystem::new();
        backend
            .mount_local(
                VirtualPath::new("/engine").unwrap(),
                HostPath::from_path_buf(tempdir.path().to_path_buf()),
            )
            .expect("mount local filesystem");
        let scoped = scoped_fs(Arc::new(backend), "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let first = store
            .allow(input(scope.clone()))
            .await
            .expect("allow first policy");
        let second_source = ApprovalRequestId::new();
        let mut second_input = input(scope);
        second_input.source_approval_request_id = Some(second_source);

        assert!(matches!(
            store.allow(second_input).await,
            Err(PersistentApprovalPolicyError::Filesystem(_))
        ));
        assert!(matches!(
            store.revoke(&key).await,
            Err(PersistentApprovalPolicyError::Filesystem(_))
        ));
        assert_eq!(
            store.lookup(&key).await.unwrap().expect("policy"),
            first,
            "unsupported versioned mutations must not overwrite the existing policy"
        );
    }

    #[tokio::test]
    async fn filesystem_policy_store_evicts_idle_mutation_locks() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(scoped);

        store
            .allow(input(scope(None, Some("thread-a"))))
            .await
            .expect("allow first policy");
        store
            .allow(input(scope(None, Some("thread-b"))))
            .await
            .expect("allow second policy");

        assert!(store.records.mutation_lock_count() <= 1);
    }

    #[tokio::test]
    async fn revoke_if_source_approval_request_preserves_newer_policy() {
        let store = memory_store();
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);
        let first_source = ApprovalRequestId::new();
        let second_source = ApprovalRequestId::new();

        let mut first = input(scope.clone());
        first.source_approval_request_id = Some(first_source);
        store.allow(first).await.expect("allow first policy");
        let mut second = input(scope);
        second.source_approval_request_id = Some(second_source);
        store.allow(second).await.expect("allow second policy");

        let revoked = store
            .revoke_if_source_approval_request(&key, first_source)
            .await
            .expect("conditional revoke");

        assert!(revoked.is_none());
        let current = store.lookup(&key).await.expect("lookup").expect("policy");
        assert_eq!(current.source_approval_request_id, Some(second_source));
        assert!(current.active_grant().is_some());
    }

    #[tokio::test]
    async fn revoke_if_source_approval_request_returns_none_for_absent_policy() {
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);
        let source = ApprovalRequestId::new();

        let store = memory_store();
        assert!(
            store
                .revoke_if_source_approval_request(&key, source)
                .await
                .expect("conditional revoke")
                .is_none()
        );
    }

    #[tokio::test]
    async fn filesystem_revoke_if_source_approval_request_revokes_matching_source() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(backend, "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(scoped);
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);
        let source = ApprovalRequestId::new();
        let mut input = input(scope);
        input.source_approval_request_id = Some(source);

        store.allow(input).await.expect("allow policy");
        let revoked = store
            .revoke_if_source_approval_request(&key, source)
            .await
            .expect("conditional revoke")
            .expect("revoked policy");

        assert!(revoked.active_grant().is_none());
        assert!(
            store
                .lookup(&key)
                .await
                .expect("lookup revoked policy")
                .expect("policy")
                .active_grant()
                .is_none()
        );
    }

    #[test]
    fn policy_scope_prefers_project_over_thread() {
        // Project is still part of the scope; differing thread ids under the same
        // project produce identical keys.
        let scope_a = scope(Some("project-a"), Some("thread-a"));
        let scope_b = scope(Some("project-a"), Some("thread-b"));

        let derived_a = PersistentApprovalScope::from_resource_scope(&scope_a);
        let derived_b = PersistentApprovalScope::from_resource_scope(&scope_b);

        assert_eq!(derived_a, derived_b);
        assert_eq!(
            derived_a.project_id,
            Some(ProjectId::new("project-a").unwrap())
        );
    }

    #[test]
    fn project_scoped_policy_key_serialization_is_threadless() {
        // Persistent approvals intentionally ignore thread_id. There is no
        // backward-compatibility read path for old local test records, so the
        // key serialization should not retain thread_id.
        let key = key_for(&scope(Some("project-a"), Some("thread-ignored")));
        let json = serde_json::to_string(&key).expect("serialize policy key");
        assert!(
            !json.contains("thread_id"),
            "persistent approval keys must not serialize thread_id; got {json}"
        );

        let digest_thread_one =
            policy_digest(&key_for(&scope(Some("project-a"), Some("thread-1")))).expect("digest");
        let digest_thread_two =
            policy_digest(&key_for(&scope(Some("project-a"), Some("thread-2")))).expect("digest");
        assert_eq!(digest_thread_one, digest_thread_two);
        assert_eq!(
            within_tenant_scope(&key.scope),
            "agents/agent-a/projects/project-a"
        );
    }

    #[tokio::test]
    async fn filesystem_project_scoped_policy_matches_in_new_thread_after_reload() {
        // A project-scoped "always allow" applies across threads in the same
        // project. A fresh store instance looks the policy up from a different
        // thread and finds it through the canonical path.
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_fs(Arc::clone(&backend), "tenant-a", "alice");
        let store = FilesystemPersistentApprovalPolicyStore::new(Arc::clone(&scoped));

        let granted = store
            .allow(input(scope(Some("project-a"), Some("thread-1"))))
            .await
            .expect("allow project-scoped policy");

        let new_thread_key = key_for(&scope(Some("project-a"), Some("thread-2")));
        let reloaded = FilesystemPersistentApprovalPolicyStore::new(scoped)
            .lookup(&new_thread_key)
            .await
            .expect("lookup")
            .expect("pre-existing project-scoped policy still matches in a new thread");

        assert_eq!(reloaded, granted);
        assert!(reloaded.active_grant().is_some());
    }

    #[test]
    fn policy_scope_without_project_is_thread_agnostic() {
        let scope_a = scope(None, Some("thread-a"));
        let scope_b = scope(None, Some("thread-b"));

        let derived_a = PersistentApprovalScope::from_resource_scope(&scope_a);
        let derived_b = PersistentApprovalScope::from_resource_scope(&scope_b);

        assert_eq!(derived_a, derived_b);
    }

    #[test]
    fn policy_scope_without_project_isolates_users() {
        let scope_a = ResourceScope {
            user_id: UserId::new("alice").unwrap(),
            ..scope(None, Some("thread-a"))
        };
        let scope_b = ResourceScope {
            user_id: UserId::new("bob").unwrap(),
            ..scope(None, Some("thread-a"))
        };

        assert_ne!(
            PersistentApprovalScope::from_resource_scope(&scope_a),
            PersistentApprovalScope::from_resource_scope(&scope_b)
        );
    }

    #[test]
    fn policy_scope_without_project_isolates_agents() {
        let scope_a = ResourceScope {
            agent_id: Some(AgentId::new("agent-x").unwrap()),
            ..scope(None, Some("thread-a"))
        };
        let scope_b = ResourceScope {
            agent_id: Some(AgentId::new("agent-y").unwrap()),
            ..scope(None, Some("thread-a"))
        };

        assert_ne!(
            PersistentApprovalScope::from_resource_scope(&scope_a),
            PersistentApprovalScope::from_resource_scope(&scope_b)
        );
    }

    #[tokio::test]
    async fn active_grant_returns_none_for_expired_policy() {
        let store = memory_store();
        let scope = scope(None, Some("thread-a"));
        let mut input = input(scope);
        input.constraints.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));

        let policy = store.allow(input).await.expect("allow policy");

        assert!(policy.active_grant().is_none());
    }

    #[tokio::test]
    async fn active_grant_reuses_persisted_policy_grant_id() {
        let store = memory_store();
        let scope = scope(None, Some("thread-a"));
        let key = key_for(&scope);

        let policy = store.allow(input(scope)).await.expect("allow policy");
        let first_grant = policy.active_grant().expect("active grant");
        let second_grant = policy.active_grant().expect("active grant");
        let reloaded = store.lookup(&key).await.expect("lookup policy").unwrap();
        let reloaded_grant = reloaded.active_grant().expect("active grant");

        assert_eq!(policy.grant_id, first_grant.id);
        assert_eq!(first_grant.id, second_grant.id);
        assert_eq!(first_grant.id, reloaded_grant.id);
    }

    #[tokio::test]
    async fn active_grant_uses_persistent_approval_issuer_marker() {
        let store = memory_store();
        let scope = scope(None, Some("thread-a"));

        let policy = store.allow(input(scope)).await.expect("allow policy");
        let grant = policy.active_grant().expect("active grant");

        assert_eq!(
            policy.approved_by,
            Principal::User(UserId::new("alice").unwrap())
        );
        assert_eq!(grant.issued_by, persistent_approval_grant_issuer());
    }

    fn input(scope: ResourceScope) -> PersistentApprovalPolicyInput {
        PersistentApprovalPolicyInput {
            scope,
            action: PersistentApprovalAction::Dispatch,
            capability_id: CapabilityId::new("fixture.echo").unwrap(),
            grantee: Principal::User(UserId::new("alice").unwrap()),
            approved_by: Principal::User(UserId::new("alice").unwrap()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
            source_approval_request_id: Some(ApprovalRequestId::new()),
        }
    }

    fn key_for(scope: &ResourceScope) -> PersistentApprovalPolicyKey {
        PersistentApprovalPolicyKey::new(
            scope,
            PersistentApprovalAction::Dispatch,
            CapabilityId::new("fixture.echo").unwrap(),
            Principal::User(UserId::new("alice").unwrap()),
        )
    }

    fn scope(project_id: Option<&str>, thread_id: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("alice").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
            project_id: project_id.map(|id| ProjectId::new(id).unwrap()),
            mission_id: None,
            thread_id: thread_id.map(|id| ThreadId::new(id).unwrap()),
            invocation_id: InvocationId::new(),
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
}
