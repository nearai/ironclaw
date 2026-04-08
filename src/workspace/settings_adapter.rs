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
use tokio::sync::OnceCell;
use tracing::debug;

use crate::db::{Database, SettingsStore};
use crate::error::DatabaseError;
use crate::history::SettingRow;
use crate::workspace::Workspace;
use crate::workspace::settings_schemas::{schema_for_key, settings_path, validate_settings_key};

/// Returns true if `actual` matches the expected `.system/.config` metadata
/// closely enough that no repair is needed. The check is permissive: an
/// older/newer adapter version may have written extra fields, but the
/// load-bearing flags are:
///
/// - `skip_indexing == true` (so descendants are excluded from search)
/// - `skip_versioning != true` (versioning must NOT be silently disabled —
///   absent or `false` is fine)
/// - `hygiene.enabled != true` (system state must not be auto-cleaned)
fn system_config_metadata_matches(
    actual: &serde_json::Value,
    _expected: &serde_json::Value,
) -> bool {
    let skip_indexing_ok = actual
        .get("skip_indexing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let versioning_ok = actual.get("skip_versioning").and_then(|v| v.as_bool()) != Some(true);
    let hygiene_ok = actual
        .get("hygiene")
        .and_then(|h| h.get("enabled"))
        .and_then(|v| v.as_bool())
        != Some(true);
    skip_indexing_ok && versioning_ok && hygiene_ok
}

/// Implements `SettingsStore` by reading/writing workspace documents at
/// `.system/settings/{key}.json`. Falls back to the legacy `settings` table
/// for reads during migration.
pub struct WorkspaceSettingsAdapter {
    workspace: Arc<Workspace>,
    legacy_store: Arc<dyn Database>,
    /// Guards the lazy `ensure_system_config()` call so it runs at most once
    /// per adapter instance regardless of which write path triggers it. This
    /// removes the requirement that callers run `ensure_system_config()` at
    /// startup before any setting write — see the comment on `set_setting`.
    system_config_seeded: OnceCell<()>,
}

impl WorkspaceSettingsAdapter {
    pub fn new(workspace: Arc<Workspace>, legacy_store: Arc<dyn Database>) -> Self {
        Self {
            workspace,
            legacy_store,
            system_config_seeded: OnceCell::new(),
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
        let expected = serde_json::json!({
            "skip_indexing": true,
            "skip_versioning": false,
            "hygiene": { "enabled": false }
        });

        // If the doc already exists, verify its metadata column matches the
        // expected inherited values and repair it if it diverges. Older
        // workspaces (pre-PR or pre-fix #3042846635) may have a `.config`
        // doc whose metadata silently disables versioning for `.system/**`,
        // and we need this seeding to be idempotent across upgrades.
        if self
            .workspace
            .exists(config_path)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace exists check failed: {e}")))?
        {
            let doc = self
                .workspace
                .read(config_path)
                .await
                .map_err(|e| DatabaseError::Query(format!("workspace read failed: {e}")))?;
            if !system_config_metadata_matches(&doc.metadata, &expected) {
                debug!("repairing .system/.config metadata to expected system defaults");
                self.workspace
                    .update_metadata(doc.id, &expected)
                    .await
                    .map_err(|e| {
                        DatabaseError::Query(format!("workspace metadata repair failed: {e}"))
                    })?;
            }
            return Ok(());
        }

        let doc = self
            .workspace
            .write(config_path, &expected.to_string())
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace write failed: {e}")))?;

        // The .config doc's metadata column is the inheritance source for
        // descendants. Mirror the JSON content so future readers see the
        // same values via either path. `skip_versioning: false` is critical:
        // changing it to `true` here would silently disable versioning for
        // every document under `.system/**`.
        self.workspace
            .update_metadata(doc.id, &expected)
            .await
            .map_err(|e| DatabaseError::Query(format!("workspace metadata update failed: {e}")))?;

        debug!("seeded .system/.config for workspace settings");
        Ok(())
    }

    /// Lazy idempotent wrapper around `ensure_system_config` used by write
    /// paths so callers don't have to remember to seed at startup. After the
    /// first successful call this becomes a cheap atomic load.
    async fn ensure_system_config_lazy(&self) -> Result<(), DatabaseError> {
        // We can't return a borrow of `()` and propagate errors cleanly with
        // OnceCell::get_or_try_init because the closure must be `'static`,
        // so we manually check `get()` first.
        if self.system_config_seeded.get().is_some() {
            return Ok(());
        }
        self.ensure_system_config().await?;
        // Ignore the SetError if another task raced us — both calls succeeded.
        let _ = self.system_config_seeded.set(());
        Ok(())
    }

    /// Write a setting to workspace with optional schema in metadata.
    async fn write_to_workspace(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        // Lazily seed `.system/.config` so the first setting write cannot
        // create `.system/settings/**` documents before the inherited
        // `skip_indexing` / hygiene flags are in place. Cheap after the
        // first call (atomic OnceCell load).
        self.ensure_system_config_lazy().await?;
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
        // We don't propagate workspace delete errors: the legacy table is the
        // source of truth during migration, so a stale workspace doc on a
        // failed delete is recoverable on next write. We do log the failure
        // so partial-delete state is observable.
        if validate_settings_key(key).is_ok() {
            let path = settings_path(key);
            if let Err(e) = self.workspace.delete(&path).await {
                // `debug!` not `warn!`: settings writes are reachable from
                // REPL/CLI channels where `warn!`/`info!` output corrupts
                // the terminal UI (CLAUDE.md → Code Style → logging). The
                // legacy table remains the source of truth during
                // migration, so a stale workspace doc is recoverable on
                // the next write.
                debug!(
                    key = %key,
                    error = %e,
                    "workspace delete failed in delete_setting; legacy table will still be updated"
                );
            }
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

    /// Regression for review comment #3043199991: a caller that forgets to
    /// run `ensure_system_config()` at startup must still get a properly
    /// configured `.system/.config` after the first `set_setting` write.
    #[tokio::test]
    async fn set_setting_lazily_seeds_system_config() {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("settings_lazy_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));

        let adapter = WorkspaceSettingsAdapter::new(Arc::clone(&ws), db);
        // Deliberately do NOT call ensure_system_config() here.
        adapter
            .set_setting("test_user", "llm_backend", &serde_json::json!("anthropic"))
            .await
            .unwrap();

        // .system/.config must now exist with the expected metadata.
        let cfg = ws.read(".system/.config").await.unwrap();
        assert!(system_config_metadata_matches(
            &cfg.metadata,
            &serde_json::Value::Null
        ));
    }

    /// Regression for review comment #3043199972: if `.system/.config`
    /// already exists with broken metadata (e.g., from an older adapter
    /// that set `skip_versioning: true`), `ensure_system_config()` must
    /// repair it instead of silently leaving it broken.
    #[tokio::test]
    async fn ensure_system_config_repairs_existing_metadata() {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("settings_repair_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));

        // Simulate an old workspace where .system/.config exists but its
        // metadata column has skip_versioning: true (the pre-fix bug).
        let doc = ws.write(".system/.config", "{}").await.unwrap();
        ws.update_metadata(
            doc.id,
            &serde_json::json!({
                "skip_indexing": true,
                "skip_versioning": true,
                "hygiene": { "enabled": false }
            }),
        )
        .await
        .unwrap();

        let adapter = WorkspaceSettingsAdapter::new(Arc::clone(&ws), db);
        adapter.ensure_system_config().await.unwrap();

        // After ensure, the metadata must no longer disable versioning.
        let cfg = ws.read(".system/.config").await.unwrap();
        assert!(system_config_metadata_matches(
            &cfg.metadata,
            &serde_json::Value::Null
        ));
    }
}
