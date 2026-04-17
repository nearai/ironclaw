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
use crate::error::{DatabaseError, WorkspaceError};
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
///
/// ## Multi-tenant scoping
///
/// `Workspace` is scoped to a single `user_id` at construction time, so any
/// workspace read/write goes to *that* user's documents — independent of the
/// `user_id` argument passed to a `SettingsStore` method. Without gating, a
/// dual-write triggered for `user_B` would actually land in the
/// **owner's** workspace, and a subsequent `user_A.get_setting(...)` would
/// observe `user_B`'s value: a cross-user data leak.
///
/// To prevent that, the adapter is constructed with the workspace's owner
/// `user_id` (`gate_user_id`) and **only** dual-writes / reads from
/// workspace when the calling `user_id` matches. All other users fall
/// through to the legacy `settings` table only — preserving the pre-#2049
/// behavior they always had. This matches the long-term plan: per-user
/// settings live in legacy until a per-user `WorkspaceSettingsAdapter`
/// (one per `WorkspacePool` entry) is wired up; admin/global settings go
/// through the workspace-backed path so they pick up schema validation.
pub struct WorkspaceSettingsAdapter {
    workspace: Arc<Workspace>,
    legacy_store: Arc<dyn Database>,
    /// Identity allowed to use the workspace-backed dual-write path. Set to
    /// the workspace's owner `user_id` at construction. Any other caller
    /// goes legacy-only — see the rationale on the struct doc above.
    gate_user_id: String,
    /// Guards the lazy `ensure_system_config()` call so it runs at most once
    /// per adapter instance regardless of which write path triggers it. This
    /// removes the requirement that callers run `ensure_system_config()` at
    /// startup before any setting write — see the comment on `set_setting`.
    system_config_seeded: OnceCell<()>,
}

impl WorkspaceSettingsAdapter {
    pub fn new(workspace: Arc<Workspace>, legacy_store: Arc<dyn Database>) -> Self {
        let gate_user_id = workspace.user_id().to_string();
        Self {
            workspace,
            legacy_store,
            gate_user_id,
            system_config_seeded: OnceCell::new(),
        }
    }

