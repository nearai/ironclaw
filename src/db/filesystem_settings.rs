//! Filesystem-backed implementation of [`SettingsStore`].
//!
//! Routes per-user key/value settings through the unified [`RootFilesystem`]
//! surface from `ironclaw_filesystem`. Mirrors the canonical migration shape
//! demonstrated in `crates/ironclaw_secrets/src/filesystem_store.rs` and
//! `crates/ironclaw_authorization/src/lib.rs`: one record per setting, scope
//! carried in both the virtual path and the indexed projection, CAS-driven
//! writes through `put` / `get` / `delete`.
//!
//! Path layout (under the `/system/settings` virtual root):
//!
//! - `/system/settings/<user_id>/<key>` — one [`Entry`] per setting.
//!
//! The indexed projection carries `user_id` and `key` so `query` can filter
//! within a user's settings tree without pulling every record body. The
//! filesystem backends translate `Filter::Eq` into native predicates
//! (`indexed->>'user_id' = $1` on Postgres, `json_extract(indexed,
//! '$.user_id') = $1` on libSQL).
//!
//! Encryption-at-rest is provided by mounting under an `EncryptedBackend`
//! decorator (forthcoming) — this store writes plaintext JSON.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RecordKind,
    RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use serde::{Deserialize, Serialize};

use crate::db::SettingsStore;
use crate::error::DatabaseError;
use crate::history::SettingRow;

const RECORD_KIND: &str = "user_setting";

mod fs_keys {
    pub const USER_ID: &str = "user_id";
    pub const KEY: &str = "key";
}

/// Filesystem-backed [`SettingsStore`].
///
/// Construct with any shared [`RootFilesystem`]. Use [`InMemoryBackend`] in
/// tests; production deployments mount one of the SQL or local backends.
pub struct FilesystemSettingsStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSetting {
    user_id: String,
    key: String,
    value: serde_json::Value,
    updated_at: chrono::DateTime<Utc>,
}

impl<F> FilesystemSettingsStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    fn setting_path(user_id: &str, key: &str) -> Result<VirtualPath, DatabaseError> {
        let user = encode_segment(user_id);
        let key_seg = encode_segment(key);
        VirtualPath::new(format!("/system/settings/{user}/{key_seg}"))
            .map_err(|e| DatabaseError::Query(format!("invalid settings path: {e}")))
    }

    fn settings_root(user_id: &str) -> Result<VirtualPath, DatabaseError> {
        let user = encode_segment(user_id);
        VirtualPath::new(format!("/system/settings/{user}"))
            .map_err(|e| DatabaseError::Query(format!("invalid settings root: {e}")))
    }

    fn build_entry(setting: &StoredSetting) -> Result<Entry, DatabaseError> {
        let body =
            serde_json::to_vec(setting).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let kind = RecordKind::new(RECORD_KIND).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let user_key =
            IndexKey::new(fs_keys::USER_ID).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let key_key =
            IndexKey::new(fs_keys::KEY).map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
        entry.kind = Some(kind);
        Ok(entry
            .with_indexed(user_key, IndexValue::Text(setting.user_id.clone()))
            .with_indexed(key_key, IndexValue::Text(setting.key.clone())))
    }

    async fn read_stored(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<StoredSetting>, DatabaseError> {
        let path = Self::setting_path(user_id, key)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredSetting = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        if stored.user_id != user_id || stored.key != key {
            return Ok(None);
        }
        Ok(Some(stored))
    }
}

#[async_trait]
impl<F> SettingsStore for FilesystemSettingsStore<F>
where
    F: RootFilesystem,
{
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        Ok(self
            .read_stored(user_id, key)
            .await?
            .map(|stored| stored.value))
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        Ok(self
            .read_stored(user_id, key)
            .await?
            .map(|stored| SettingRow {
                key: stored.key,
                value: stored.value,
                updated_at: stored.updated_at,
            }))
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let setting = StoredSetting {
            user_id: user_id.to_string(),
            key: key.to_string(),
            value: value.clone(),
            updated_at: Utc::now(),
        };
        let path = Self::setting_path(user_id, key)?;
        let entry = Self::build_entry(&setting)?;
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        let path = Self::setting_path(user_id, key)?;
        match self.filesystem.delete(&path).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        let stored = self.list_stored_for_user(user_id).await?;
        let mut rows: Vec<SettingRow> = stored
            .into_iter()
            .map(|s| SettingRow {
                key: s.key,
                value: s.value,
                updated_at: s.updated_at,
            })
            .collect();
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(rows)
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        let stored = self.list_stored_for_user(user_id).await?;
        Ok(stored.into_iter().map(|s| (s.key, s.value)).collect())
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        // We don't have multi-key transactions on the unified surface in the
        // CAS-only floor; iterate and write each setting. Failure midway
        // through leaves the partial set committed, mirroring what a SQL
        // ROLLBACK can't atomically prevent across separate filesystem
        // mounts. See ironclaw_filesystem CLAUDE.md invariant 2 (CAS floor).
        for (key, value) in settings {
            self.set_setting(user_id, key, value).await?;
        }
        Ok(())
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        let root = Self::settings_root(user_id)?;
        match self.filesystem.list_dir(&root).await {
            Ok(entries) => Ok(!entries.is_empty()),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }
}

