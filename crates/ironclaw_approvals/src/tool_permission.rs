use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{CapabilityId, Principal, ResourceScope, Timestamp};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::PersistentApprovalScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionState {
    AlwaysAllow,
    AskEachTime,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionOverride {
    AskEachTime,
    Disabled,
}

impl ToolPermissionOverride {
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
    #[error("tool permission override integrity error: {0}")]
    Integrity(String),
    #[error("tool permission override store error: {0}")]
    Store(String),
}

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
    async fn set(
        &self,
        input: ToolPermissionOverrideInput,
    ) -> Result<ToolPermissionOverrideRecord, ToolPermissionStoreError>;

    async fn get(
        &self,
        key: &ToolPermissionOverrideKey,
    ) -> Result<Option<ToolPermissionOverrideRecord>, ToolPermissionStoreError>;

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
