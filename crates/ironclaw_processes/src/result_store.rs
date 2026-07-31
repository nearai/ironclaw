//! Filesystem-backed process result metadata and externalized output bodies.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::sanitize_error_kind;
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::ProcessId,
    path::{ScopedPath, VirtualPath},
    resource::ResourceScope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{
    ProcessError, ProcessResultRecord, ProcessResultStorePort, ProcessStatus, invalid_path,
    same_scope_owner,
};

pub struct ProcessResultStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> ProcessResultStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    pub fn from_arc(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::new(filesystem)
    }

    async fn write_result(&self, record: &ProcessResultRecord) -> Result<(), ProcessError> {
        let path = process_result_path(&record.scope, record.process_id)?;
        let body = serialize_pretty(record)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&record.scope, &path, entry, CasExpectation::Any)
            .await?;
        Ok(())
    }

    async fn write_output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: &Value,
    ) -> Result<VirtualPath, ProcessError> {
        let path = process_output_path(scope, process_id)?;
        let body = serialize_pretty(output)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(scope, &path, entry, CasExpectation::Any)
            .await?;
        self.filesystem
            .resolve(scope, &path)
            .map_err(ProcessError::Filesystem)
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
impl<F> ProcessResultStorePort for ProcessResultStore<F>
where
    F: RootFilesystem + 'static,
{
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
            Some(sanitize_error_kind(error_kind)),
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
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let record = deserialize::<ProcessResultRecord>(&versioned.entry.body)?;
        ensure_result_record_matches(&record, process_id)?;
        Ok(same_scope_owner(&record.scope, scope).then_some(record))
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
        let expected_scoped = process_output_path(scope, process_id)?;
        let expected_virtual = self
            .filesystem
            .resolve(scope, &expected_scoped)
            .map_err(ProcessError::Filesystem)?;
        if output_ref != expected_virtual {
            return Err(ProcessError::InvalidStoredRecord {
                reason: format!(
                    "process result output ref {} does not match expected {}",
                    output_ref.as_str(),
                    expected_virtual.as_str()
                ),
            });
        }
        let Some(versioned) = self.filesystem.get(scope, &expected_scoped).await? else {
            return Ok(None);
        };
        deserialize::<Value>(&versioned.entry.body).map(Some)
    }
}

const PROCESSES_PREFIX: &str = "/processes";

fn process_result_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessError> {
    scoped_path(&format!(
        "{}/results/{process_id}.json",
        scope_owner_root_string(scope)
    ))
}

fn process_output_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessError> {
    scoped_path(&format!(
        "{}/outputs/{process_id}/output.json",
        scope_owner_root_string(scope)
    ))
}

fn scope_owner_root_string(scope: &ResourceScope) -> String {
    let mut base = String::from(PROCESSES_PREFIX);
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base.push_str("/threads/");
        base.push_str(thread_id.as_str());
    }
    base
}

fn scoped_path(raw: &str) -> Result<ScopedPath, ProcessError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

fn ensure_result_record_matches(
    record: &ProcessResultRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id == process_id {
        Ok(())
    } else {
        Err(ProcessError::InvalidStoredRecord {
            reason: format!(
                "stored process result id {} does not match requested {}",
                record.process_id, process_id
            ),
        })
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
