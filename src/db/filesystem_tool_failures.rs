//! Filesystem-backed [`ToolFailureStore`] over the universal `RootFilesystem`.
//!
//! Routes the self-repair `tool_failures` table through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface.
//!
//! Path layout (all absolute, validated by
//! [`VirtualPath`](ironclaw_host_api::VirtualPath)):
//!
//! - `/tool_failures/<tool_name>` — one record per tool
//!
//! Indexed projections (see [`Entry::indexed`]):
//!
//! - `tool_name` — exact-equality lookup mirror of the SQL primary key.
//! - `error_count` — used by [`get_broken_tools`] to filter on threshold; the
//!   trait surface exposes it via the in-memory filter below.
//! - `repaired` — `true` once the tool was marked repaired so listing skips it.
//!
//! Compare-and-swap retry is used for the `record_tool_failure` upsert
//! (incrementing `error_count`) and the two simple counters.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RootFilesystem,
};
use ironclaw_host_api::{HostApiError, VirtualPath};
use serde::{Deserialize, Serialize};

use crate::agent::BrokenTool;
use crate::db::ToolFailureStore;
use crate::error::DatabaseError;

/// Wire shape for `/tool_failures/<tool_name>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredToolFailure {
    tool_name: String,
    error_message: Option<String>,
    error_count: u32,
    first_failure: DateTime<Utc>,
    last_failure: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_build_result: Option<serde_json::Value>,
    #[serde(default)]
    repair_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repaired_at: Option<DateTime<Utc>>,
}

/// Filesystem-backed [`ToolFailureStore`].
pub struct FilesystemToolFailureStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemToolFailureStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    async fn read_with_version(
        &self,
        tool_name: &str,
    ) -> Result<Option<(StoredToolFailure, ironclaw_filesystem::RecordVersion)>, DatabaseError>
    {
        let path = tool_failure_path(tool_name)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredToolFailure = decode_json(&versioned.entry.body)?;
        Ok(Some((stored, versioned.version)))
    }
}

#[async_trait]
impl<F> ToolFailureStore for FilesystemToolFailureStore<F>
where
    F: RootFilesystem,
{
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        // Upsert with CAS retry. INSERT on absent, UPDATE on present with
        // error_count++.
        loop {
            let path = tool_failure_path(tool_name)?;
            let now = Utc::now();
            match self.read_with_version(tool_name).await? {
                Some((mut stored, version)) => {
                    stored.error_message = Some(error_message.to_string());
                    stored.error_count = stored.error_count.saturating_add(1);
                    stored.last_failure = now;
                    let body = encode_json(&stored)?;
                    let entry = tool_failure_entry(&stored, body);
                    match self
                        .filesystem
                        .put(&path, entry, CasExpectation::Version(version))
                        .await
                    {
                        Ok(_) => return Ok(()),
                        Err(FilesystemError::VersionMismatch { .. }) => continue,
                        Err(error) => return Err(fs_to_db_error(error)),
                    }
                }
                None => {
                    let stored = StoredToolFailure {
                        tool_name: tool_name.to_string(),
                        error_message: Some(error_message.to_string()),
                        error_count: 1,
                        first_failure: now,
                        last_failure: now,
                        last_build_result: None,
                        repair_attempts: 0,
                        repaired_at: None,
                    };
                    let body = encode_json(&stored)?;
                    let entry = tool_failure_entry(&stored, body);
                    match self
                        .filesystem
                        .put(&path, entry, CasExpectation::Absent)
                        .await
                    {
                        Ok(_) => return Ok(()),
                        Err(FilesystemError::VersionMismatch { .. }) => continue,
                        Err(error) => return Err(fs_to_db_error(error)),
                    }
                }
            }
        }
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        let root = tool_failures_root()?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut broken = Vec::new();
        for dir_entry in entries {
            let Some(versioned) = self
                .filesystem
                .get(&dir_entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let stored: StoredToolFailure = decode_json(&versioned.entry.body)?;
            if stored.repaired_at.is_some() {
                continue;
            }
            if (stored.error_count as i32) < threshold {
                continue;
            }
            broken.push(BrokenTool {
                name: stored.tool_name,
                last_error: stored.error_message,
                failure_count: stored.error_count,
                first_failure: stored.first_failure,
                last_failure: stored.last_failure,
                last_build_result: stored.last_build_result,
                repair_attempts: stored.repair_attempts,
            });
        }
        // Match SQL ORDER BY error_count DESC.
        broken.sort_by(|left, right| right.failure_count.cmp(&left.failure_count));
        Ok(broken)
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        loop {
            let path = tool_failure_path(tool_name)?;
            let Some((mut stored, version)) = self.read_with_version(tool_name).await? else {
                // SQL UPDATE on missing row is a no-op; mirror that.
                return Ok(());
            };
            stored.repaired_at = Some(Utc::now());
            stored.error_count = 0;
            let body = encode_json(&stored)?;
            let entry = tool_failure_entry(&stored, body);
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_db_error(error)),
            }
        }
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        loop {
            let path = tool_failure_path(tool_name)?;
            let Some((mut stored, version)) = self.read_with_version(tool_name).await? else {
                // SQL UPDATE on missing row is a no-op; mirror that.
                return Ok(());
            };
            stored.repair_attempts = stored.repair_attempts.saturating_add(1);
            let body = encode_json(&stored)?;
            let entry = tool_failure_entry(&stored, body);
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_db_error(error)),
            }
        }
    }
}