    /// Returns true if the calling `user_id` is allowed to use the
    /// workspace-backed dual-write/read path. False callers fall through to
    /// the legacy table only.
    fn workspace_allowed_for(&self, user_id: &str) -> bool {
        user_id == self.gate_user_id
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
                // Also rewrite the human-readable content so it stays in sync
                // with the metadata. The metadata column is the inheritance
                // source of truth, but having the doc's content silently
                // diverge confuses anyone reading the doc directly to
                // understand which inherited flags are active.
                self.workspace
                    .write(config_path, &expected.to_string())
                    .await
                    .map_err(|e| {
                        DatabaseError::Query(format!("workspace content repair failed: {e}"))
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
    ///
    /// Uses `OnceCell::get_or_try_init` so two concurrent first-callers do
    /// not both run `ensure_system_config()`. The previous manual
    /// `get()`/`set()` pattern was functionally correct (idempotent) but
    /// wasteful under concurrent first-access; the OnceCell variant
    /// guarantees single execution.
    async fn ensure_system_config_lazy(&self) -> Result<(), DatabaseError> {
        self.system_config_seeded
            .get_or_try_init(|| async { self.ensure_system_config().await })
            .await?;
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
        validate_settings_key(key).map_err(|e| match e {
            WorkspaceError::InvalidPath { reason, .. } => {
                DatabaseError::Query(format!("invalid settings key '{key}': {reason}"))
            }
            other => DatabaseError::Query(format!("invalid settings key '{key}': {other}")),
        })?;
        let path = settings_path(key);
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        // Resolve the registered schema once and reuse for both pre-write
        // validation and post-write metadata persistence. Calling
        // `schema_for_key` twice would do duplicate work and (in the unlikely
        // event the registry ever became non-deterministic) could let the
        // validated schema diverge from the persisted one.
        let schema = schema_for_key(key);

        // Validate against the known schema BEFORE the first write so the
        // initial document creation cannot bypass schema enforcement. Once
        // metadata is set below, subsequent writes are validated by the
        // workspace itself via the resolved metadata chain.
        if let Some(schema) = schema.as_ref() {
            crate::workspace::schema::validate_content_against_schema(&path, &content, schema)
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
        if let Some(schema) = schema {
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
        // Owner-only workspace path: non-owner callers go straight to the
        // legacy table to avoid reading from the wrong user's workspace.
        if self.workspace_allowed_for(user_id)
            && let Some(value) = self.read_from_workspace(key).await?
        {
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
        // Owner-only workspace path. Non-owner callers, and invalid keys
        // (which can never exist in workspace and must not be used to
        // construct paths), go straight to the legacy table.
        if self.workspace_allowed_for(user_id) && validate_settings_key(key).is_ok() {
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
        // Dual-write order: legacy first, workspace second. The legacy
        // table is the source-of-truth during migration (it backs the
        // aggregate `list_settings` / `get_all_settings` reads), so writing
        // it first guarantees those readers always see a consistent value
        // even if the workspace write subsequently fails. A failed
        // workspace write becomes self-healing on the next read-miss
        // because per-key reads check the workspace first and fall back
        // to legacy.
        //
        // Workspace writes are also gated to the owner — see the struct
        // doc on `WorkspaceSettingsAdapter`.
        self.legacy_store.set_setting(user_id, key, value).await?;
        if self.workspace_allowed_for(user_id) {
            self.write_to_workspace(key, value).await?;
        }
        Ok(())
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        // Workspace delete is owner-gated — non-owner callers never wrote
        // to the workspace, so there's nothing to delete from it. We also
        // skip workspace for invalid keys (cannot exist there). Workspace
        // delete failures are logged but not propagated: the legacy table
        // is the source of truth during migration, so a stale workspace
        // doc is recoverable on next write.
        if self.workspace_allowed_for(user_id) && validate_settings_key(key).is_ok() {
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
        // Legacy first (source of truth, drives aggregate reads), then
        // workspace dual-write — same ordering as `set_setting`. Workspace
        // writes are skipped entirely for non-owner callers to prevent the
        // cross-tenant data leak (see struct doc).
        self.legacy_store
            .set_all_settings(user_id, settings)
            .await?;

        if !self.workspace_allowed_for(user_id) {
            return Ok(());
        }

        // Owner path: dual-write each setting to workspace. Collect the
        // first error so partial-migration state is observable, but never
        // mask the (already-successful) legacy write — failures here are
        // self-healing on the next per-key read.
        let mut workspace_error: Option<DatabaseError> = None;
        for (key, value) in settings {
            if let Err(e) = self.write_to_workspace(key, value).await {
                debug!(key = %key, error = %e, "workspace write failed in set_all_settings");
                if workspace_error.is_none() {
                    workspace_error = Some(e);
                }
            }
        }

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

    /// Regression for the multi-tenant cross-user data leak: a non-owner
    /// caller's `set_setting` must NOT touch the owner's workspace, and a
    /// non-owner `get_setting` must NOT see the owner's workspace value.
    /// All non-owner I/O must round-trip through the legacy table only.
    #[tokio::test]
    async fn workspace_settings_are_owner_gated_in_multi_tenant_mode() {
        use crate::db::libsql::LibSqlBackend;
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("settings_gating.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        <LibSqlBackend as Database>::run_migrations(&backend)
            .await
            .expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        // Workspace is constructed for the OWNER (mirroring how AppBuilder
        // wires it from `config.owner_id`). The adapter therefore gates
        // workspace reads/writes to caller `user_id == "owner"`.
        let owner_ws = Arc::new(Workspace::new_with_db("owner", Arc::clone(&db)));
        let adapter = WorkspaceSettingsAdapter::new(Arc::clone(&owner_ws), Arc::clone(&db));
        adapter.ensure_system_config().await.unwrap();

        // Owner writes a setting through the dual-write path.
        adapter
            .set_setting("owner", "llm_backend", &serde_json::json!("anthropic"))
            .await
            .expect("owner write");

        // Non-owner ("alice") writes a setting with the same key but a
        // different value. This must NOT touch the owner's workspace.
        adapter
            .set_setting("alice", "llm_backend", &serde_json::json!("openai"))
            .await
            .expect("alice write");

        // The OWNER's workspace doc must still hold the owner's value —
        // alice's write must have gone to legacy only.
        let owner_ws_doc = owner_ws
            .read(".system/settings/llm_backend.json")
            .await
            .expect("owner workspace doc still readable");
        let owner_ws_value: serde_json::Value =
            serde_json::from_str(&owner_ws_doc.content).expect("parse owner ws value");
        assert_eq!(
            owner_ws_value,
            serde_json::json!("anthropic"),
            "owner's workspace doc must NOT have been overwritten by alice's write"
        );

        // The legacy table is per-user, so each user reads back their own
        // value.
        let alice_value = adapter
            .get_setting("alice", "llm_backend")
            .await
            .expect("alice read")
            .expect("alice setting present in legacy");
        assert_eq!(alice_value, serde_json::json!("openai"));
        let owner_value = adapter
            .get_setting("owner", "llm_backend")
            .await
            .expect("owner read")
            .expect("owner setting present");
        assert_eq!(owner_value, serde_json::json!("anthropic"));

        // Critical: alice's `get_setting` must NEVER see the owner's
        // workspace value when only legacy holds alice's value. We verify
        // by deleting alice's legacy entry directly via the underlying
        // store and asserting alice now sees `None` — not the owner's
        // workspace value bleeding through.
        db.delete_setting("alice", "llm_backend")
            .await
            .expect("clear alice legacy");
        let alice_after_delete = adapter
            .get_setting("alice", "llm_backend")
            .await
            .expect("alice read after delete");
        assert!(
            alice_after_delete.is_none(),
            "alice must NOT see the owner's workspace value through the gate \
             (cross-user data leak); got {alice_after_delete:?}"
        );
    }
}
