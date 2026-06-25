use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{Principal, ResourceScope, TenantId, Timestamp, UserId};
use serde::{Deserialize, Serialize};

use crate::ToolPermissionStoreError;

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
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, ToolPermissionStoreError>;

    async fn get(
        &self,
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, ToolPermissionStoreError>;

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
