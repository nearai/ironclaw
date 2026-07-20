use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed local-dev access role persisted on trigger-fire access rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalTriggerAccessRole {
    /// Owner-level local trigger-fire access.
    Owner,
}

impl LocalTriggerAccessRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
        }
    }
}

/// Local-dev bootstrap path that owns a trigger-fire access row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalTriggerAccessSource {
    /// Environment-token `serve` bootstrap path.
    LocalDevEnvBootstrap,
    /// SSO-admitted WebUI user bootstrap path.
    LocalDevSsoBootstrap,
    /// CLI `run` default-owner bootstrap path.
    LocalDevRunBootstrap,
}

impl LocalTriggerAccessSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::LocalDevEnvBootstrap => "local_dev_env_bootstrap",
            Self::LocalDevSsoBootstrap => "local_dev_sso_bootstrap",
            Self::LocalDevRunBootstrap => "local_dev_run_bootstrap",
        }
    }
}

/// Fixed lifecycle state persisted on local-dev access rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LocalTriggerAccessStatus {
    Active,
    Inactive,
}

impl LocalTriggerAccessStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
        }
    }
}

/// Failure modes of a local trigger access store.
#[derive(Debug, Error)]
pub enum RebornLocalTriggerAccessStoreError {
    /// The backend (connect / migrate / query / commit) failed.
    #[error("reborn local trigger access store backend failure: {0}")]
    Backend(String),
}

/// Local-dev trigger access row to seed from trusted host/operator input.
pub struct LocalTriggerAccessSeed<'a> {
    /// Tenant scope for the local access row.
    pub tenant_id: &'a TenantId,
    /// User that is allowed to fire triggers for the exact scope.
    pub user_id: &'a UserId,
    /// Optional agent scope. `None` is stored as an exact no-agent scope, not
    /// a wildcard.
    pub agent_id: Option<&'a AgentId>,
    /// Optional project scope. `None` is stored as an exact no-project scope,
    /// not a wildcard.
    pub project_id: Option<&'a ProjectId>,
    /// Local role to persist on the access row.
    pub role: LocalTriggerAccessRole,
    /// Source for the host/operator seed path.
    pub source: LocalTriggerAccessSource,
}

/// Current trusted local-dev access set for one bootstrap source and exact
/// tenant/agent/project scope.
pub struct LocalTriggerAccessReconciliation<'a> {
    /// Tenant scope for the local access rows.
    pub tenant_id: &'a TenantId,
    /// Users that should keep active access for this bootstrap source/scope.
    pub user_ids: &'a [UserId],
    /// Optional agent scope. `None` is an exact no-agent scope, not a wildcard.
    pub agent_id: Option<&'a AgentId>,
    /// Optional project scope. `None` is an exact no-project scope, not a
    /// wildcard.
    pub project_id: Option<&'a ProjectId>,
    /// Local role for newly inserted rows.
    pub role: LocalTriggerAccessRole,
    /// Source for this host/operator seed path.
    pub source: LocalTriggerAccessSource,
}

/// Backend-neutral local trigger access repository contract.
#[async_trait::async_trait]
pub trait LocalTriggerAccessStore: Send + Sync {
    async fn seed_local_access(
        &self,
        seed: LocalTriggerAccessSeed<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError>;

    async fn reconcile_local_access(
        &self,
        reconciliation: LocalTriggerAccessReconciliation<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError>;

    async fn has_active_local_access(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        agent_id: Option<&AgentId>,
        project_id: Option<&ProjectId>,
    ) -> Result<bool, RebornLocalTriggerAccessStoreError>;
}

pub(super) fn backend(err: impl std::fmt::Display) -> RebornLocalTriggerAccessStoreError {
    RebornLocalTriggerAccessStoreError::Backend(err.to_string())
}

pub(super) fn optional_scope_key(value: Option<&str>) -> &str {
    value.unwrap_or("")
}
