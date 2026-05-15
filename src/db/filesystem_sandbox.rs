//! Filesystem-backed [`SandboxStore`] over the universal `RootFilesystem`.
//!
//! Routes sandbox job persistence through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so the
//! per-user sandbox job records and per-job event log share the same dispatch
//! fabric as the rest of the kernel-storage rework.
//!
//! Path layout (all absolute, validated by
//! [`VirtualPath`](ironclaw_host_api::VirtualPath)):
//!
//! - `/sandbox/jobs/<job_id>` — one record per sandbox job (encoded as an
//!   [`Entry::record`] with the `sandbox_job` schema kind)
//! - `/sandbox/jobs/<job_id>/events` — append-only event log for the job,
//!   accessed exclusively through the [`append`](RootFilesystem::append) /
//!   [`tail`](RootFilesystem::tail) event plane. The job event sequence id
//!   carried on [`JobEventRecord::id`] maps 1:1 to the backend
//!   [`SeqNo`](ironclaw_filesystem::SeqNo).
//!
//! Indexed projections (see [`Entry::indexed`]):
//!
//! - `user_id` — single-tenant queries (`list_sandbox_jobs_for_user`,
//!   `sandbox_job_summary_for_user`, `sandbox_job_belongs_to_user`).
//! - `status` — summary aggregation.
//! - `mode` — optional job_mode column equivalent.
//!
//! Compare-and-swap is used for all status transitions so concurrent writers
//! cannot lose a transition. The default [`InMemoryBackend`] tests exercise
//! this contract end-to-end.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RootFilesystem,
};
use ironclaw_host_api::{HostApiError, VirtualPath};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::SandboxStore;
use crate::error::DatabaseError;
use crate::history::{JobEventRecord, SandboxJobRecord, SandboxJobSummary};

/// Wire shape stored at `/sandbox/jobs/<id>`. Intentionally separate from
/// [`SandboxJobRecord`] so the on-disk format is owned by this module and
/// won't drift if the in-memory shape adds runtime-only fields later.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSandboxJob {
    id: Uuid,
    task: String,
    status: String,
    user_id: String,
    project_dir: String,
    success: Option<bool>,
    failure_reason: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    credential_grants_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
}

impl StoredSandboxJob {
    fn from_record(record: &SandboxJobRecord, mode: Option<String>) -> Self {
        Self {
            id: record.id,
            task: record.task.clone(),
            status: record.status.clone(),
            user_id: record.user_id.clone(),
            project_dir: record.project_dir.clone(),
            success: record.success,
            failure_reason: record.failure_reason.clone(),
            created_at: record.created_at,
            started_at: record.started_at,
            completed_at: record.completed_at,
            credential_grants_json: record.credential_grants_json.clone(),
            mcp_servers: record.mcp_servers.clone(),
            max_iterations: record.max_iterations,
            mode,
        }
    }

    fn to_record(&self) -> SandboxJobRecord {
        SandboxJobRecord {
            id: self.id,
            task: self.task.clone(),
            status: self.status.clone(),
            user_id: self.user_id.clone(),
            project_dir: self.project_dir.clone(),
            success: self.success,
            failure_reason: self.failure_reason.clone(),
            created_at: self.created_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
            credential_grants_json: self.credential_grants_json.clone(),
            mcp_servers: self.mcp_servers.clone(),
            max_iterations: self.max_iterations,
        }
    }
}

/// Wire shape for one persisted job event in the append/tail plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredJobEvent {
    event_type: String,
    data: serde_json::Value,
    created_at: DateTime<Utc>,
}

/// Filesystem-backed [`SandboxStore`].
pub struct FilesystemSandboxStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemSandboxStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    async fn read_job(&self, id: Uuid) -> Result<Option<StoredSandboxJob>, DatabaseError> {
        let path = sandbox_job_path(id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredSandboxJob = decode_json(&versioned.entry.body)?;
        Ok(Some(stored))
    }

