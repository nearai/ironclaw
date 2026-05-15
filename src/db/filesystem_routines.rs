//! Filesystem-backed [`RoutineStore`] over the universal `RootFilesystem`.
//!
//! Routes routine persistence through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so routines
//! and routine runs share the same dispatch fabric as the rest of the
//! kernel-storage rework.
//!
//! Path layout (all absolute, validated by
//! [`VirtualPath`](ironclaw_host_api::VirtualPath)):
//!
//! - `/routines/<routine_id>` — one record per routine
//! - `/routines/<routine_id>/runs/<run_id>` — one record per run
//!
//! Indexed projections (see [`Entry::indexed`]):
//!
//! - On routine entries: `user_id`, `kind` (trigger type tag), `cron_schedule`
//!   (for cron triggers), `due_at` (epoch seconds — only set when both enabled
//!   and `next_fire_at` is present so `list_due_cron_routines` can range-scan).
//! - On run entries: `routine_id`, `status`, `job_id` (when linked).
//!
//! Trait-mandated query shapes (`list_event_routines`, `list_due_cron_routines`,
//! `get_webhook_routine_by_path`) are implemented as in-memory filters over
//! the materialised list. The indexed projection above leaves room for a
//! later optimisation that pushes these through
//! [`RootFilesystem::query`] without changing the trait surface.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, IndexKey, IndexValue, RootFilesystem,
};
use ironclaw_host_api::{HostApiError, VirtualPath};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::routine::{Routine, RoutineRun, RunStatus, Trigger};
use crate::db::RoutineStore;
use crate::error::DatabaseError;

/// Wire shape for `/routines/<id>`. Carries the full [`Routine`] payload —
/// the on-disk format is owned by this module and intentionally decoupled
/// from any internal `Routine` API additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRoutine {
    routine: Routine,
}

/// Wire shape for `/routines/<id>/runs/<run_id>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRoutineRun {
    run: RoutineRun,
}

/// Filesystem-backed [`RoutineStore`].
pub struct FilesystemRoutineStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemRoutineStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    async fn read_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        let path = routine_path(id)?;
        let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
            return Ok(None);
        };
        let stored: StoredRoutine = decode_json(&versioned.entry.body)?;
        Ok(Some(stored.routine))
    }

    async fn write_routine(
        &self,
        routine: &Routine,
        cas: CasExpectation,
    ) -> Result<(), DatabaseError> {
        let path = routine_path(routine.id)?;
        let body = encode_json(&StoredRoutine {
            routine: routine.clone(),
        })?;
        let entry = routine_entry(routine, body);
        self.filesystem
            .put(&path, entry, cas)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn list_routines_inner(&self) -> Result<Vec<Routine>, DatabaseError> {
        let root = routines_root()?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut routines = Vec::new();
        for dir_entry in entries {
            if !is_routine_leaf(&dir_entry.path) {
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
            let stored: StoredRoutine = decode_json(&versioned.entry.body)?;
            routines.push(stored.routine);
        }
        Ok(routines)
    }

    async fn write_run(&self, run: &RoutineRun, cas: CasExpectation) -> Result<(), DatabaseError> {
        let path = routine_run_path(run.routine_id, run.id)?;
        let body = encode_json(&StoredRoutineRun { run: run.clone() })?;
        let entry = routine_run_entry(run, body);
        self.filesystem
            .put(&path, entry, cas)
            .await
            .map(|_| ())
            .map_err(fs_to_db_error)
    }

    async fn list_runs_for_routine(
        &self,
        routine_id: Uuid,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let root = routine_runs_root(routine_id)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_db_error(error)),
        };
        let mut runs = Vec::new();
        for dir_entry in entries {
            let Some(versioned) = self
                .filesystem
                .get(&dir_entry.path)
                .await
                .map_err(fs_to_db_error)?
            else {
                continue;
            };
            let stored: StoredRoutineRun = decode_json(&versioned.entry.body)?;
            runs.push(stored.run);
        }
        Ok(runs)
    }

    async fn list_all_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        let routines = self.list_routines_inner().await?;
        let mut runs = Vec::new();
        for routine in routines {
            runs.extend(self.list_runs_for_routine(routine.id).await?);
        }
        Ok(runs)
    }
}

