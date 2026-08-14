//! Process test doubles and in-memory-backed production-store constructors.
//!
//! The Reborn architecture-simplification note
//! (`docs/internal/reborn/contracts/processes.md`)
//! replaces the hand-written `InMemory*Store` parallel implementations with the
//! one production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). These helpers wire that seam once — a
//! `ScopedFilesystem<InMemoryBackend>` mounted at `/processes` — so tests
//! instantiate the same store a deployment runs.
//!
//! Note on sub-scope isolation: `ProcessJournalStore` encodes
//! agent/project/mission/thread in the path (structural under any mount), while
//! tenant/user isolation lives in the `MountView`. The single fixed mount below
//! therefore isolates by agent/project/mission/thread but not by tenant/user —
//! which matches single-tenant state-machine tests; cross-tenant isolation is
//! exercised by the `filesystem_process_store_isolates_two_tenants_*` tests,
//! which mount per tenant/user.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` so nothing here
//! ships in production binaries; downstream crates enable the `test-support`
//! feature from their `[dev-dependencies]`.

use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    approval::ApprovalRequest,
    ids::InvocationId,
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};

use crate::types::same_scope_owner;
use crate::{
    ProcessInvocationError, ProcessInvocationRecord, ProcessInvocationStart,
    ProcessInvocationStatePort, ProcessInvocationStatus, ProcessJournalStore, ProcessResultStore,
    ProcessServices,
};

/// One named step in a sequential state-transition table.
///
/// Keeping the command and expected result together makes state-machine tests
/// read like the transition contract instead of repeating arrange/act/assert
/// plumbing. The state is deliberately supplied to the runner separately so
/// every step observes the state left by the previous one.
#[derive(Debug)]
pub struct StateTransitionCase<Command, Output, Error> {
    pub name: &'static str,
    pub command: Command,
    pub expected: Result<Output, Error>,
}

impl<Command, Output, Error> StateTransitionCase<Command, Output, Error> {
    pub fn new(name: &'static str, command: Command, expected: Result<Output, Error>) -> Self {
        Self {
            name,
            command,
            expected,
        }
    }
}

/// Execute a sequential transition table and attach the case name to failures.
///
/// Pure materialized-state machines can call this directly. Use
/// [`assert_async_state_transition_table`] for stores and service objects.
pub fn assert_state_transition_table<State, Command, Output, Error>(
    state: &mut State,
    cases: impl IntoIterator<Item = StateTransitionCase<Command, Output, Error>>,
    mut apply: impl FnMut(&mut State, Command) -> Result<Output, Error>,
) where
    Output: std::fmt::Debug + PartialEq,
    Error: std::fmt::Debug + PartialEq,
{
    for case in cases {
        let actual = apply(state, case.command);
        assert_eq!(actual, case.expected, "transition case: {}", case.name);
    }
}

/// Async counterpart to [`assert_state_transition_table`].
pub async fn assert_async_state_transition_table<State, Command, Output, Error>(
    state: &mut State,
    cases: impl IntoIterator<Item = StateTransitionCase<Command, Output, Error>>,
    mut apply: impl for<'state> FnMut(
        &'state mut State,
        Command,
    ) -> futures::future::BoxFuture<'state, Result<Output, Error>>,
) where
    Output: std::fmt::Debug + PartialEq,
    Error: std::fmt::Debug + PartialEq,
{
    for case in cases {
        let actual = apply(state, case.command).await;
        assert_eq!(actual, case.expected, "transition case: {}", case.name);
    }
}

/// Pure, mutex-backed fake for caller-level invocation-state tests.
///
/// Despite the historical generic name, this type does not exercise
/// `RootFilesystem`, CAS, row encoding, or durable-backend behavior. Use
/// [`ProcessJournalStore<InMemoryBackend>`] for production-store coverage.
/// The backend marker remains only for source compatibility with existing
/// test harness type annotations.
pub struct ProcessInvocationStateStore<F> {
    records: Mutex<Vec<ProcessInvocationRecord>>,
    backend: PhantomData<F>,
}

impl<F> ProcessInvocationStateStore<F> {
    pub fn new(_filesystem: Arc<ScopedFilesystem<F>>) -> Self
    where
        F: ironclaw_filesystem::RootFilesystem,
    {
        Self {
            records: Mutex::new(Vec::new()),
            backend: PhantomData,
        }
    }

