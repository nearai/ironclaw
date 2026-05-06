//! In-memory process and process-result stores.
//!
//! Used by tests, in-memory composition (`ProcessServices::in_memory`), and
//! anywhere durability isn't required. The persistent equivalents live in
//! [`crate::filesystem_store`].

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use async_trait::async_trait;
use ironclaw_host_api::{ProcessId, ResourceScope};
use serde_json::Value;

use crate::types::{
    ProcessError, ProcessKey, ProcessRecord, ProcessResultRecord, ProcessResultStore, ProcessStart,
    ProcessStatus, ProcessStore, ensure_status_transition, same_scope_owner,
};

#[derive(Debug, Default)]
pub struct InMemoryProcessStore {
    records: Mutex<HashMap<ProcessKey, ProcessRecord>>,
}

impl InMemoryProcessStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        to: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let key = ProcessKey::new(scope, process_id);
        let mut records = self.records_guard();
        let record = records
            .get_mut(&key)
            .ok_or(ProcessError::UnknownProcess { process_id })?;
        ensure_status_transition(process_id, record.status, to)?;
        record.status = to;
        record.error_kind = error_kind;
        Ok(record.clone())
    }
}

#[async_trait]
impl ProcessStore for InMemoryProcessStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
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
        let key = ProcessKey::new(&record.scope, record.process_id);
        let mut records = self.records_guard();
        if records.contains_key(&key) {
            return Err(ProcessError::ProcessAlreadyExists {
                process_id: record.process_id,
            });
        }
        records.insert(key, record.clone());
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Failed, Some(error_kind))
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        Ok(self
            .records_guard()
            .get(&ProcessKey::new(scope, process_id))
            .cloned())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let mut records = self
            .records_guard()
            .values()
            .filter(|record| same_scope_owner(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryProcessResultStore {
    records: Mutex<HashMap<ProcessKey, ProcessResultRecord>>,
}

impl InMemoryProcessResultStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessResultRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn store_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        output: Option<Value>,
        error_kind: Option<String>,
    ) -> ProcessResultRecord {
        let record = ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status,
            output,
            output_ref: None,
            error_kind,
        };
        self.records_guard()
            .insert(ProcessKey::new(scope, process_id), record.clone());
        record
    }
}

#[async_trait]
impl ProcessResultStore for InMemoryProcessResultStore {
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(
            scope,
            process_id,
            ProcessStatus::Completed,
            Some(output),
            None,
        ))
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(
            scope,
            process_id,
            ProcessStatus::Failed,
            None,
            Some(error_kind),
        ))
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        Ok(self.store_result(scope, process_id, ProcessStatus::Killed, None, None))
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        Ok(self
            .records_guard()
            .get(&ProcessKey::new(scope, process_id))
            .cloned())
    }
}
