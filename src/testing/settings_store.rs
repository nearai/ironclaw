use std::collections::HashMap;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::db::SettingsStore;
use crate::error::DatabaseError;
use crate::history::SettingRow;

/// Simple in-memory SettingsStore for tests.
pub struct MemorySettingsStore {
    values: RwLock<HashMap<(String, String), serde_json::Value>>,
}

impl MemorySettingsStore {
    pub fn new() -> Self {
        Self {
            values: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemorySettingsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SettingsStore for MemorySettingsStore {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        Ok(self
            .values
            .read()
            .await
            .get(&(user_id.to_string(), key.to_string()))
            .cloned())
    }

    async fn get_setting_full(
        &self,
        _user_id: &str,
        _key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        Ok(None)
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.values
            .write()
            .await
            .insert((user_id.to_string(), key.to_string()), value.clone());
        Ok(())
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        Ok(self
            .values
            .write()
            .await
            .remove(&(user_id.to_string(), key.to_string()))
            .is_some())
    }

    async fn list_settings(&self, _user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        Ok(Vec::new())
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        Ok(self
            .values
            .read()
            .await
            .iter()
            .filter(|((stored_user_id, _), _)| stored_user_id == user_id)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect())
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        let mut values = self.values.write().await;
        values.retain(|(stored_user_id, _), _| stored_user_id != user_id);
        for (key, value) in settings {
            values.insert((user_id.to_string(), key.clone()), value.clone());
        }
        Ok(())
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        Ok(self
            .values
            .read()
            .await
            .keys()
            .any(|(stored_user_id, _)| stored_user_id == user_id))
    }
}
