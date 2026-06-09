use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    Action, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId, GrantConstraints,
    HostApiError, PermissionMode, Principal, ProjectId, ResourceScope, ScopedPath, TenantId,
    ThreadId, UserId, sha256_digest_token,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const POLICY_PREFIX: &str = "/approvals/persistent";

pub fn permission_mode_allows_persistent_approval(permission: PermissionMode) -> bool {
    matches!(permission, PermissionMode::Allow)
}

#[derive(Debug, Error)]
pub enum PersistentApprovalPolicyError {
    #[error("persistent approval scope must include project_id or thread_id")]
    UnsupportedScope,
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
    pub thread_id: Option<ThreadId>,
}

impl PersistentApprovalScope {
    pub fn from_resource_scope(
        scope: &ResourceScope,
    ) -> Result<Self, PersistentApprovalPolicyError> {
        if scope.project_id.is_none() && scope.thread_id.is_none() {
            return Err(PersistentApprovalPolicyError::UnsupportedScope);
        }
        let thread_id = if scope.project_id.is_some() {
            None
        } else {
            scope.thread_id.clone()
        };
        Ok(Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            thread_id,
        })
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
    ) -> Result<Self, PersistentApprovalPolicyError> {
        Ok(Self {
            scope: PersistentApprovalScope::from_resource_scope(scope)?,
            action,
            capability_id,
            grantee,
        })
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
            issued_by: self.approved_by.clone(),
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
    async fn allow(
        &self,
        input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError>;

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError>;

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError>;
}

#[derive(Debug, Default)]
pub struct InMemoryPersistentApprovalPolicyStore {
    policies: RwLock<HashMap<PersistentApprovalPolicyKey, PersistentApprovalPolicy>>,
}

impl InMemoryPersistentApprovalPolicyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PersistentApprovalPolicyStore for InMemoryPersistentApprovalPolicyStore {
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
        )?;
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Utc::now();
        let (created_at, grant_id) = policies
            .get(&key)
            .map_or((now, CapabilityGrantId::new()), |existing| {
                (existing.created_at, existing.grant_id)
            });
        let policy = PersistentApprovalPolicy {
            key: key.clone(),
            grant_id,
            approved_by: input.approved_by,
            constraints: input.constraints,
            source_approval_request_id: input.source_approval_request_id,
            created_at,
            updated_at: now,
            revoked_at: None,
        };
        policies.insert(key, policy.clone());
        Ok(policy)
    }

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(self
            .policies
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned())
    }

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let policy = policies
            .get_mut(key)
            .ok_or(PersistentApprovalPolicyError::UnknownPolicy)?;
        let now = Utc::now();
        policy.revoked_at = Some(now);
        policy.updated_at = now;
        Ok(policy.clone())
    }
}

pub struct FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemPersistentApprovalPolicyStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    fn record_entry(
        policy: &PersistentApprovalPolicy,
    ) -> Result<Entry, PersistentApprovalPolicyError> {
        Ok(Entry::bytes(serialize(policy)?).with_content_type(ContentType::json()))
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
        )?;
        let path = policy_path(&key)?;
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
            key,
            grant_id,
            approved_by: input.approved_by,
            constraints: input.constraints,
            source_approval_request_id: input.source_approval_request_id,
            created_at,
            updated_at: now,
            revoked_at: None,
        };
        self.filesystem
            .put(&scope, &path, Self::record_entry(&policy)?, cas)
            .await?;
        Ok(policy)
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

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        let (mut policy, version) = self
            .lookup_versioned(key)
            .await?
            .ok_or(PersistentApprovalPolicyError::UnknownPolicy)?;
        let now = Utc::now();
        policy.revoked_at = Some(now);
        policy.updated_at = now;
        let scope = resource_scope_for_policy_key(key);
        self.filesystem
            .put(
                &scope,
                &policy_path(key)?,
                Self::record_entry(&policy)?,
                CasExpectation::Version(version),
            )
            .await?;
        Ok(policy)
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
        let path = policy_path(key)?;
        let scope = resource_scope_for_policy_key(key);
        let Some(versioned) = self.filesystem.get(&scope, &path).await? else {
            return Ok(None);
        };
        deserialize_versioned_policy(key, versioned)
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

fn within_tenant_scope(scope: &PersistentApprovalScope) -> String {
    let mut segments = Vec::new();
    if let Some(agent_id) = &scope.agent_id {
        segments.push(format!("agents/{agent_id}"));
    }
    if let Some(project_id) = &scope.project_id {
        segments.push(format!("projects/{project_id}"));
    } else if let Some(thread_id) = &scope.thread_id {
        segments.push(format!("threads/{thread_id}"));
    }
    if segments.is_empty() {
        "scope".to_string()
    } else {
        segments.join("/")
    }
}

fn policy_digest(
    key: &PersistentApprovalPolicyKey,
) -> Result<String, PersistentApprovalPolicyError> {
    let bytes = serde_json::to_vec(key).map_err(serialization)?;
    let digest = sha256_digest_token(&bytes);
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
        thread_id: key.scope.thread_id.clone(),
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

    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, EffectKind, GrantConstraints, InvocationId, MountAlias, MountGrant,
        MountPermissions, MountView, NetworkPolicy, ProjectId, VirtualPath,
    };

    use super::*;

    #[tokio::test]
    async fn in_memory_policy_revoke_removes_active_grant() {
        let store = InMemoryPersistentApprovalPolicyStore::new();
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
    async fn policy_scope_prefers_project_over_thread() {
        let scope_a = scope(Some("project-a"), Some("thread-a"));
        let scope_b = scope(Some("project-a"), Some("thread-b"));

        assert_eq!(
            PersistentApprovalScope::from_resource_scope(&scope_a).unwrap(),
            PersistentApprovalScope::from_resource_scope(&scope_b).unwrap()
        );
    }

    #[tokio::test]
    async fn policy_scope_uses_thread_without_project() {
        let scope_a = scope(None, Some("thread-a"));
        let scope_b = scope(None, Some("thread-b"));

        assert_ne!(
            PersistentApprovalScope::from_resource_scope(&scope_a).unwrap(),
            PersistentApprovalScope::from_resource_scope(&scope_b).unwrap()
        );
    }

    #[tokio::test]
    async fn active_grant_returns_none_for_expired_policy() {
        let store = InMemoryPersistentApprovalPolicyStore::new();
        let scope = scope(None, Some("thread-a"));
        let mut input = input(scope);
        input.constraints.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));

        let policy = store.allow(input).await.expect("allow policy");

        assert!(policy.active_grant().is_none());
    }

    #[tokio::test]
    async fn active_grant_reuses_persisted_policy_grant_id() {
        let store = InMemoryPersistentApprovalPolicyStore::new();
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
    async fn from_resource_scope_errs_without_project_or_thread() {
        let scope = scope(None, None);

        assert!(matches!(
            PersistentApprovalScope::from_resource_scope(&scope),
            Err(PersistentApprovalPolicyError::UnsupportedScope)
        ));
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
        .unwrap()
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
