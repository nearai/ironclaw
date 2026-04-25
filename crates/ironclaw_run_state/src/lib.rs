//! Run-state contracts for IronClaw Reborn.
//!
//! `ironclaw_run_state` stores the current lifecycle state for host-managed
//! invocations. It is separate from runtime events: events are append-only
//! history, while run state answers "what is this invocation waiting on now?".

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use ironclaw_host_api::{
    ApprovalRequest, ApprovalRequestId, CapabilityId, InvocationId, ResourceScope,
};
use thiserror::Error;

/// Current lifecycle state for one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    BlockedApproval,
    BlockedAuth,
    Completed,
    Failed,
}

/// State record keyed by invocation ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub status: RunStatus,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub error_kind: Option<String>,
}

/// Start metadata for a capability invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
}

/// Run-state store errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RunStateError {
    #[error("unknown invocation {invocation_id}")]
    UnknownInvocation { invocation_id: InvocationId },
}

/// Current-state store for invocation lifecycle.
pub trait RunStateStore: Send + Sync {
    fn start(&self, start: RunStart) -> RunRecord;
    fn block_approval(
        &self,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError>;
    fn block_auth(
        &self,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;
    fn complete(&self, invocation_id: InvocationId) -> Result<RunRecord, RunStateError>;
    fn fail(
        &self,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError>;
    fn get(&self, invocation_id: InvocationId) -> Option<RunRecord>;
    fn records(&self) -> Vec<RunRecord>;
}

/// In-memory run-state store for tests and early host wiring.
#[derive(Debug, Default)]
pub struct InMemoryRunStateStore {
    records: Mutex<HashMap<InvocationId, RunRecord>>,
}

impl InMemoryRunStateStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn update(
        &self,
        invocation_id: InvocationId,
        update: impl FnOnce(&mut RunRecord),
    ) -> Result<RunRecord, RunStateError> {
        let mut records = self.records_guard();
        let record = records
            .get_mut(&invocation_id)
            .ok_or(RunStateError::UnknownInvocation { invocation_id })?;
        update(record);
        Ok(record.clone())
    }

    fn records_guard(&self) -> MutexGuard<'_, HashMap<InvocationId, RunRecord>> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl RunStateStore for InMemoryRunStateStore {
    fn start(&self, start: RunStart) -> RunRecord {
        let record = RunRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            status: RunStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        self.records_guard()
            .insert(record.invocation_id, record.clone());
        record
    }

    fn block_approval(
        &self,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        self.update(invocation_id, |record| {
            record.status = RunStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
    }

    fn block_auth(
        &self,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update(invocation_id, |record| {
            record.status = RunStatus::BlockedAuth;
            record.error_kind = Some(error_kind);
        })
    }

    fn complete(&self, invocation_id: InvocationId) -> Result<RunRecord, RunStateError> {
        self.update(invocation_id, |record| {
            record.status = RunStatus::Completed;
            record.error_kind = None;
        })
    }

    fn fail(
        &self,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        self.update(invocation_id, |record| {
            record.status = RunStatus::Failed;
            record.error_kind = Some(error_kind);
        })
    }

    fn get(&self, invocation_id: InvocationId) -> Option<RunRecord> {
        self.records_guard().get(&invocation_id).cloned()
    }

    fn records(&self) -> Vec<RunRecord> {
        let mut records = self.records_guard().values().cloned().collect::<Vec<_>>();
        records.sort_by_key(|record| record.invocation_id.as_uuid());
        records
    }
}