impl<F> FilesystemSettingsStore<F>
where
    F: RootFilesystem,
{
    async fn list_stored_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<StoredSetting>, DatabaseError> {
        let root = match Self::settings_root(user_id) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let stored: StoredSetting = match serde_json::from_slice(&versioned.entry.body) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if stored.user_id != user_id {
                continue;
            }
            out.push(stored);
        }
        Ok(out)
    }
}

/// Encode an arbitrary string as a path segment. Replaces characters that
/// are unsafe in a virtual path component with `_`. This keeps `/` and
/// other separators out of user-supplied IDs and keys so a malicious value
/// can't traverse the namespace.
fn encode_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | '\n' | '\r' | '\t' | ' ' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn fs_to_db_error(error: FilesystemError) -> DatabaseError {
    DatabaseError::Query(format!("filesystem error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use serde_json::json;

    fn store() -> FilesystemSettingsStore<InMemoryBackend> {
        FilesystemSettingsStore::new(Arc::new(InMemoryBackend::new()))
    }

    #[tokio::test]
    async fn get_setting_returns_none_for_missing_key() {
        let store = store();
        assert!(
            store
                .get_setting("alice", "missing")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn set_then_get_round_trips_setting_value() {
        let store = store();
        store
            .set_setting("alice", "theme", &json!("dark"))
            .await
            .unwrap();
        assert_eq!(
            store.get_setting("alice", "theme").await.unwrap(),
            Some(json!("dark"))
        );
    }

    #[tokio::test]
    async fn set_setting_overwrites_previous_value() {
        let store = store();
        store
            .set_setting("alice", "theme", &json!("light"))
            .await
            .unwrap();
        store
            .set_setting("alice", "theme", &json!("dark"))
            .await
            .unwrap();
        assert_eq!(
            store.get_setting("alice", "theme").await.unwrap(),
            Some(json!("dark"))
        );
    }

    #[tokio::test]
    async fn delete_setting_returns_false_when_absent() {
        let store = store();
        assert!(!store.delete_setting("alice", "nope").await.unwrap());
    }

    #[tokio::test]
    async fn delete_setting_returns_true_and_clears_value() {
        let store = store();
        store.set_setting("alice", "x", &json!(1)).await.unwrap();
        assert!(store.delete_setting("alice", "x").await.unwrap());
        assert!(store.get_setting("alice", "x").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_settings_returns_settings_sorted_by_key() {
        let store = store();
        store.set_setting("alice", "z", &json!(1)).await.unwrap();
        store.set_setting("alice", "a", &json!(2)).await.unwrap();
        let rows = store.list_settings("alice").await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "a");
        assert_eq!(rows[1].key, "z");
    }

    #[tokio::test]
    async fn set_all_settings_writes_each_entry() {
        let store = store();
        let mut map = HashMap::new();
        map.insert("a".to_string(), json!(1));
        map.insert("b".to_string(), json!("two"));
        store.set_all_settings("alice", &map).await.unwrap();
        let fetched = store.get_all_settings("alice").await.unwrap();
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched.get("a"), Some(&json!(1)));
        assert_eq!(fetched.get("b"), Some(&json!("two")));
    }

    #[tokio::test]
    async fn has_settings_reflects_whether_user_has_any_settings() {
        let store = store();
        assert!(!store.has_settings("alice").await.unwrap());
        store.set_setting("alice", "k", &json!(1)).await.unwrap();
        assert!(store.has_settings("alice").await.unwrap());
    }

    #[tokio::test]
    async fn settings_isolate_users() {
        let store = store();
        store.set_setting("alice", "k", &json!("a")).await.unwrap();
        store.set_setting("bob", "k", &json!("b")).await.unwrap();
        assert_eq!(
            store.get_setting("alice", "k").await.unwrap(),
            Some(json!("a"))
        );
        assert_eq!(
            store.get_setting("bob", "k").await.unwrap(),
            Some(json!("b"))
        );
    }

    #[tokio::test]
    async fn encode_segment_replaces_path_separators() {
        // Keys containing `/` or `..` should not escape the user's settings
        // namespace.
        let store = store();
        store
            .set_setting("alice", "../secrets", &json!(1))
            .await
            .unwrap();
        // The dangerous-looking key still round-trips, but the path under
        // it sits in /settings/alice rather than escaping.
        assert_eq!(
            store.get_setting("alice", "../secrets").await.unwrap(),
            Some(json!(1))
        );
        assert!(
            store.has_settings("alice").await.unwrap(),
            "encoded segment must remain inside the user's settings root"
        );
    }
}
