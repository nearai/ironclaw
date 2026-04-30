//! Filesystem-backed process and process-result stores.
//!
//! Records are stored as JSON under the exact resource-owner path
//! `tenants/<tenant>/users/<user>[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/`,
//! split into:
//!
//! - `processes/<process_id>.json` — lifecycle records ([`FilesystemProcessStore`])
//! - `process-results/<process_id>.json` — terminal result metadata
//! - `process-outputs/<process_id>/output.json` — large/sensitive output bodies
//!
//! All path/serde helpers are private to this module since they are tied to
//! the on-disk layout above.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{ProcessId, ResourceScope, VirtualPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::types::{
    ProcessError, ProcessRecord, ProcessResultRecord, ProcessResultStore, ProcessStart,
    ProcessStatus, ProcessStore, ensure_status_transition, invalid_path, same_scope_owner,
};

pub(crate) enum FilesystemHandle<'a, F>
where
    F: RootFilesystem,
{
    Borrowed(&'a F),
    Shared(Arc<F>),
}

impl<F> FilesystemHandle<'_, F>
where
    F: RootFilesystem,
{
    fn as_ref(&self) -> &F {
        match self {
            Self::Borrowed(filesystem) => filesystem,
            Self::Shared(filesystem) => filesystem.as_ref(),
        }
    }
}

pub struct FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: FilesystemHandle<'a, F>,
    transition_lock: AsyncMutex<()>,
}

impl<'a, F> FilesystemProcessStore<'a, F>
where
    F: RootFilesystem,
{
    /// Construct a filesystem-backed process store.
    ///
    /// **Single-instance invariant**: the `transition_lock` only serializes
    /// `start` and `update_status` (i.e. `complete`/`fail`/`kill`) within a
    /// single `FilesystemProcessStore` instance. Operating multiple instances
    /// concurrently against the same on-disk root is unsupported and will
    /// race on the JSON record files. Construct the store once and share via
    /// `Arc` (see [`from_arc`](Self::from_arc)).
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem: FilesystemHandle::Borrowed(filesystem),
            transition_lock: AsyncMutex::new(()),
        }
    }

    /// Construct an owned (`'static`) variant from a shared filesystem handle.
    ///
    /// The same single-instance invariant from [`new`](Self::new) applies:
    /// share the resulting store via `Arc` rather than constructing multiple
    /// instances pointed at the same root.
    pub fn from_arc(filesystem: Arc<F>) -> FilesystemProcessStore<'static, F> {
        FilesystemProcessStore {
            filesystem: FilesystemHandle::Shared(filesystem),
            transition_lock: AsyncMutex::new(()),
        }
    }

    async fn write_record(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let path = process_record_path(&record.scope, record.process_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(())
    }

    async fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        to: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let mut record = self
            .get(scope, process_id)
            .await?
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        ensure_status_transition(process_id, record.status, to)?;
        record.status = to;
        record.error_kind = error_kind;
        self.write_record(&record).await?;
        Ok(record)
    }
}

#[async_trait]
impl<F> ProcessStore for FilesystemProcessStore<'_, F>
where
    F: RootFilesystem,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let path = process_record_path(&start.scope, start.process_id)?;
        let existing = match self.filesystem.as_ref().read_file(&path).await {
            Ok(_) => true,
            Err(error) if is_not_found(&error) => false,
            Err(error) => return Err(error.into()),
        };
        if existing {
            return Err(ProcessError::ProcessAlreadyExists {
                process_id: start.process_id,
            });
        }
        let record = ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        };
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
            .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Failed, Some(error_kind))
            .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        let path = process_record_path(scope, process_id)?;
        let bytes = match self.filesystem.as_ref().read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<ProcessRecord>(&bytes)?;
        ensure_process_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let root = process_records_root(scope)?;
        let entries = match self.filesystem.as_ref().list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if entry.name.ends_with(".json") {
                let bytes = self.filesystem.as_ref().read_file(&entry.path).await?;
                let record = deserialize::<ProcessRecord>(&bytes)?;
                if same_scope_owner(&record.scope, scope) {
                    records.push(record);
                }
            }
        }
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

pub struct FilesystemProcessResultStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: FilesystemHandle<'a, F>,
}