// -- Paths ------------------------------------------------------------------

fn tool_failures_root() -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new("/tool_failures").map_err(host_api_to_db_error)
}

fn tool_failure_path(tool_name: &str) -> Result<VirtualPath, DatabaseError> {
    if tool_name.is_empty() {
        return Err(DatabaseError::Query(
            "tool name must not be empty".to_string(),
        ));
    }
    VirtualPath::new(format!("/tool_failures/{tool_name}")).map_err(host_api_to_db_error)
}

// -- Indexed projections ----------------------------------------------------

fn tool_failure_entry(stored: &StoredToolFailure, body: Vec<u8>) -> Entry {
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    if let Ok(key) = IndexKey::new("tool_name") {
        entry
            .indexed
            .insert(key, IndexValue::Text(stored.tool_name.clone()));
    }
    if let Ok(key) = IndexKey::new("error_count") {
        entry
            .indexed
            .insert(key, IndexValue::I64(i64::from(stored.error_count)));
    }
    if let Ok(key) = IndexKey::new("repaired") {
        entry
            .indexed
            .insert(key, IndexValue::Bool(stored.repaired_at.is_some()));
    }
    entry
}

// -- Helpers ----------------------------------------------------------------

fn encode_json<T>(value: &T) -> Result<Vec<u8>, DatabaseError>
where
    T: Serialize,
{
    serde_json::to_vec(value).map_err(|error| DatabaseError::Serialization(error.to_string()))
}

fn decode_json<T>(bytes: &[u8]) -> Result<T, DatabaseError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| DatabaseError::Serialization(error.to_string()))
}

fn fs_to_db_error(error: FilesystemError) -> DatabaseError {
    match error {
        FilesystemError::NotFound { .. } => DatabaseError::Query("filesystem entry missing".into()),
        FilesystemError::VersionMismatch { .. } => {
            DatabaseError::Query("filesystem version mismatch".into())
        }
        other => DatabaseError::Query(format!("filesystem error: {other}")),
    }
}

fn host_api_to_db_error(error: HostApiError) -> DatabaseError {
    DatabaseError::Query(format!("invalid filesystem path: {error}"))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    #[tokio::test]
    async fn record_failure_inserts_then_increments_count() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemToolFailureStore::new(fs);

        store
            .record_tool_failure("echo", "first oops")
            .await
            .unwrap();
        store
            .record_tool_failure("echo", "second oops")
            .await
            .unwrap();

        let broken = store.get_broken_tools(1).await.unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].name, "echo");
        assert_eq!(broken[0].failure_count, 2);
        assert_eq!(broken[0].last_error.as_deref(), Some("second oops"));
    }

    #[tokio::test]
    async fn get_broken_tools_filters_threshold_and_repaired() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemToolFailureStore::new(fs);

        for _ in 0..5 {
            store.record_tool_failure("flaky", "boom").await.unwrap();
        }
        store.record_tool_failure("rare", "boom").await.unwrap();

        let at_least_three = store.get_broken_tools(3).await.unwrap();
        assert_eq!(at_least_three.len(), 1);
        assert_eq!(at_least_three[0].name, "flaky");
        assert_eq!(at_least_three[0].failure_count, 5);

        store.mark_tool_repaired("flaky").await.unwrap();
        let after_repair = store.get_broken_tools(1).await.unwrap();
        assert_eq!(after_repair.iter().filter(|t| t.name == "flaky").count(), 0);
        assert_eq!(after_repair.iter().filter(|t| t.name == "rare").count(), 1);
    }

    #[tokio::test]
    async fn mark_repaired_zeros_count_and_is_idempotent_on_missing() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemToolFailureStore::new(fs);

        // No-op on a name that was never recorded — mirrors SQL UPDATE.
        store.mark_tool_repaired("never-seen").await.unwrap();
        store.increment_repair_attempts("never-seen").await.unwrap();
        // Still empty:
        let broken = store.get_broken_tools(1).await.unwrap();
        assert!(broken.is_empty());

        store.record_tool_failure("tool", "kaboom").await.unwrap();
        store.increment_repair_attempts("tool").await.unwrap();
        store.increment_repair_attempts("tool").await.unwrap();

        // After 2 repair attempts the failure record is still listed until repaired:
        let pre = store.get_broken_tools(1).await.unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0].repair_attempts, 2);

        store.mark_tool_repaired("tool").await.unwrap();
        let post = store.get_broken_tools(1).await.unwrap();
        assert!(post.iter().all(|t| t.name != "tool"));
    }

    #[tokio::test]
    async fn get_broken_tools_orders_by_failure_count_desc() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemToolFailureStore::new(fs);
        for _ in 0..2 {
            store.record_tool_failure("a", "x").await.unwrap();
        }
        for _ in 0..5 {
            store.record_tool_failure("b", "x").await.unwrap();
        }
        for _ in 0..3 {
            store.record_tool_failure("c", "x").await.unwrap();
        }
        let broken = store.get_broken_tools(1).await.unwrap();
        let names: Vec<&str> = broken.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }
}