    fn update(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        mutate: impl FnOnce(&mut ProcessInvocationRecord),
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let mut records = self.records.lock().map_err(|_| {
            ProcessInvocationError::Backend("test process-invocation mutex poisoned".to_string())
        })?;
        let record = records
            .iter_mut()
            .find(|record| {
                record.invocation_id == invocation_id && same_scope_owner(&record.scope, scope)
            })
            .ok_or(ProcessInvocationError::UnknownInvocation { invocation_id })?;
        mutate(record);
        Ok(record.clone())
    }
}

#[async_trait]
impl<F> ProcessInvocationStatePort for ProcessInvocationStateStore<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync,
{
    async fn start(
        &self,
        start: ProcessInvocationStart,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        let mut records = self.records.lock().map_err(|_| {
            ProcessInvocationError::Backend("test process-invocation mutex poisoned".to_string())
        })?;
        if records.iter().any(|record| {
            record.invocation_id == start.invocation_id
                && same_scope_owner(&record.scope, &start.scope)
        }) {
            return Err(ProcessInvocationError::InvocationAlreadyExists {
                invocation_id: start.invocation_id,
            });
        }
        let record = ProcessInvocationRecord {
            invocation_id: start.invocation_id,
            capability_id: start.capability_id,
            scope: start.scope,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
            status: ProcessInvocationStatus::Running,
            approval_request_id: None,
            error_kind: None,
        };
        records.push(record.clone());
        Ok(record)
    }

    async fn block_approval(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        approval: ApprovalRequest,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.update(scope, invocation_id, |record| {
            record.status = ProcessInvocationStatus::BlockedApproval;
            record.approval_request_id = Some(approval.id);
            record.error_kind = None;
        })
    }

    async fn block_auth(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.update(scope, invocation_id, |record| {
            record.status = ProcessInvocationStatus::BlockedAuth;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.update(scope, invocation_id, |record| {
            record.status = ProcessInvocationStatus::Completed;
            record.approval_request_id = None;
            record.error_kind = None;
        })
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
        error_kind: String,
    ) -> Result<ProcessInvocationRecord, ProcessInvocationError> {
        self.update(scope, invocation_id, |record| {
            record.status = ProcessInvocationStatus::Failed;
            record.approval_request_id = None;
            record.error_kind = Some(error_kind);
        })
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        invocation_id: InvocationId,
    ) -> Result<Option<ProcessInvocationRecord>, ProcessInvocationError> {
        let records = self.records.lock().map_err(|_| {
            ProcessInvocationError::Backend("test process-invocation mutex poisoned".to_string())
        })?;
        Ok(records
            .iter()
            .find(|record| {
                record.invocation_id == invocation_id && same_scope_owner(&record.scope, scope)
            })
            .cloned())
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessInvocationRecord>, ProcessInvocationError> {
        let records = self.records.lock().map_err(|_| {
            ProcessInvocationError::Backend("test process-invocation mutex poisoned".to_string())
        })?;
        let mut visible = records
            .iter()
            .filter(|record| same_scope_owner(&record.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        visible.sort_by_key(|record| record.invocation_id.as_uuid());
        Ok(visible)
    }
}

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounted at `/processes`
/// — the in-memory backend seam every process store uses in tests.
pub fn in_memory_backed_processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/processes").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid processes mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production process store over a fresh in-memory backend — the drop-in
/// replacement for the deleted `InMemoryProcessStore`.
pub fn in_memory_backed_process_store() -> ProcessJournalStore<InMemoryBackend> {
    ProcessJournalStore::new(in_memory_backed_processes_filesystem())
}

pub fn in_memory_backed_process_invocation_state_store()
-> ProcessInvocationStateStore<InMemoryBackend> {
    ProcessInvocationStateStore::new(in_memory_backed_processes_filesystem())
}

/// The production process result store over a fresh in-memory backend — the
/// drop-in replacement for the deleted `InMemoryProcessResultStore`.
pub fn in_memory_backed_process_result_store() -> ProcessResultStore<InMemoryBackend> {
    ProcessResultStore::new(in_memory_backed_processes_filesystem())
}

/// A [`ProcessServices`] whose lifecycle and result stores share **one**
/// in-memory-backed `/processes` filesystem — the drop-in replacement for the
/// deleted `ProcessServices::in_memory()`. Use this (not the two standalone
/// helpers) when a test starts a process and reads back its result, since both
/// stores must resolve against the same backend.
pub fn in_memory_backed_process_services() -> ProcessServices {
    ProcessServices::filesystem(in_memory_backed_processes_filesystem())
}