#[async_trait]
impl<F> RoutineStore for FilesystemRoutineStore<F>
where
    F: RootFilesystem,
{
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        // INSERT semantics: refuse if already present.
        self.write_routine(routine, CasExpectation::Absent).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.read_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        Ok(self
            .list_routines_inner()
            .await?
            .into_iter()
            .find(|routine| routine.user_id == user_id && routine.name == name))
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        let mut routines: Vec<Routine> = self
            .list_routines_inner()
            .await?
            .into_iter()
            .filter(|routine| routine.user_id == user_id)
            .collect();
        routines.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(routines)
    }

    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let mut routines = self.list_routines_inner().await?;
        routines.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(routines)
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        Ok(self
            .list_routines_inner()
            .await?
            .into_iter()
            .filter(|routine| {
                routine.enabled
                    && matches!(
                        routine.trigger,
                        Trigger::Event { .. } | Trigger::SystemEvent { .. }
                    )
            })
            .collect())
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        let now = Utc::now();
        Ok(self
            .list_routines_inner()
            .await?
            .into_iter()
            .filter(|routine| {
                routine.enabled
                    && matches!(routine.trigger, Trigger::Cron { .. })
                    && routine.next_fire_at.is_some_and(|next| next <= now)
            })
            .collect())
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        // The legacy SQL implementation refreshes `updated_at` on every UPDATE
        // but trusts the caller to supply the other fields verbatim. Mirror
        // that contract: `updated_at` is forced to "now" before write so
        // callers don't need to remember to bump it.
        let mut updated = routine.clone();
        updated.updated_at = Utc::now();
        self.write_routine(&updated, CasExpectation::Any).await
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        loop {
            let path = routine_path(id)?;
            let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
                return Err(DatabaseError::NotFound {
                    entity: "routine".to_string(),
                    id: id.to_string(),
                });
            };
            let mut stored: StoredRoutine = decode_json(&versioned.entry.body)?;
            stored.routine.last_run_at = Some(last_run_at);
            stored.routine.next_fire_at = next_fire_at;
            stored.routine.run_count = run_count;
            stored.routine.consecutive_failures = consecutive_failures;
            stored.routine.state = state.clone();
            stored.routine.updated_at = Utc::now();
            let body = encode_json(&stored)?;
            let entry = routine_entry(&stored.routine, body);
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

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let path = routine_path(id)?;
        if self
            .filesystem
            .get(&path)
            .await
            .map_err(fs_to_db_error)?
            .is_none()
        {
            return Ok(false);
        }
        match self.filesystem.delete(&path).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(fs_to_db_error(error)),
        }
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.write_run(run, CasExpectation::Absent).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        // The legacy schema indexed runs only by their own id; we have to
        // find which routine owns this run. Scan and update via CAS.
        let routines = self.list_routines_inner().await?;
        for routine in routines {
            let path = routine_run_path(routine.id, id)?;
            let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
                continue;
            };
            let mut stored: StoredRoutineRun = decode_json(&versioned.entry.body)?;
            stored.run.status = status;
            stored.run.completed_at = Some(Utc::now());
            stored.run.result_summary = result_summary.map(str::to_string);
            stored.run.tokens_used = tokens_used;
            let body = encode_json(&stored)?;
            let entry = routine_run_entry(&stored.run, body);
            self.filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
                .map_err(fs_to_db_error)?;
            return Ok(());
        }
        Err(DatabaseError::NotFound {
            entity: "routine_run".to_string(),
            id: id.to_string(),
        })
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        let mut runs = self.list_runs_for_routine(routine_id).await?;
        runs.sort_by_key(|right| std::cmp::Reverse(right.started_at));
        if limit >= 0 {
            runs.truncate(limit as usize);
        }
        Ok(runs)
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        Ok(self
            .list_runs_for_routine(routine_id)
            .await?
            .into_iter()
            .filter(|run| run.status == RunStatus::Running)
            .count() as i64)
    }

    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError> {
        if routine_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let requested: HashSet<Uuid> = routine_ids.iter().copied().collect();
        let mut counts: HashMap<Uuid, i64> =
            routine_ids.iter().copied().map(|id| (id, 0)).collect();
        let runs = self.list_all_runs().await?;
        for run in runs {
            if run.status == RunStatus::Running && requested.contains(&run.routine_id) {
                *counts.entry(run.routine_id).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError> {
        if routine_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let requested: HashSet<Uuid> = routine_ids.iter().copied().collect();
        let mut latest: HashMap<Uuid, RoutineRun> = HashMap::new();
        for routine_id in &requested {
            for run in self.list_runs_for_routine(*routine_id).await? {
                latest
                    .entry(run.routine_id)
                    .and_modify(|existing| {
                        if run.started_at > existing.started_at {
                            *existing = run.clone();
                        }
                    })
                    .or_insert(run);
            }
        }
        Ok(latest
            .into_iter()
            .map(|(routine_id, run)| (routine_id, run.status))
            .collect())
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        // Scan to find owning routine; CAS to update.
        let routines = self.list_routines_inner().await?;
        for routine in routines {
            let path = routine_run_path(routine.id, run_id)?;
            let Some(versioned) = self.filesystem.get(&path).await.map_err(fs_to_db_error)? else {
                continue;
            };
            let mut stored: StoredRoutineRun = decode_json(&versioned.entry.body)?;
            stored.run.job_id = Some(job_id);
            let body = encode_json(&stored)?;
            let entry = routine_run_entry(&stored.run, body);
            self.filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
                .map_err(fs_to_db_error)?;
            return Ok(());
        }
        Err(DatabaseError::NotFound {
            entity: "routine_run".to_string(),
            id: run_id.to_string(),
        })
    }

    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError> {
        let routines = self.list_routines_inner().await?;
        for routine in routines {
            if !routine.enabled {
                continue;
            }
            let Trigger::Webhook {
                path: ref hook_path,
                ..
            } = routine.trigger
            else {
                continue;
            };
            if let Some(uid) = user_id
                && routine.user_id != uid
            {
                continue;
            }
            let matches = match hook_path {
                Some(p) => p == path,
                None => routine.id.to_string() == path,
            };
            if matches {
                return Ok(Some(routine));
            }
        }
        Ok(None)
    }

    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        Ok(self
            .list_all_runs()
            .await?
            .into_iter()
            .filter(|run| run.status == RunStatus::Running && run.job_id.is_some())
            .collect())
    }
}

// -- Paths ------------------------------------------------------------------

fn routines_root() -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new("/routines").map_err(host_api_to_db_error)
}