    async fn write_job(
        &self,
        job: &StoredSandboxJob,
        cas: CasExpectation,
    ) -> Result<(), DatabaseError> {
        let path = sandbox_job_path(job.id)?;
        let body = encode_json(job)?;
        let entry = sandbox_job_entry(job, body);
        self.filesystem
            .put(&path, entry, cas)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn list_all(&self) -> Result<Vec<StoredSandboxJob>, DatabaseError> {
        let root = sandbox_jobs_root()?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut jobs = Vec::new();
        for dir_entry in entries {
            // The job record itself lives at the same path; descendant `events`
            // entries are filtered out by `is_file_entry` (events are stored
            // under a child path, not the job leaf).
            if !is_sandbox_job_leaf(&dir_entry.path) {
                continue;
            }
            let Some(versioned) = self
                .filesystem
                .get(&dir_entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let stored: StoredSandboxJob = decode_json(&versioned.entry.body)?;
            jobs.push(stored);
        }
        // Most-recent-first to match the legacy ORDER BY created_at DESC.
        jobs.sort_by_key(|right| std::cmp::Reverse(right.created_at));
        Ok(jobs)
    }
}

#[async_trait]
impl<F> SandboxStore for FilesystemSandboxStore<F>
where
    F: RootFilesystem,
{
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        // Preserve the existing job_mode (if any) on upsert — the legacy
        // libSQL ON CONFLICT clause keeps `job_mode` untouched. We mirror
        // that by reading the prior record before overwriting.
        let prior_mode = self.read_job(job.id).await?.and_then(|p| p.mode);
        let stored = StoredSandboxJob::from_record(job, prior_mode);
        self.write_job(&stored, CasExpectation::Any).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        Ok(self.read_job(id).await?.map(|stored| stored.to_record()))
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        Ok(self
            .list_all()
            .await?
            .into_iter()
            .map(|stored| stored.to_record())
            .collect())
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        // CAS retry loop — concurrent callers may race on the same job; the
        // filesystem `put` rejects a stale version and we re-read.
        loop {
            let path = sandbox_job_path(id)?;
            let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
                return Err(DatabaseError::NotFound {
                    entity: "sandbox_job".to_string(),
                    id: id.to_string(),
                });
            };
            let mut stored: StoredSandboxJob = decode_json(&versioned.entry.body)?;
            stored.status = status.to_string();
            if success.is_some() {
                stored.success = success;
            }
            if let Some(msg) = message {
                stored.failure_reason = Some(msg.to_string());
            }
            if let Some(ts) = started_at {
                stored.started_at = Some(ts);
            }
            if let Some(ts) = completed_at {
                stored.completed_at = Some(ts);
            }
            let body = encode_json(&stored)?;
            let entry = sandbox_job_entry(&stored, body);
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_db_error(error)),
            }
        }
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        let jobs = self.list_all().await?;
        let now = Utc::now();
        let mut count: u64 = 0;
        for stored in jobs {
            if stored.status != "running" && stored.status != "creating" {
                continue;
            }
            self.update_sandbox_job_status(
                stored.id,
                "interrupted",
                None,
                Some("Process restarted"),
                None,
                Some(now),
            )
            .await?;
            count += 1;
        }
        Ok(count)
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        let jobs = self.list_all().await?;
        Ok(summarize(jobs.iter().map(|stored| stored.status.as_str())))
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        Ok(self
            .list_all()
            .await?
            .into_iter()
            .filter(|stored| stored.user_id == user_id)
            .map(|stored| stored.to_record())
            .collect())
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        let jobs = self.list_all().await?;
        Ok(summarize(jobs.iter().filter_map(|stored| {
            if stored.user_id == user_id {
                Some(stored.status.as_str())
            } else {
                None
            }
        })))
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        Ok(self
            .read_job(job_id)
            .await?
            .is_some_and(|stored| stored.user_id == user_id))
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        loop {
            let path = sandbox_job_path(id)?;
            let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
                return Err(DatabaseError::NotFound {
                    entity: "sandbox_job".to_string(),
                    id: id.to_string(),
                });
            };
            let mut stored: StoredSandboxJob = decode_json(&versioned.entry.body)?;
            stored.mode = Some(mode.to_string());
            let body = encode_json(&stored)?;
            let entry = sandbox_job_entry(&stored, body);
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_db_error(error)),
            }
        }
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        Ok(self.read_job(id).await?.and_then(|stored| stored.mode))
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let path = sandbox_job_events_path(job_id)?;
        let stored = StoredJobEvent {
            event_type: event_type.to_string(),
            data: data.clone(),
            created_at: Utc::now(),
        };
        let payload = encode_json(&stored)?;
        self.filesystem
            .append(&path, payload)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        let path = sandbox_job_events_path(job_id)?;
        let records = match self
            .filesystem
            .tail(&path, ironclaw_filesystem::SeqNo::ZERO)
            .await
        {
            Ok(records) => records,
            Err(error) if is_not_found(&error) || is_unsupported(&error) => Vec::new(),
            Err(error) => return Err(fs_to_db_error(error)),
        };

        let mut events: Vec<JobEventRecord> = records
            .into_iter()
            .map(|record| {
                let stored: StoredJobEvent = decode_json(&record.payload)?;
                Ok(JobEventRecord {
                    id: record.seq.get() as i64,
                    job_id,
                    event_type: stored.event_type,
                    data: stored.data,
                    created_at: stored.created_at,
                })
            })
            .collect::<Result<Vec<_>, DatabaseError>>()?;

        // Legacy SQL behaviour: ORDER BY id ASC, with `LIMIT n` keeping the
        // newest `n` rows (achieved via inner `ORDER BY id DESC LIMIT n`,
        // then outer ASC). Mirror that semantics here.
        if let Some(n) = limit {
            let n = n.max(0) as usize;
            if events.len() > n {
                events.drain(0..events.len() - n);
            }
        }
        Ok(events)
    }
}

