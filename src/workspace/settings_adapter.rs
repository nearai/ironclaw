//! Settings adapter that bridges `SettingsStore` to workspace documents.
//!
//! During migration, this adapter dual-writes settings to both the old
//! `settings` table and workspace documents at `.system/settings/`.
//! Per-key reads (`get_setting`, `get_setting_full`) prefer the workspace
//! and fall back to the legacy table. Aggregate reads (`list_settings`,
//! `get_all_settings`) currently always read from the legacy store, which
//! remains the source of truth for "list everything" until migration is
//! complete.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::debug;

use crate::db::{Database, SettingsStore};
use crate::error::DatabaseError;
use crate::history::SettingRow;
use crate::workspace::Workspace;
use crate::workspace::settings_schemas::{schema_for_key, settings_path, validate_settings_key};

/// Implements `SettingsStore` by reading/writing workspace documents at
/// `.system/settings/{key}.json`. Falls back to the legacy `settings` table
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

    /// Ensure the `.system/.config` document exists with system defaults.
    ///
    /// Called once during startup to seed the system folder configuration.
    /// Errors are propagated so startup can fail fast if the system config
    /// cannot be enforced — leaving `.system/` indexed by accident would
    /// pollute search results with internal state.
    ///
    /// The `.config` doc's `metadata` column is what descendants inherit
    /// via `find_nearest_config` — so we set `skip_indexing: true` (system
    /// state should never appear in search) and explicitly `skip_versioning:
    /// false` so all `.system/**` documents (settings, extension state,
    /// skill manifests) ARE versioned for audit trail. The doc's content is
    /// a human-readable JSON summary of what gets inherited.
    pub async fn ensure_system_config(&self) -> Result<(), DatabaseError> {
        let config_path = ".system/.config";
        if self
            .workspace
            .exists(config_path)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace exists check failed: {e}")))?
        {
            return Ok(());
        }

        let inherited_metadata = serde_json::json!({
            "skip_indexing": true,
            "skip_versioning": false,
            "hygiene": { "enabled": false }
        });
        let doc = self
            .workspace
            .write(config_path, &inherited_metadata.to_string())
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace write failed: {e}")))?;

        // The .config doc's metadata column is the inheritance source for
        // descendants. Mirror the JSON content so future readers see the
        // same values via either path. `skip_versioning: false` is critical:
        // changing it to `true` here would silently disable versioning for
        // every document under `.system/**`.
        self.workspace
            .update_metadata(doc.id, &inherited_metadata)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace metadata update failed: {e}")))?;

        debug!("seeded .system/.config for workspace settings");
        Ok(())
    }

    /// Write a setting to workspace with optional schema in metadata.
    async fn write_to_workspace(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        validate_settings_key(key)
            .map_err(|e| DatabaseError::Query(format!("invalid settings key '{key}': {e}")))?;
        let path = settings_path(key);
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        // Validate against the known schema BEFORE the first write so the
        // initial document creation cannot bypass schema enforcement. Once
        // metadata is set below, subsequent writes are validated by the
        // workspace itself via the resolved metadata chain.
        if let Some(schema) = schema_for_key(key) {
            crate::workspace::schema::validate_content_against_schema(&path, &content, &schema)
                .map_err(|e| DatabaseError::Query(format!("schema validation failed: {e}")))?;
        }

        // Write the content
        let doc = self
            .workspace
            .write(&path, &content)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace write failed: {e}")))?;

        // Persist the schema in metadata so future writes are validated
        // automatically by the workspace write path. Propagate errors so a
        // metadata-update failure doesn't silently leave the doc un-typed.
        if let Some(schema) = schema_for_key(key) {
            self.workspace
                .update_metadata(
                    doc.id,
                    &serde_json::json!({
                        "schema": schema,
                        "skip_indexing": true
                    }),
                )
                .await
                .map_err(|e| {
                    DatabaseError::Query(format!(
                        "failed to persist schema metadata for '{key}': {e}"
                    ))
                })?;
        }

        Ok(())
    }

    /// Read a setting from workspace, returning the parsed JSON value.
    async fn read_from_workspace(
        &self,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        if validate_settings_key(key).is_err() {
            return Ok(None);
        }
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
        // Try workspace first (only for valid keys; invalid keys can never
        // exist in workspace and must not be used to construct paths).
        if validate_settings_key(key).is_err() {
            return self.legacy_store.get_setting_full(user_id, key).await;
        }
        let path = settings_path(key);
        if let Ok(doc) = self.workspace.read(&path).await
            && !doc.content.is_empty()
        {
            let value: serde_json::Value = serde_json::from_str(&doc.content)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            return Ok(Some(SettingRow {
                key: key.to_string(),
                value,
                updated_at: doc.updated_at,
            }));
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
        // Delete from both. Skip workspace for invalid keys (cannot exist there).
        if validate_settings_key(key).is_ok() {
            let path = settings_path(key);
            let _ = self.workspace.delete(&path).await;
        }
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
        // Dual-write each setting to workspace. Collect the first error so
        // partial-migration state is observable, but always run the legacy
        // write so the legacy table stays the source of truth during
        // migration even if some workspace writes fail.
        let mut workspace_error: Option<DatabaseError> = None;
        for (key, value) in settings {
            if let Err(e) = self.write_to_workspace(key, value).await {
                debug!(key = %key, error = %e, "workspace write failed in set_all_settings");
                if workspace_error.is_none() {
                    workspace_error = Some(e);
                }
            }
        }

        self.legacy_store
            .set_all_settings(user_id, settings)
            .await?;

        if let Some(err) = workspace_error {
            return Err(err);
        }
        Ok(())
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
        adapter.ensure_system_config().await.unwrap();

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
        adapter.ensure_system_config().await.unwrap();

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