fn routine_path(id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("/routines/{id}")).map_err(host_api_to_db_error)
}

fn routine_runs_root(routine_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("/routines/{routine_id}/runs")).map_err(host_api_to_db_error)
}

fn routine_run_path(routine_id: Uuid, run_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("/routines/{routine_id}/runs/{run_id}")).map_err(host_api_to_db_error)
}

fn is_routine_leaf(path: &VirtualPath) -> bool {
    let trimmed = path.as_str().strip_prefix("/routines/").unwrap_or("");
    !trimmed.is_empty() && !trimmed.contains('/')
}

// -- Indexed projections ----------------------------------------------------

fn routine_entry(routine: &Routine, body: Vec<u8>) -> Entry {
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    if let Ok(key) = IndexKey::new("user_id") {
        entry
            .indexed
            .insert(key, IndexValue::Text(routine.user_id.clone()));
    }
    if let Ok(key) = IndexKey::new("kind") {
        entry.indexed.insert(
            key,
            IndexValue::Text(routine.trigger.type_tag().to_string()),
        );
    }
    if let Trigger::Cron { schedule, .. } = &routine.trigger
        && let Ok(key) = IndexKey::new("cron_schedule")
    {
        entry
            .indexed
            .insert(key, IndexValue::Text(schedule.clone()));
    }
    if routine.enabled
        && let Some(next) = routine.next_fire_at
        && let Ok(key) = IndexKey::new("due_at")
    {
        entry.indexed.insert(key, IndexValue::I64(next.timestamp()));
    }
    entry
}