// -- Paths ------------------------------------------------------------------

fn sandbox_jobs_root() -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new("/sandbox/jobs").map_err(host_api_to_db_error)
}

fn sandbox_job_path(id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("/sandbox/jobs/{id}")).map_err(host_api_to_db_error)
}

fn sandbox_job_events_path(id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("/sandbox/jobs/{id}/events")).map_err(host_api_to_db_error)
}

fn is_sandbox_job_leaf(path: &VirtualPath) -> bool {
    // `/sandbox/jobs/<id>` has exactly 3 path components below root.
    // Children under `/events` carry an extra segment and are filtered out.
    let trimmed = path.as_str().strip_prefix("/sandbox/jobs/").unwrap_or("");
    !trimmed.is_empty() && !trimmed.contains('/')
}

// -- Helpers ----------------------------------------------------------------

fn sandbox_job_entry(stored: &StoredSandboxJob, body: Vec<u8>) -> Entry {
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    if let Ok(key) = IndexKey::new("user_id") {
        entry
            .indexed
            .insert(key, IndexValue::Text(stored.user_id.clone()));
    }
    if let Ok(key) = IndexKey::new("status") {
        entry
            .indexed
            .insert(key, IndexValue::Text(stored.status.clone()));
    }
    if let Some(mode) = &stored.mode
        && let Ok(key) = IndexKey::new("mode")
    {
        entry.indexed.insert(key, IndexValue::Text(mode.clone()));
    }
    entry
}

fn summarize<'a, I>(statuses: I) -> SandboxJobSummary
where
    I: Iterator<Item = &'a str>,
{
    let mut summary = SandboxJobSummary::default();
    for status in statuses {
        summary.total += 1;
        match status {
            "creating" => summary.creating += 1,
            "running" => summary.running += 1,
            "completed" => summary.completed += 1,
            "failed" => summary.failed += 1,
            "interrupted" => summary.interrupted += 1,
            _ => {}
        }
    }
    summary
}

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
        other => DatabaseError::Query(format!("filesystem error: {other}")),
    }
}

