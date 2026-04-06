//! Settings adapter that bridges `SettingsStore` to workspace documents.
//!
//! During migration, this adapter dual-writes settings to both the old
//! `settings` table and workspace documents at `_system/settings/`.
//! Reads prefer workspace, falling back to the legacy table.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::debug;

use crate::db::{Database, SettingsStore};
use crate::error::DatabaseError;
use crate::history::SettingRow;
use crate::workspace::Workspace;
use crate::workspace::settings_schemas::{schema_for_key, settings_path};

/// Implements `SettingsStore` by reading/writing workspace documents at
/// `_system/settings/{key}.json`. Falls back to the legacy `settings` table
/// for reads during migration.
pub struct WorkspaceSettingsAdapter {
    workspace: Arc<Workspace>,
    legacy_store: Arc<dyn Database>,
}

impl WorkspaceSettingsAdapter {
    pub fn new(workspace: Arc<Workspace>, legacy_store: Arc<dyn Database>) -> Self {
        Self {
            workspace,
            legacy_store,
        }
    }

    /// Ensure the `_system/.config` document exists with system defaults.
    ///
    /// Called once during startup to seed the system folder configuration.
    pub async fn ensure_system_config(&self) {
        let config_path = "_system/.config";
        match self.workspace.exists(config_path).await {
            Ok(true) => {}
            _ => {
                let config_content = serde_json::json!({
                    "skip_indexing": true,
                    "skip_versioning": false,
                    "hygiene": { "enabled": false }
                });
                if let Ok(doc) = self
                    .workspace
                    .write(config_path, &config_content.to_string())
                    .await
                {
                    let _ = self
                        .workspace
                        .update_metadata(
                            doc.id,
                            &serde_json::json!({
                                "skip_indexing": true,
                                "skip_versioning": true,
                                "hygiene": { "enabled": false }
                            }),
                        )
                        .await;
                    debug!("seeded _system/.config for workspace settings");
                }
            }
        }
    }

    /// Write a setting to workspace with optional schema in metadata.
    async fn write_to_workspace(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let path = settings_path(key);
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        // Write the content first
        let doc = self
            .workspace
            .write(&path, &content)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace write failed: {e}")))?;

        // Set schema in metadata if this is a known key
        if let Some(schema) = schema_for_key(key) {
            let _ = self
                .workspace
                .update_metadata(
                    doc.id,
                    &serde_json::json!({
                        "schema": schema,
                        "skip_indexing": true
                    }),
                )
                .await;
        }

        Ok(())
    }

    /// Read a setting from workspace, returning the parsed JSON value.
    async fn read_from_workspace(
        &self,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        let path = settings_path(key);
        match self.workspace.read(&path).await {
            Ok(doc) => {
                if doc.content.is_empty() {
                    return Ok(None);
                }
                let value: serde_json::Value = serde_json::from_str(&doc.content)
                    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Err(_) => Ok(None),
        }
    }
}

#[async_trait]
impl SettingsStore for WorkspaceSettingsAdapter {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        // Try workspace first
        if let Some(value) = self.read_from_workspace(key).await? {
            return Ok(Some(value));
        }
        // Fall back to legacy table
        self.legacy_store.get_setting(user_id, key).await
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        // Try workspace first
        let path = settings_path(key);
        if let Ok(doc) = self.workspace.read(&path).await {
            if !doc.content.is_empty() {
                let value: serde_json::Value = serde_json::from_str(&doc.content)
                    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
                return Ok(Some(SettingRow {
                    key: key.to_string(),
                    value,
                    updated_at: doc.updated_at,
                }));
            }
        }
        // Fall back to legacy table
        self.legacy_store.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        // Dual-write: workspace + legacy table
        self.write_to_workspace(key, value).await?;
        self.legacy_store.set_setting(user_id, key, value).await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        // Delete from both
        let path = settings_path(key);
        let _ = self.workspace.delete(&path).await;
        self.legacy_store.delete_setting(user_id, key).await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        // Use legacy store as source of truth during migration
        // (it has all keys; workspace may be partially populated)
        self.legacy_store.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.legacy_store.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        // Dual-write each setting to workspace
        for (key, value) in settings {
            let _ = self.write_to_workspace(key, value).await;
        }
        self.legacy_store.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.legacy_store.has_settings(user_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_workspace_settings() {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("settings_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));

        let adapter = WorkspaceSettingsAdapter::new(ws, db);
        adapter.ensure_system_config().await;

        // Write a setting
        adapter
            .set_setting("test_user", "llm_backend", &serde_json::json!("anthropic"))
            .await
            .unwrap();

        // Read it back — should come from workspace
        let value = adapter
            .get_setting("test_user", "llm_backend")
            .await
            .unwrap();
        assert_eq!(value, Some(serde_json::json!("anthropic")));

        // Read full setting
        let full = adapter
            .get_setting_full("test_user", "llm_backend")
            .await
            .unwrap();
        assert!(full.is_some());
        assert_eq!(full.unwrap().value, serde_json::json!("anthropic"));
    }

    #[tokio::test]
    async fn delete_removes_from_workspace() {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("settings_del_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));

        let adapter = WorkspaceSettingsAdapter::new(ws, db);
        adapter.ensure_system_config().await;

        adapter
            .set_setting("test_user", "test_key", &serde_json::json!(42))
            .await
            .unwrap();

        let deleted = adapter
            .delete_setting("test_user", "test_key")
            .await
            .unwrap();
        assert!(deleted);

        // Should not be found in workspace anymore
        let value = adapter.get_setting("test_user", "test_key").await.unwrap();
        assert!(value.is_none());
    }
}
