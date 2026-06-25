use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    Action, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId, GrantConstraints,
    PermissionMode, Principal, ProjectId, ResourceScope, SystemServiceId, TenantId, UserId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PERSISTENT_APPROVAL_GRANT_ISSUER: &str = "persistent-approval";

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
    #[error("persistent approval policy store error: {0}")]
    Store(String),
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

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError>;
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
        let key = PersistentApprovalPolicyKey::new(
            &input.scope,
            input.action,
            input.capability_id,
            input.grantee,
        );
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

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(policy) = policies.get_mut(key) else {
            return Ok(None);
        };
        if policy.source_approval_request_id != Some(source_approval_request_id) {
            return Ok(None);
        }
        let now = Utc::now();
        policy.revoked_at = Some(now);
        policy.updated_at = now;
        Ok(Some(policy.clone()))
    }
}