fn host_api_to_db_error(error: HostApiError) -> DatabaseError {
    DatabaseError::Query(format!("invalid filesystem path: {error}"))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

fn is_unsupported(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::Unsupported { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    fn sample_job(id: Uuid, user_id: &str, status: &str) -> SandboxJobRecord {
        SandboxJobRecord {
            id,
            task: "test task".into(),
            status: status.into(),
            user_id: user_id.into(),
            project_dir: "/projects/p1".into(),
            success: None,
            failure_reason: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            credential_grants_json: "[]".into(),
            mcp_servers: None,
            max_iterations: None,
        }
    }

    #[tokio::test]
    async fn save_and_get_sandbox_job_round_trips() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id = Uuid::new_v4();
        let job = sample_job(id, "user-a", "creating");
        store.save_sandbox_job(&job).await.unwrap();

        let fetched = store.get_sandbox_job(id).await.unwrap().unwrap();
        assert_eq!(fetched.id, id);
        assert_eq!(fetched.user_id, "user-a");
        assert_eq!(fetched.status, "creating");
    }

    #[tokio::test]
    async fn update_status_uses_cas_and_preserves_other_fields() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id = Uuid::new_v4();
        let job = sample_job(id, "user-a", "creating");
        store.save_sandbox_job(&job).await.unwrap();

        let completed = Utc::now();
        store
            .update_sandbox_job_status(id, "completed", Some(true), None, None, Some(completed))
            .await
            .unwrap();

        let fetched = store.get_sandbox_job(id).await.unwrap().unwrap();
        assert_eq!(fetched.status, "completed");
        assert_eq!(fetched.success, Some(true));
        assert_eq!(fetched.completed_at, Some(completed));
        assert_eq!(fetched.user_id, "user-a");
    }

    #[tokio::test]
    async fn list_and_summary_filter_by_user() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        store
            .save_sandbox_job(&sample_job(Uuid::new_v4(), "user-a", "running"))
            .await
            .unwrap();
        store
            .save_sandbox_job(&sample_job(Uuid::new_v4(), "user-a", "completed"))
            .await
            .unwrap();
        store
            .save_sandbox_job(&sample_job(Uuid::new_v4(), "user-b", "failed"))
            .await
            .unwrap();

        let user_a = store.list_sandbox_jobs_for_user("user-a").await.unwrap();
        assert_eq!(user_a.len(), 2);
        let summary_a = store.sandbox_job_summary_for_user("user-a").await.unwrap();
        assert_eq!(summary_a.total, 2);
        assert_eq!(summary_a.running, 1);
        assert_eq!(summary_a.completed, 1);

        let summary_all = store.sandbox_job_summary().await.unwrap();
        assert_eq!(summary_all.total, 3);
        assert_eq!(summary_all.failed, 1);
    }

    #[tokio::test]
    async fn belongs_to_user_matches_owner() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id = Uuid::new_v4();
        store
            .save_sandbox_job(&sample_job(id, "user-a", "running"))
            .await
            .unwrap();

        assert!(
            store
                .sandbox_job_belongs_to_user(id, "user-a")
                .await
                .unwrap()
        );
        assert!(
            !store
                .sandbox_job_belongs_to_user(id, "user-b")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn job_mode_round_trips_and_save_preserves_existing_mode() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id = Uuid::new_v4();
        store
            .save_sandbox_job(&sample_job(id, "user-a", "creating"))
            .await
            .unwrap();
        store.update_sandbox_job_mode(id, "plan").await.unwrap();
        assert_eq!(
            store.get_sandbox_job_mode(id).await.unwrap().as_deref(),
            Some("plan")
        );

        // Re-saving the job record must not wipe the mode (matches legacy
        // SQL ON CONFLICT semantics which only touched a subset of columns).
        store
            .save_sandbox_job(&sample_job(id, "user-a", "running"))
            .await
            .unwrap();
        assert_eq!(
            store.get_sandbox_job_mode(id).await.unwrap().as_deref(),
            Some("plan")
        );
    }

    #[tokio::test]
    async fn cleanup_marks_running_and_creating_as_interrupted() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id_run = Uuid::new_v4();
        let id_create = Uuid::new_v4();
        let id_done = Uuid::new_v4();
        store
            .save_sandbox_job(&sample_job(id_run, "u", "running"))
            .await
            .unwrap();
        store
            .save_sandbox_job(&sample_job(id_create, "u", "creating"))
            .await
            .unwrap();
        store
            .save_sandbox_job(&sample_job(id_done, "u", "completed"))
            .await
            .unwrap();

        let count = store.cleanup_stale_sandbox_jobs().await.unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            store.get_sandbox_job(id_run).await.unwrap().unwrap().status,
            "interrupted"
        );
        assert_eq!(
            store
                .get_sandbox_job(id_done)
                .await
                .unwrap()
                .unwrap()
                .status,
            "completed"
        );
    }

    #[tokio::test]
    async fn save_and_list_job_events_via_append_tail() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let id = Uuid::new_v4();
        store
            .save_sandbox_job(&sample_job(id, "user-a", "running"))
            .await
            .unwrap();

        store
            .save_job_event(id, "start", &serde_json::json!({"phase": "init"}))
            .await
            .unwrap();
        store
            .save_job_event(id, "tool", &serde_json::json!({"name": "echo"}))
            .await
            .unwrap();
        store
            .save_job_event(id, "end", &serde_json::json!({"ok": true}))
            .await
            .unwrap();

        let all = store.list_job_events(id, None).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].event_type, "start");
        assert_eq!(all[2].event_type, "end");

        // Monotonic seq ids
        assert!(all[0].id < all[1].id);
        assert!(all[1].id < all[2].id);

        let last_two = store.list_job_events(id, Some(2)).await.unwrap();
        assert_eq!(last_two.len(), 2);
        assert_eq!(last_two[0].event_type, "tool");
        assert_eq!(last_two[1].event_type, "end");
    }

    #[tokio::test]
    async fn list_events_for_unknown_job_returns_empty() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSandboxStore::new(fs);
        let events = store.list_job_events(Uuid::new_v4(), None).await.unwrap();
        assert!(events.is_empty());
    }
}