impl<'a, F> FilesystemProcessResultStore<'a, F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: &'a F) -> Self {
        Self {
            filesystem: FilesystemHandle::Borrowed(filesystem),
        }
    }

    pub fn from_arc(filesystem: Arc<F>) -> FilesystemProcessResultStore<'static, F> {
        FilesystemProcessResultStore {
            filesystem: FilesystemHandle::Shared(filesystem),
        }
    }

    async fn write_result(&self, record: &ProcessResultRecord) -> Result<(), ProcessError> {
        let path = process_result_path(&record.scope, record.process_id)?;
        let bytes = serialize_pretty(record)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(())
    }

    async fn write_output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: &Value,
    ) -> Result<VirtualPath, ProcessError> {
        let path = process_output_path(scope, process_id)?;
        let bytes = serialize_pretty(output)?;
        self.filesystem.as_ref().write_file(&path, &bytes).await?;
        Ok(path)
    }

    async fn store_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        output: Option<Value>,
        output_ref: Option<VirtualPath>,
        error_kind: Option<String>,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let record = ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status,
            output,
            output_ref,
            error_kind,
        };
        self.write_result(&record).await?;
        Ok(record)
    }
}

#[async_trait]
impl<F> ProcessResultStore for FilesystemProcessResultStore<'_, F>
where
    F: RootFilesystem,
{
    /// Persist a successful terminal record and its output blob.
    ///
    /// Writes happen in two steps (`write_output` then `write_result`); if
    /// the second write fails, the output blob at
    /// `process-outputs/<process_id>/output.json` is left on disk as an
    /// orphan. Cleanup of orphaned output blobs is the caller's responsibility
    /// (typically swept during operator-initiated reconciliation rather than
    /// inline, since orphans are observable via missing
    /// `process-results/<process_id>.json`).
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let output_ref = self.write_output(scope, process_id, &output).await?;
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Completed,
            None,
            Some(output_ref),
            None,
        )
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Failed,
            None,
            None,
            Some(error_kind),
        )
        .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(scope, process_id, ProcessStatus::Killed, None, None, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        let path = process_result_path(scope, process_id)?;
        let bytes = match self.filesystem.as_ref().read_file(&path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let record = deserialize::<ProcessResultRecord>(&bytes)?;
        ensure_result_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        let Some(record) = self.get(scope, process_id).await? else {
            return Ok(None);
        };
        if let Some(output) = record.output {
            return Ok(Some(output));
        }
        let Some(output_ref) = record.output_ref else {
            return Ok(None);
        };
        let expected_output_ref = process_output_path(scope, process_id)?;
        if output_ref != expected_output_ref {
            return Err(invalid_stored_record(format!(
                "process result output ref {} does not match expected {}",
                output_ref.as_str(),
                expected_output_ref.as_str()
            )));
        }
        let bytes = match self.filesystem.as_ref().read_file(&output_ref).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        deserialize::<Value>(&bytes).map(Some)
    }
}

fn process_record_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/{process_id}.json",
        process_records_root(scope)?.as_str()
    ))
    .map_err(invalid_path)
}

fn process_records_root(scope: &ResourceScope) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!("{}/processes", resource_owner_root(scope))).map_err(invalid_path)
}

fn process_result_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/{process_id}.json",
        process_results_root(scope)?.as_str()
    ))
    .map_err(invalid_path)
}

fn process_results_root(scope: &ResourceScope) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!("{}/process-results", resource_owner_root(scope)))
        .map_err(invalid_path)
}

fn process_output_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/output.json",
        process_outputs_root(scope, process_id)?.as_str()
    ))
    .map_err(invalid_path)
}

fn process_outputs_root(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<VirtualPath, ProcessError> {
    VirtualPath::new(format!(
        "{}/process-outputs/{process_id}",
        resource_owner_root(scope)
    ))
    .map_err(invalid_path)
}

fn resource_owner_root(scope: &ResourceScope) -> String {
    let mut base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    if let Some(agent_id) = &scope.agent_id {
        base = format!("{base}/agents/{}", agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base = format!("{base}/projects/{}", project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base = format!("{base}/missions/{}", mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base = format!("{base}/threads/{}", thread_id.as_str());
    }
    base
}

fn ensure_process_record_matches(
    record: &ProcessRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn ensure_result_record_matches(
    record: &ProcessResultRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process result id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn invalid_stored_record(reason: impl Into<String>) -> ProcessError {
    ProcessError::InvalidStoredRecord {
        reason: reason.into(),
    }
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, ProcessError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(|error| ProcessError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ProcessError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| ProcessError::Deserialization(error.to_string()))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}