fn routine_run_entry(run: &RoutineRun, body: Vec<u8>) -> Entry {
    let mut entry = Entry::bytes(body).with_content_type(ContentType::json());
    if let Ok(key) = IndexKey::new("routine_id") {
        entry
            .indexed
            .insert(key, IndexValue::Text(run.routine_id.to_string()));
    }
    if let Ok(key) = IndexKey::new("status") {
        entry
            .indexed
            .insert(key, IndexValue::Text(run.status.to_string()));
    }
    if let Some(job_id) = run.job_id
        && let Ok(key) = IndexKey::new("job_id")
    {
        entry
            .indexed
            .insert(key, IndexValue::Text(job_id.to_string()));
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
    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, Trigger,
    };
    use chrono::Duration;
    use ironclaw_filesystem::InMemoryBackend;

    fn sample_routine(user_id: &str, name: &str, trigger: Trigger) -> Routine {
        Routine {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "test".into(),
            user_id: user_id.into(),
            enabled: true,
            trigger,
            action: RoutineAction::Lightweight {
                prompt: "hi".into(),
                context_paths: Vec::new(),
                max_tokens: 64,
                use_tools: false,
                max_tool_rounds: 1,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_run(routine_id: Uuid, status: RunStatus, started_at: DateTime<Utc>) -> RoutineRun {
        RoutineRun {
            id: Uuid::new_v4(),
            routine_id,
            trigger_type: "manual".into(),
            trigger_detail: None,
            started_at,
            completed_at: None,
            status,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: started_at,
        }
    }

    #[tokio::test]
    async fn create_get_list_routine_round_trips() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let routine = sample_routine("user-a", "morning", Trigger::Manual);
        store.create_routine(&routine).await.unwrap();
        let fetched = store.get_routine(routine.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, routine.id);
        let by_name = store
            .get_routine_by_name("user-a", "morning")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_name.id, routine.id);
        let listed = store.list_routines("user-a").await.unwrap();
        assert_eq!(listed.len(), 1);
        let none_for_other = store.list_routines("user-b").await.unwrap();
        assert!(none_for_other.is_empty());
    }

    #[tokio::test]
    async fn list_event_and_due_cron_filter_correctly() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);

        let mut cron_due = sample_routine(
            "u",
            "cron_due",
            Trigger::Cron {
                schedule: "* * * * *".into(),
                timezone: None,
            },
        );
        cron_due.next_fire_at = Some(Utc::now() - Duration::seconds(10));

        let mut cron_future = sample_routine(
            "u",
            "cron_future",
            Trigger::Cron {
                schedule: "* * * * *".into(),
                timezone: None,
            },
        );
        cron_future.next_fire_at = Some(Utc::now() + Duration::seconds(3600));

        let event = sample_routine(
            "u",
            "event",
            Trigger::Event {
                channel: None,
                pattern: ".*".into(),
            },
        );

        store.create_routine(&cron_due).await.unwrap();
        store.create_routine(&cron_future).await.unwrap();
        store.create_routine(&event).await.unwrap();

        let due = store.list_due_cron_routines().await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, cron_due.id);

        let events = store.list_event_routines().await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);
    }

    #[tokio::test]
    async fn update_runtime_uses_cas_and_persists_state() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let routine = sample_routine("u", "r", Trigger::Manual);
        store.create_routine(&routine).await.unwrap();

        let last_run_at = Utc::now();
        let next_fire = last_run_at + Duration::seconds(60);
        store
            .update_routine_runtime(
                routine.id,
                last_run_at,
                Some(next_fire),
                3,
                1,
                &serde_json::json!({"k": "v"}),
            )
            .await
            .unwrap();
        let updated = store.get_routine(routine.id).await.unwrap().unwrap();
        assert_eq!(updated.run_count, 3);
        assert_eq!(updated.consecutive_failures, 1);
        assert_eq!(updated.state, serde_json::json!({"k": "v"}));
    }

    #[tokio::test]
    async fn run_lifecycle_create_complete_list_count() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let routine = sample_routine("u", "r", Trigger::Manual);
        store.create_routine(&routine).await.unwrap();

        let now = Utc::now();
        let run1 = sample_run(routine.id, RunStatus::Running, now);
        let run2 = sample_run(routine.id, RunStatus::Running, now + Duration::seconds(1));
        store.create_routine_run(&run1).await.unwrap();
        store.create_routine_run(&run2).await.unwrap();
        assert_eq!(
            store.count_running_routine_runs(routine.id).await.unwrap(),
            2
        );

        store
            .complete_routine_run(run1.id, RunStatus::Ok, Some("done"), Some(100))
            .await
            .unwrap();
        let listed = store.list_routine_runs(routine.id, 10).await.unwrap();
        assert_eq!(listed.len(), 2);
        // Most-recent-first: run2 first.
        assert_eq!(listed[0].id, run2.id);
        assert_eq!(listed[1].id, run1.id);
        assert_eq!(listed[1].status, RunStatus::Ok);
    }

    #[tokio::test]
    async fn batch_helpers_scope_to_requested_routines() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let r1 = sample_routine("u", "r1", Trigger::Manual);
        let r2 = sample_routine("u", "r2", Trigger::Manual);
        store.create_routine(&r1).await.unwrap();
        store.create_routine(&r2).await.unwrap();

        let now = Utc::now();
        store
            .create_routine_run(&sample_run(r1.id, RunStatus::Running, now))
            .await
            .unwrap();
        store
            .create_routine_run(&sample_run(
                r1.id,
                RunStatus::Ok,
                now + Duration::seconds(1),
            ))
            .await
            .unwrap();
        store
            .create_routine_run(&sample_run(r2.id, RunStatus::Failed, now))
            .await
            .unwrap();

        let counts = store
            .count_running_routine_runs_batch(&[r1.id, r2.id])
            .await
            .unwrap();
        assert_eq!(counts.get(&r1.id).copied(), Some(1));
        assert_eq!(counts.get(&r2.id).copied(), Some(0));

        let statuses = store
            .batch_get_last_run_status(&[r1.id, r2.id])
            .await
            .unwrap();
        assert_eq!(statuses.get(&r1.id), Some(&RunStatus::Ok));
        assert_eq!(statuses.get(&r2.id), Some(&RunStatus::Failed));
    }

    #[tokio::test]
    async fn link_run_to_job_and_list_dispatched() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let routine = sample_routine("u", "r", Trigger::Manual);
        store.create_routine(&routine).await.unwrap();
        let run = sample_run(routine.id, RunStatus::Running, Utc::now());
        store.create_routine_run(&run).await.unwrap();

        let job_id = Uuid::new_v4();
        store.link_routine_run_to_job(run.id, job_id).await.unwrap();

        let dispatched = store.list_dispatched_routine_runs().await.unwrap();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].job_id, Some(job_id));
    }

    #[tokio::test]
    async fn webhook_routine_matches_by_path_or_id_fallback() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);

        let with_path = sample_routine(
            "u",
            "hook-1",
            Trigger::Webhook {
                path: Some("custom".into()),
                secret: None,
            },
        );
        let by_id = sample_routine(
            "u",
            "hook-2",
            Trigger::Webhook {
                path: None,
                secret: None,
            },
        );
        store.create_routine(&with_path).await.unwrap();
        store.create_routine(&by_id).await.unwrap();

        let m1 = store
            .get_webhook_routine_by_path("custom", Some("u"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(m1.id, with_path.id);

        let m2 = store
            .get_webhook_routine_by_path(&by_id.id.to_string(), Some("u"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(m2.id, by_id.id);

        let none = store
            .get_webhook_routine_by_path("nope", Some("u"))
            .await
            .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn delete_routine_returns_true_then_false() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemRoutineStore::new(fs);
        let routine = sample_routine("u", "r", Trigger::Manual);
        store.create_routine(&routine).await.unwrap();
        assert!(store.delete_routine(routine.id).await.unwrap());
        assert!(!store.delete_routine(routine.id).await.unwrap());
    }
}
