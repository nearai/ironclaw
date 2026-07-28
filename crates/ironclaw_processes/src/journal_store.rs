use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, FileType, FilesystemError, Filter, Page, RootFilesystem, ScopedFilesystem,
    SeqNo,
};
use ironclaw_host_api::{ProcessId, ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::ProcessRuntimePort;
use crate::journal::{
    CancelProcessRequest, ClaimProcessesRequest, ClaimedProcess, CloseProcessDependencyRequest,
    FailProcessRequest, GetProcessCheckpointRequest, GetProcessSnapshotRequest,
    JournaledProcessSnapshot, KillProcessRequest, OpenProcessDependencyRequest,
    ProcessCheckpointPort, ProcessCheckpointRecord, ProcessConcurrencyLimits, ProcessControlPort,
    ProcessControlResult, ProcessDependencyPort, ProcessDependencyQuery, ProcessDependencyRecord,
    ProcessGateQuery, ProcessGateQuerySource, ProcessGateRecord, ProcessJournalCursor,
    ProcessJournalEntry, ProcessJournalKind, ProcessJournalPage, ProcessJournalSource,
    ProcessLeaseRequest, ProcessLifecycleLookupBatchRequest, ProcessLifecycleLookupResult,
    ProcessLifecycleLookupSource, ProcessLifecycleStatus, ProcessOperationId,
    ProcessSubmissionPort, ProcessSuspension, ProcessTransitionPort, ProcessTreePort,
    ProcessTreeReservation, PruneReleasedProcessRequest, RecordProcessCheckpointRequest,
    RecoverExpiredProcessLeasesRequest, RecoverExpiredProcessLeasesResponse,
    ReleaseProcessTreeRequest, ReserveProcessTreeRequest, ResumeProcessRequest,
    SettleProcessDependencyRequest, StopProcessRequest, SubmitProcessRequest,
    SubmitProcessWithCheckpointRequest, SuspendProcessRequest,
};
use crate::types::{invalid_path, same_scope_owner};

mod command;
mod migration;
mod observer;
mod rows;
mod state;
mod validation;
use command::StoredProcessCommand;
use migration::legacy_turn_record_contains_data;
use observer::RegisteredProcessObserver;
use state::ProcessJournalMaterializedState;
use validation::{
    ensure_lease, ensure_transition, process_claim_within_limits, process_gate_snapshot_matches,
    process_scope_visible, same_lineage_scope, validate_tree_root,
};

const LEGACY_COMMAND_LOG_PATH: &str = "/processes/journal/records";
const LEGACY_JOURNAL_STATE_PATH: &str = "/processes/journal/state.json";
const DEFAULT_LEASE_DURATION: Duration = Duration::from_secs(90);
const JOURNAL_READ_BATCH: usize = 1024;
const MAX_TRANSACTION_RETRIES: usize = 64;

#[derive(Debug, Error)]
pub enum ProcessJournalStoreError {
    #[error("unknown process {process_id}")]
    UnknownProcess { process_id: ProcessId },
    #[error("process {process_id} already exists")]
    ProcessAlreadyExists { process_id: ProcessId },
    #[error(
        "scope already has active {process_kind:?} process {process_id} in {status:?} at cursor {cursor:?}"
    )]
    ActiveProcessConflict {
        process_id: ProcessId,
        process_kind: crate::ProcessKind,
        status: ProcessLifecycleStatus,
        suspension: Option<Box<ProcessSuspension>>,
        cursor: ProcessJournalCursor,
    },
    #[error("process {process_id} cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        process_id: ProcessId,
        from: ProcessLifecycleStatus,
        to: ProcessLifecycleStatus,
    },
    #[error("process {process_id} lease is invalid")]
    InvalidLease { process_id: ProcessId },
    #[error("process scope is not authorized for lineage operation")]
    UnauthorizedScope,
    #[error("invalid process journal request: {0}")]
    InvalidRequest(String),
    #[error("process tree descendant capacity {cap} exceeded")]
    ProcessTreeCapacityExceeded { cap: u32 },
    #[error("process {process_id} changed after cursor {expected:?}; current cursor is {actual:?}")]
    StaleSnapshot {
        process_id: ProcessId,
        expected: ProcessJournalCursor,
        actual: ProcessJournalCursor,
    },
    #[error("invalid storage path: {0}")]
    InvalidPath(String),
    #[error("filesystem error: {0}")]
    Filesystem(#[from] FilesystemError),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
    #[error("process journal observer error: {0}")]
    Observer(String),
    #[error("legacy process lifecycle data requires migration before row-native initialization")]
    MigrationRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "schema", content = "command", rename_all = "snake_case")]
enum StoredProcessJournalRecord {
    V1(StoredProcessCommand),
}

#[derive(Debug)]
enum StoredCommandOutcome {
    Imported,
    Submitted(JournaledProcessSnapshot, bool),
    Claimed(Vec<ClaimedProcess>),
    Heartbeat(JournaledProcessSnapshot),
    Recovered(RecoverExpiredProcessLeasesResponse),
    Transitioned(JournaledProcessSnapshot),
    Controlled(ProcessControlResult, Option<ProcessJournalKind>),
    TreeReserved(ProcessTreeReservation),
    TreeReleased,
    TreePruned,
    Dependency(Option<ProcessDependencyRecord>),
    Checkpointed(ProcessCheckpointRecord),
}

#[async_trait]
impl<F> crate::ProcessInputPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn get_process_input(
        &self,
        request: crate::GetProcessInputRequest,
    ) -> Result<Option<crate::ProcessInputRecord>, Self::Error> {
        self.ensure_materialized().await?;
        let path = rows::input_scoped_path(request.process_id)?;
        let record = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await?
            .as_ref()
            .map(rows::decode_input)
            .transpose()?
            .flatten();
        Ok(record.filter(|record| process_scope_visible(&record.scope, &request.scope)))
    }
}

pub struct ProcessJournalStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    migration: Arc<Mutex<()>>,
    materialized_ready: Arc<AtomicBool>,
    observers: Arc<StdMutex<Vec<RegisteredProcessObserver>>>,
    lease_duration: Duration,
    concurrency_limits: ProcessConcurrencyLimits,
}

impl<F> Clone for ProcessJournalStore<F>
where
    F: RootFilesystem,
{
    fn clone(&self) -> Self {
        Self {
            filesystem: Arc::clone(&self.filesystem),
            migration: Arc::clone(&self.migration),
            materialized_ready: Arc::clone(&self.materialized_ready),
            observers: Arc::clone(&self.observers),
            lease_duration: self.lease_duration,
            concurrency_limits: self.concurrency_limits.clone(),
        }
    }
}

impl<F> ProcessJournalStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            migration: Arc::new(Mutex::new(())),
            materialized_ready: Arc::new(AtomicBool::new(false)),
            observers: Arc::new(StdMutex::new(Vec::new())),
            lease_duration: DEFAULT_LEASE_DURATION,
            concurrency_limits: ProcessConcurrencyLimits::default(),
        }
    }

    pub fn with_lease_duration(mut self, lease_duration: Duration) -> Self {
        self.lease_duration = lease_duration;
        self
    }

    pub fn with_concurrency_limits(mut self, limits: ProcessConcurrencyLimits) -> Self {
        self.concurrency_limits = limits;
        self
    }

    async fn submit_process_inner(
        &self,
        request: SubmitProcessRequest,
    ) -> Result<(JournaledProcessSnapshot, bool), ProcessJournalStoreError> {
        match self
            .execute(StoredProcessCommand::Submit(Box::new(request)))
            .await?
        {
            StoredCommandOutcome::Submitted(snapshot, changed) => Ok((snapshot, changed)),
            outcome => Err(unexpected_outcome("submit", outcome)),
        }
    }

    async fn execute(
        &self,
        command: StoredProcessCommand,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError> {
        self.ensure_materialized().await?;
        let references = command.load_references()?;
        for attempt in 0..MAX_TRANSACTION_RETRIES {
            let mut loaded = rows::load(self.filesystem.as_ref(), &references).await?;
            let mut state = std::mem::take(&mut loaded.state);
            let prefix = processes_prefix()?;
            let mut txn = self
                .filesystem
                .begin(&ResourceScope::system(), &prefix)
                .await?;
            let sequence_path = self
                .filesystem
                .resolve(&ResourceScope::system(), &process_journal_sequence_path()?)?;
            let reservation_count = command.cursor_reservation_count(state.processes.len());
            let mut first_cursor = None;
            for _ in 0..reservation_count {
                let reserved = txn.reserve_sequence(&sequence_path).await?;
                first_cursor.get_or_insert(reserved.get());
            }
            state.next_cursor = first_cursor.unwrap_or(1);
            let outcome = match state.apply_command(command.clone()) {
                Ok(outcome) => outcome,
                Err(error) => {
                    txn.rollback().await;
                    return Err(error);
                }
            };
            let entries = std::mem::take(&mut state.journal);
            let result = async {
                rows::persist(self.filesystem.as_ref(), txn.as_mut(), &loaded, &state).await?;
                rows::persist_journal(self.filesystem.as_ref(), txn.as_mut(), entries.as_slice())
                    .await?;
                Ok::<(), ProcessJournalStoreError>(())
            }
            .await;
            match result {
                Ok(()) => match txn.commit().await {
                    Ok(()) => return Ok(outcome),
                    Err(error) if rows::retryable_transaction_error(&error) => {
                        rows::retry_transaction(attempt).await;
                    }
                    Err(error) => return Err(error.into()),
                },
                Err(ProcessJournalStoreError::Filesystem(error))
                    if rows::retryable_transaction_error(&error) =>
                {
                    txn.rollback().await;
                    rows::retry_transaction(attempt).await;
                }
                Err(error) => {
                    txn.rollback().await;
                    return Err(error);
                }
            }
        }
        Err(ProcessJournalStoreError::Filesystem(
            FilesystemError::BackendBusy {
                path: self
                    .filesystem
                    .resolve(&ResourceScope::system(), &processes_prefix()?)?,
                operation: ironclaw_filesystem::FilesystemOperation::BeginTxn,
            },
        ))
    }

    async fn load_process(
        &self,
        process_id: ProcessId,
    ) -> Result<Option<JournaledProcessSnapshot>, ProcessJournalStoreError> {
        self.ensure_materialized().await?;
        let path = rows::process_scoped_path(process_id)?;
        self.filesystem
            .get(&ResourceScope::system(), &path)
            .await?
            .as_ref()
            .map(rows::decode_process)
            .transpose()
            .map(Option::flatten)
    }

    async fn read_journal_page(
        &self,
        scope: Option<&ResourceScope>,
        owner_user_id: Option<&ironclaw_host_api::UserId>,
        after: Option<ProcessJournalCursor>,
        limit: usize,
    ) -> Result<ProcessJournalPage, ProcessJournalStoreError> {
        self.ensure_materialized().await?;
        if limit >= ironclaw_filesystem::Page::MAX_LIMIT as usize {
            return Err(ProcessJournalStoreError::InvalidRequest(format!(
                "process journal page limit must be below {}",
                ironclaw_filesystem::Page::MAX_LIMIT
            )));
        }
        let after_cursor = after.map(|cursor| cursor.0).unwrap_or(0);
        let mut entries = rows::journal_page(
            self.filesystem.as_ref(),
            scope,
            owner_user_id,
            after_cursor,
            limit.saturating_add(1),
        )
        .await?;
        let truncated = entries.len() > limit;
        if truncated {
            entries.truncate(limit);
        }
        let next_cursor = entries
            .last()
            .map(|entry| entry.cursor)
            .unwrap_or(ProcessJournalCursor(after_cursor));
        Ok(ProcessJournalPage {
            entries,
            next_cursor,
            truncated,
            rebase_required: None,
        })
    }

    async fn ensure_materialized(&self) -> Result<(), ProcessJournalStoreError> {
        if self.materialized_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        let _guard = self.migration.lock().await;
        if self.materialized_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        rows::ensure_indexes(self.filesystem.as_ref()).await?;
        if rows::is_initialized(self.filesystem.as_ref()).await? {
            self.materialized_ready.store(true, Ordering::Release);
            return Ok(());
        }
        if self.legacy_process_journal_present().await? {
            return Err(ProcessJournalStoreError::MigrationRequired);
        }
        self.initialize_materialized(false).await?;
        self.materialized_ready.store(true, Ordering::Release);
        Ok(())
    }

    /// Explicit offline migration for the legacy command log/blob.
    ///
    /// Normal construction and request handling never invoke this method. It
    /// must run before any request initializes the row-native journal.
    pub async fn migrate_legacy_journal(&self) -> Result<usize, ProcessJournalStoreError> {
        let _guard = self.migration.lock().await;
        rows::ensure_indexes(self.filesystem.as_ref()).await?;
        if rows::is_initialized(self.filesystem.as_ref()).await? {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "legacy process journal migration must run before row-native initialization"
                    .to_string(),
            ));
        }
        if self.deployed_legacy_authority_present().await? {
            return Err(ProcessJournalStoreError::MigrationRequired);
        }
        let imported = self.initialize_materialized(true).await?;
        self.materialized_ready.store(true, Ordering::Release);
        Ok(imported)
    }

    /// Explicit offline rebuild for row-native ordered projections.
    ///
    /// This is the only path allowed to enumerate materialized collections.
    /// It rewrites each row under CAS so backend projection triggers populate
    /// indexes introduced after those rows were stored.
    pub async fn migrate_row_native_indexes(&self) -> Result<usize, ProcessJournalStoreError> {
        let _guard = self.migration.lock().await;
        rows::ensure_indexes(self.filesystem.as_ref()).await?;
        let mut migrated = 0usize;
        for collection in ["journal", "process", "dependency"] {
            let prefix = ScopedPath::new(format!("/processes/materialized/{collection}"))
                .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))?;
            let mut offset = 0u64;
            loop {
                let batch = self
                    .filesystem
                    .query(
                        &ResourceScope::system(),
                        &prefix,
                        &Filter::All,
                        Page::new(offset, Page::MAX_LIMIT),
                    )
                    .await?;
                if batch.is_empty() {
                    break;
                }
                let received = batch.len();
                let mut txn = self
                    .filesystem
                    .begin(&ResourceScope::system(), &prefix)
                    .await?;
                for row in batch {
                    let entry = rows::encode(&row.path, &rows::decode(&row)?)?;
                    txn.put(&row.path, entry, CasExpectation::Version(row.version))
                        .await?;
                }
                txn.commit().await?;
                migrated = migrated.saturating_add(received);
                if received < Page::MAX_LIMIT as usize {
                    break;
                }
                offset = offset.saturating_add(received as u64);
            }
        }
        Ok(migrated)
    }

    async fn legacy_process_journal_present(&self) -> Result<bool, ProcessJournalStoreError> {
        if self.deployed_legacy_authority_present().await? {
            return Ok(true);
        }
        let state_present = self
            .filesystem
            .get(&ResourceScope::system(), &legacy_journal_state_path()?)
            .await?
            .is_some();
        if state_present {
            return Ok(true);
        }
        Ok(self
            .filesystem
            .head_seq(
                &ResourceScope::system(),
                &legacy_command_log_path()?,
                SeqNo::ZERO,
            )
            .await?
            .is_some())
    }

    async fn deployed_legacy_authority_present(&self) -> Result<bool, ProcessJournalStoreError> {
        for raw in ["/turns/state.json", "/turns/rows/v1/meta/state.json"] {
            let path = ScopedPath::new(raw)
                .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))?;
            // Test and narrow-purpose mount views may intentionally expose only
            // `/processes`. Probe deployed authorities only when their legacy
            // alias is present in the supplied production view.
            if self
                .filesystem
                .resolve(&ResourceScope::system(), &path)
                .is_err()
            {
                continue;
            }
            match self.filesystem.get(&ResourceScope::system(), &path).await {
                Ok(Some(versioned))
                    if legacy_turn_record_contains_data(raw, &versioned.entry.body)? =>
                {
                    tracing::debug!(path = raw, "detected deployed legacy turn authority");
                    return Ok(true);
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }

        let runs_path = ScopedPath::new("/turns/rows/v1/runs")
            .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))?;
        if self
            .filesystem
            .resolve(&ResourceScope::system(), &runs_path)
            .is_ok()
            && !self
                .filesystem
                .list_dir_bounded(&ResourceScope::system(), &runs_path, 1)
                .await
                .or_else(|error| match error {
                    FilesystemError::NotFound { .. } => Ok(Vec::new()),
                    error => Err(error),
                })?
                .is_empty()
        {
            tracing::debug!(
                path = %runs_path,
                "detected deployed legacy turn run rows"
            );
            return Ok(true);
        }
        self.deployed_run_state_records_present().await
    }

    async fn deployed_run_state_records_present(&self) -> Result<bool, ProcessJournalStoreError> {
        let root = ScopedPath::new("/run-state")
            .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))?;
        if self
            .filesystem
            .resolve(&ResourceScope::system(), &root)
            .is_err()
        {
            return Ok(false);
        }

        // Only populated scope-relative `runs` directories are legacy evidence;
        // other current records can legitimately share this mount.
        let mut pending = VecDeque::from([(root, 0_u8)]);
        while let Some((path, depth)) = pending.pop_front() {
            let entries = match self
                .filesystem
                .list_dir_bounded(&ResourceScope::system(), &path, Page::MAX_LIMIT as usize)
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::NotFound { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            for entry in entries {
                if entry.file_type != FileType::Directory {
                    continue;
                }
                let child = ScopedPath::new(format!(
                    "{}/{}",
                    path.as_str().trim_end_matches('/'),
                    entry.name
                ))
                .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))?;
                if entry.name == "runs" {
                    if !self
                        .filesystem
                        .list_dir_bounded(&ResourceScope::system(), &child, 1)
                        .await?
                        .is_empty()
                    {
                        tracing::debug!(
                            path = %child,
                            "detected deployed legacy run-state rows"
                        );
                        return Ok(true);
                    }
                    continue;
                }
                if depth < 8 {
                    pending.push_back((child, depth.saturating_add(1)));
                }
            }
        }
        Ok(false)
    }

    async fn initialize_materialized(
        &self,
        import_legacy: bool,
    ) -> Result<usize, ProcessJournalStoreError> {
        for attempt in 0..MAX_TRANSACTION_RETRIES {
            let mut loaded =
                rows::load(self.filesystem.as_ref(), &rows::LoadReferences::default()).await?;
            if loaded.initialized {
                return Ok(0);
            }
            let mut state = std::mem::take(&mut loaded.state);
            if import_legacy {
                let legacy_log = legacy_command_log_path()?;
                let mut applied = SeqNo::ZERO;
                loop {
                    let records = self
                        .filesystem
                        .tail_bounded(
                            &ResourceScope::system(),
                            &legacy_log,
                            applied,
                            JOURNAL_READ_BATCH,
                        )
                        .await?;
                    if records.is_empty() {
                        break;
                    }
                    for record in records {
                        applied = record.seq;
                        let stored: StoredProcessJournalRecord =
                            serde_json::from_slice(&record.payload).map_err(|error| {
                                ProcessJournalStoreError::Deserialization(error.to_string())
                            })?;
                        let StoredProcessJournalRecord::V1(command) = stored;
                        state.apply_command(command)?;
                    }
                }
                if applied == SeqNo::ZERO {
                    let legacy_path = legacy_journal_state_path()?;
                    if let Some(versioned) = self
                        .filesystem
                        .get(&ResourceScope::system(), &legacy_path)
                        .await?
                    {
                        state = serde_json::from_slice(&versioned.entry.body).map_err(|error| {
                            ProcessJournalStoreError::Deserialization(error.to_string())
                        })?;
                    }
                }
            }
            let entries = std::mem::take(&mut state.journal);
            let imported = entries.len();
            let prefix = processes_prefix()?;
            let mut txn = self
                .filesystem
                .begin(&ResourceScope::system(), &prefix)
                .await?;
            let result = async {
                if import_legacy {
                    let sequence_path = self
                        .filesystem
                        .resolve(&ResourceScope::system(), &process_journal_sequence_path()?)?;
                    for _ in 1..state.next_cursor {
                        txn.reserve_sequence(&sequence_path).await?;
                    }
                }
                rows::persist(self.filesystem.as_ref(), txn.as_mut(), &loaded, &state).await?;
                rows::persist_journal(self.filesystem.as_ref(), txn.as_mut(), entries.as_slice())
                    .await?;
                Ok::<(), ProcessJournalStoreError>(())
            }
            .await;
            match result {
                Ok(()) => match txn.commit().await {
                    Ok(()) => return Ok(imported),
                    Err(error) if rows::retryable_transaction_error(&error) => {
                        rows::retry_transaction(attempt).await;
                    }
                    Err(error) => return Err(error.into()),
                },
                Err(ProcessJournalStoreError::Filesystem(error))
                    if rows::retryable_transaction_error(&error) =>
                {
                    txn.rollback().await;
                    rows::retry_transaction(attempt).await;
                }
                Err(error) => {
                    txn.rollback().await;
                    return Err(error);
                }
            }
        }
        Err(ProcessJournalStoreError::Filesystem(
            FilesystemError::BackendBusy {
                path: self
                    .filesystem
                    .resolve(&ResourceScope::system(), &processes_prefix()?)?,
                operation: ironclaw_filesystem::FilesystemOperation::BeginTxn,
            },
        ))
    }
}

#[async_trait]
impl<F> ProcessSubmissionPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn submit_process(
        &self,
        request: SubmitProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let (snapshot, changed) = self.submit_process_inner(request).await?;
        if changed {
            self.notify_process_commit(snapshot.clone(), ProcessJournalKind::Submitted, None)
                .await;
        }
        Ok(snapshot)
    }

    async fn submit_process_with_checkpoint(
        &self,
        request: SubmitProcessWithCheckpointRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let outcome = if let Some(checkpoint) = request.checkpoint {
            self.execute(StoredProcessCommand::SubmitWithCheckpoint {
                request: Box::new(request.submission),
                checkpoint: Box::new(checkpoint),
            })
            .await?
        } else {
            self.execute(StoredProcessCommand::Submit(Box::new(request.submission)))
                .await?
        };
        let StoredCommandOutcome::Submitted(snapshot, changed) = outcome else {
            return Err(unexpected_outcome(
                "submit_process_with_checkpoint",
                outcome,
            ));
        };
        if changed {
            self.notify_process_commit(snapshot.clone(), ProcessJournalKind::Submitted, None)
                .await;
        }
        Ok(snapshot)
    }
}

#[async_trait]
impl<F> crate::ProcessSnapshotSource for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn process_snapshots(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error> {
        self.ensure_materialized().await?;
        if *scope == ResourceScope::system() {
            return Err(ProcessJournalStoreError::InvalidRequest(
                "system-wide process snapshot reads are unbounded; use paged process journal reads"
                    .to_string(),
            ));
        }
        let mut snapshots = rows::processes_for_scope(self.filesystem.as_ref(), scope).await?;
        snapshots.sort_by_key(|snapshot| snapshot.process_id.as_uuid());
        Ok(snapshots)
    }
}

#[async_trait]
impl<F> ProcessTransitionPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn claim_next_processes(
        &self,
        request: ClaimProcessesRequest,
    ) -> Result<Vec<ClaimedProcess>, Self::Error> {
        let lease_duration_millis =
            u64::try_from(self.lease_duration.as_millis()).map_err(|_| {
                ProcessJournalStoreError::InvalidRequest(
                    "process lease duration exceeds journal representation".to_string(),
                )
            })?;
        let claimed = match self
            .execute(StoredProcessCommand::Claim {
                request,
                now: Utc::now(),
                lease_duration_millis,
                lease_nonce: ProcessId::new(),
                limits: self.concurrency_limits.clone(),
            })
            .await?
        {
            StoredCommandOutcome::Claimed(claimed) => claimed,
            outcome => return Err(unexpected_outcome("claim", outcome)),
        };
        for process in &claimed {
            self.notify_process_commit(process.state.clone(), ProcessJournalKind::Claimed, None)
                .await;
        }
        Ok(claimed)
    }

    async fn heartbeat_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, Self::Error> {
        let lease_duration_millis =
            u64::try_from(self.lease_duration.as_millis()).map_err(|_| {
                ProcessJournalStoreError::InvalidRequest(
                    "process lease duration exceeds journal representation".to_string(),
                )
            })?;
        let snapshot = match self
            .execute(StoredProcessCommand::Heartbeat {
                request,
                now: Utc::now(),
                lease_duration_millis,
            })
            .await?
        {
            StoredCommandOutcome::Heartbeat(snapshot) => snapshot,
            outcome => return Err(unexpected_outcome("heartbeat", outcome)),
        };
        self.notify_process_commit(snapshot.clone(), ProcessJournalKind::Heartbeat, None)
            .await;
        Ok(snapshot.journal_cursor)
    }

    async fn recover_expired_process_leases(
        &self,
        request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
        let response = match self
            .execute(StoredProcessCommand::RecoverExpired(request))
            .await?
        {
            StoredCommandOutcome::Recovered(response) => response,
            outcome => return Err(unexpected_outcome("recover_expired", outcome)),
        };
        for snapshot in &response.recovered {
            let kind = match snapshot.status {
                ProcessLifecycleStatus::Queued => ProcessJournalKind::Resumed,
                ProcessLifecycleStatus::Cancelled => ProcessJournalKind::Cancelled,
                ProcessLifecycleStatus::Failed => ProcessJournalKind::Failed,
                _ => ProcessJournalKind::RecoveryRequired,
            };
            self.notify_process_commit(snapshot.clone(), kind, None)
                .await;
        }
        Ok(response)
    }

    async fn suspend_process(
        &self,
        request: SuspendProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.leased_transition(
            ProcessLeaseRequest {
                process_id: request.process_id,
                worker_id: request.worker_id,
                lease_token: request.lease_token,
            },
            ProcessTransitionMutation {
                status: ProcessLifecycleStatus::Suspended,
                kind: ProcessJournalKind::Suspended,
                suspension: Some(request.suspension),
                checkpoint_ref: Some(request.checkpoint_ref),
                failure: None,
                metadata: request.metadata,
            },
        )
        .await
    }

    async fn complete_process(
        &self,
        request: crate::ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.leased_transition(
            request.lease,
            ProcessTransitionMutation {
                metadata: request.metadata,
                ..ProcessTransitionMutation::new(
                    ProcessLifecycleStatus::Completed,
                    ProcessJournalKind::Completed,
                )
            },
        )
        .await
    }

    async fn cancel_process(
        &self,
        request: crate::ProcessStateTransitionRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.leased_transition(
            request.lease,
            ProcessTransitionMutation {
                metadata: request.metadata,
                ..ProcessTransitionMutation::new(
                    ProcessLifecycleStatus::Cancelled,
                    ProcessJournalKind::Cancelled,
                )
            },
        )
        .await
    }

    async fn fail_process(
        &self,
        request: FailProcessRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.leased_transition(
            ProcessLeaseRequest {
                process_id: request.process_id,
                worker_id: request.worker_id,
                lease_token: request.lease_token,
            },
            ProcessTransitionMutation {
                failure: Some(request.failure),
                metadata: request.metadata,
                ..ProcessTransitionMutation::new(
                    ProcessLifecycleStatus::Failed,
                    ProcessJournalKind::Failed,
                )
            },
        )
        .await
    }

    async fn relinquish_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        self.leased_transition(
            request,
            ProcessTransitionMutation::new(
                ProcessLifecycleStatus::Queued,
                ProcessJournalKind::Heartbeat,
            ),
        )
        .await
    }
}

#[async_trait]
impl<F> ProcessControlPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn resume_process(
        &self,
        request: ResumeProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.control_transition(ProcessControlMutation {
            scope: request.scope,
            process_id: request.process_id,
            action: ProcessControlAction::Resume,
            operation_id: request.operation_id,
            expected_cursor: request.expected_cursor,
            reason: None,
            checkpoint_ref: request.checkpoint_ref,
            metadata: request.metadata,
        })
        .await
    }

    async fn stop_process(
        &self,
        request: StopProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.control_transition(ProcessControlMutation {
            scope: request.scope,
            process_id: request.process_id,
            action: ProcessControlAction::Stop,
            operation_id: request.operation_id,
            expected_cursor: None,
            reason: request.reason,
            checkpoint_ref: None,
            metadata: None,
        })
        .await
    }

    async fn request_cancel_process(
        &self,
        request: CancelProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.control_transition(ProcessControlMutation {
            scope: request.scope,
            process_id: request.process_id,
            action: ProcessControlAction::Cancel,
            operation_id: request.operation_id,
            expected_cursor: None,
            reason: request.reason,
            checkpoint_ref: None,
            metadata: None,
        })
        .await
    }

    async fn kill_process(
        &self,
        request: KillProcessRequest,
    ) -> Result<ProcessControlResult, Self::Error> {
        self.control_transition(ProcessControlMutation {
            scope: request.scope,
            process_id: request.process_id,
            action: ProcessControlAction::Kill,
            operation_id: request.operation_id,
            expected_cursor: None,
            reason: request.reason,
            checkpoint_ref: None,
            metadata: None,
        })
        .await
    }
}

impl<F> ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    async fn control_transition(
        &self,
        mutation: ProcessControlMutation,
    ) -> Result<ProcessControlResult, ProcessJournalStoreError> {
        let reason = mutation.reason.clone();
        let (result, committed_kind) = match self
            .execute(StoredProcessCommand::Control(mutation))
            .await?
        {
            StoredCommandOutcome::Controlled(result, kind) => (result, kind),
            outcome => return Err(unexpected_outcome("control", outcome)),
        };
        if let Some(kind) = committed_kind {
            self.notify_process_commit(result.state.clone(), kind, reason)
                .await;
        }
        Ok(result)
    }

    async fn leased_transition(
        &self,
        request: ProcessLeaseRequest,
        mutation: ProcessTransitionMutation,
    ) -> Result<JournaledProcessSnapshot, ProcessJournalStoreError> {
        let kind = mutation.kind;
        let snapshot = match self
            .execute(StoredProcessCommand::LeasedTransition { request, mutation })
            .await?
        {
            StoredCommandOutcome::Transitioned(snapshot) => snapshot,
            outcome => return Err(unexpected_outcome("leased_transition", outcome)),
        };
        self.notify_process_commit(snapshot.clone(), kind, None)
            .await;
        Ok(snapshot)
    }
}

#[async_trait]
impl<F> ProcessTreePort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn child_processes(
        &self,
        scope: &ResourceScope,
        parent_process_id: ProcessId,
    ) -> Result<Vec<JournaledProcessSnapshot>, Self::Error> {
        let Some(parent) = self.load_process(parent_process_id).await? else {
            return Ok(Vec::new());
        };
        if !same_scope_owner(&parent.scope, scope) {
            return Ok(Vec::new());
        }
        let mut children = rows::child_processes(self.filesystem.as_ref(), parent_process_id)
            .await?
            .into_iter()
            .filter(|snapshot| same_lineage_scope(&snapshot.scope, scope))
            .collect::<Vec<_>>();
        children.sort_by_key(|snapshot| snapshot.created_at);
        Ok(children)
    }

    async fn reserve_process_tree(
        &self,
        request: ReserveProcessTreeRequest,
    ) -> Result<ProcessTreeReservation, Self::Error> {
        match self
            .execute(StoredProcessCommand::ReserveTree(request))
            .await?
        {
            StoredCommandOutcome::TreeReserved(reservation) => Ok(reservation),
            outcome => Err(unexpected_outcome("reserve_tree", outcome)),
        }
    }

    async fn release_process_tree(
        &self,
        request: ReleaseProcessTreeRequest,
    ) -> Result<(), Self::Error> {
        match self
            .execute(StoredProcessCommand::ReleaseTree(request))
            .await?
        {
            StoredCommandOutcome::TreeReleased => Ok(()),
            outcome => Err(unexpected_outcome("release_tree", outcome)),
        }
    }

    async fn prune_released_process(
        &self,
        request: PruneReleasedProcessRequest,
    ) -> Result<(), Self::Error> {
        match self
            .execute(StoredProcessCommand::PruneTree(request))
            .await?
        {
            StoredCommandOutcome::TreePruned => Ok(()),
            outcome => Err(unexpected_outcome("prune_tree", outcome)),
        }
    }
}

#[async_trait]
impl<F> ProcessDependencyPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn open_process_dependency(
        &self,
        request: OpenProcessDependencyRequest,
    ) -> Result<ProcessDependencyRecord, Self::Error> {
        match self
            .execute(StoredProcessCommand::OpenDependency(request))
            .await?
        {
            StoredCommandOutcome::Dependency(Some(record)) => Ok(record),
            StoredCommandOutcome::Dependency(None) => {
                Err(ProcessJournalStoreError::InvalidRequest(
                    "open dependency produced no record".to_string(),
                ))
            }
            outcome => Err(unexpected_outcome("open_dependency", outcome)),
        }
    }

    async fn settle_process_dependency(
        &self,
        request: SettleProcessDependencyRequest,
    ) -> Result<Option<ProcessDependencyRecord>, Self::Error> {
        match self
            .execute(StoredProcessCommand::SettleDependency(request))
            .await?
        {
            StoredCommandOutcome::Dependency(record) => Ok(record),
            outcome => Err(unexpected_outcome("settle_dependency", outcome)),
        }
    }

    async fn consume_process_dependency(
        &self,
        request: CloseProcessDependencyRequest,
    ) -> Result<Option<ProcessDependencyRecord>, Self::Error> {
        match self
            .execute(StoredProcessCommand::ConsumeDependency(request))
            .await?
        {
            StoredCommandOutcome::Dependency(record) => Ok(record),
            outcome => Err(unexpected_outcome("consume_dependency", outcome)),
        }
    }

    async fn abandon_process_dependency(
        &self,
        request: CloseProcessDependencyRequest,
    ) -> Result<Option<ProcessDependencyRecord>, Self::Error> {
        match self
            .execute(StoredProcessCommand::AbandonDependency(request))
            .await?
        {
            StoredCommandOutcome::Dependency(record) => Ok(record),
            outcome => Err(unexpected_outcome("abandon_dependency", outcome)),
        }
    }

    async fn query_process_dependencies(
        &self,
        request: ProcessDependencyQuery,
    ) -> Result<Vec<ProcessDependencyRecord>, Self::Error> {
        self.ensure_materialized().await?;
        let mut records = rows::dependencies_for_scope(
            self.filesystem.as_ref(),
            &request.scope,
            request.dependent_process_id,
        )
        .await?
        .into_iter()
        .filter(|record| {
            request
                .group_ref
                .as_ref()
                .is_none_or(|group_ref| record.group_ref.as_ref() == Some(group_ref))
        })
        .filter(|record| {
            request.include_closed
                || !matches!(
                    record.state,
                    crate::ProcessDependencyState::Consumed
                        | crate::ProcessDependencyState::Abandoned
                )
        })
        .collect::<Vec<_>>();
        records.sort_by_key(|record| {
            (
                record.dependent_process_id.as_uuid(),
                record.dependency_process_id.as_uuid(),
            )
        });
        Ok(records)
    }

    async fn unresolved_process_dependencies(
        &self,
    ) -> Result<Vec<ProcessDependencyRecord>, Self::Error> {
        self.ensure_materialized().await?;
        let mut records = rows::unresolved_dependencies(self.filesystem.as_ref())
            .await?
            .into_iter()
            .filter(|record| {
                !matches!(
                    record.state,
                    crate::ProcessDependencyState::Consumed
                        | crate::ProcessDependencyState::Abandoned
                )
            })
            .collect::<Vec<_>>();
        records.sort_by_key(|record| {
            (
                record.created_at,
                record.dependent_process_id.as_uuid(),
                record.dependency_process_id.as_uuid(),
            )
        });
        Ok(records)
    }
}

#[async_trait]
impl<F> ProcessCheckpointPort for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn record_process_checkpoint(
        &self,
        request: RecordProcessCheckpointRequest,
    ) -> Result<ProcessCheckpointRecord, Self::Error> {
        match self
            .execute(StoredProcessCommand::RecordCheckpoint(request))
            .await?
        {
            StoredCommandOutcome::Checkpointed(record) => Ok(record),
            outcome => Err(unexpected_outcome("record_checkpoint", outcome)),
        }
    }

    async fn get_process_checkpoint(
        &self,
        request: GetProcessCheckpointRequest,
    ) -> Result<Option<ProcessCheckpointRecord>, Self::Error> {
        self.ensure_materialized().await?;
        let path = rows::checkpoint_scoped_path(&request.checkpoint_id)?;
        let record = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await?
            .as_ref()
            .map(rows::decode_checkpoint)
            .transpose()?
            .flatten();
        Ok(record.filter(|record| {
            record.process_id == request.process_id
                && process_scope_visible(&record.scope, &request.scope)
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessTransitionMutation {
    status: ProcessLifecycleStatus,
    kind: ProcessJournalKind,
    suspension: Option<ProcessSuspension>,
    checkpoint_ref: Option<crate::ProcessCheckpointRef>,
    failure: Option<ironclaw_host_api::SanitizedFailure>,
    metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProcessControlMutation {
    scope: ResourceScope,
    process_id: ProcessId,
    action: ProcessControlAction,
    operation_id: Option<ProcessOperationId>,
    expected_cursor: Option<ProcessJournalCursor>,
    reason: Option<String>,
    checkpoint_ref: Option<crate::ProcessCheckpointRef>,
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProcessControlAction {
    Resume,
    Stop,
    Cancel,
    Kill,
}

impl ProcessControlAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Stop => "stop",
            Self::Cancel => "cancel",
            Self::Kill => "kill",
        }
    }
}

impl ProcessTransitionMutation {
    fn new(status: ProcessLifecycleStatus, kind: ProcessJournalKind) -> Self {
        Self {
            status,
            kind,
            suspension: None,
            checkpoint_ref: None,
            failure: None,
            metadata: None,
        }
    }
}

#[async_trait]
impl<F> ProcessJournalSource for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn get_process_snapshot(
        &self,
        request: GetProcessSnapshotRequest,
    ) -> Result<JournaledProcessSnapshot, Self::Error> {
        let snapshot = self
            .load_process(request.process_id)
            .await?
            .filter(|snapshot| process_scope_visible(&snapshot.scope, &request.scope))
            .ok_or(ProcessJournalStoreError::UnknownProcess {
                process_id: request.process_id,
            })?;
        Ok(snapshot)
    }

    async fn read_process_journal_after(
        &self,
        scope: &ResourceScope,
        owner_user_id: Option<&ironclaw_host_api::UserId>,
        after: Option<ProcessJournalCursor>,
        limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        self.read_journal_page(Some(scope), owner_user_id, after, limit)
            .await
    }

    async fn read_process_journal_log_after(
        &self,
        after: Option<ProcessJournalCursor>,
        limit: usize,
    ) -> Result<ProcessJournalPage, Self::Error> {
        self.read_journal_page(None, None, after, limit).await
    }
}

#[async_trait]
impl<F> ProcessLifecycleLookupSource for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn process_lifecycle_states(
        &self,
        request: ProcessLifecycleLookupBatchRequest,
    ) -> Vec<Result<ProcessLifecycleLookupResult, Self::Error>> {
        futures::future::join_all(request.processes.into_iter().map(|lookup| async move {
            let result = self
                .load_process(lookup.process_id)
                .await?
                .filter(|snapshot| snapshot.scope.tenant_id == lookup.tenant_id)
                .map(|snapshot| ProcessLifecycleLookupResult::Found {
                    status: snapshot.status,
                    suspension: snapshot.suspension,
                })
                .unwrap_or(ProcessLifecycleLookupResult::Missing);
            Ok(result)
        }))
        .await
    }
}

#[async_trait]
impl<F> ProcessGateQuerySource for ProcessJournalStore<F>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    type Error = ProcessJournalStoreError;

    async fn query_process_gates(
        &self,
        request: ProcessGateQuery,
    ) -> Result<Vec<ProcessGateRecord>, Self::Error> {
        self.ensure_materialized().await?;
        let owner_scope = matches!(
            request
                .scope_match
                .unwrap_or(crate::ProcessGateScopeMatch::Exact),
            crate::ProcessGateScopeMatch::Owner
        );
        let mut records =
            rows::gate_processes(self.filesystem.as_ref(), &request.scope, owner_scope)
                .await?
                .into_iter()
                .filter(|snapshot| process_gate_snapshot_matches(snapshot, &request))
                .filter_map(|snapshot| {
                    Some(ProcessGateRecord {
                        process_id: snapshot.process_id,
                        scope: snapshot.scope.clone(),
                        owner_user_id: snapshot.owner_user_id.clone(),
                        suspension: snapshot.suspension.clone()?,
                        resume_source_ref: snapshot
                            .metadata
                            .pointer("/agent_turn/source_binding_ref")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        reply_target_ref: snapshot
                            .metadata
                            .pointer("/agent_turn/reply_target_binding_ref")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        historical: false,
                    })
                })
                .collect::<Vec<_>>();
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

impl<F> ProcessRuntimePort for ProcessJournalStore<F> where F: RootFilesystem + Send + Sync + 'static
{}

impl ProcessJournalEntry {
    fn from_snapshot(
        snapshot: &JournaledProcessSnapshot,
        cursor: ProcessJournalCursor,
        kind: ProcessJournalKind,
    ) -> Self {
        Self {
            cursor,
            process_id: snapshot.process_id,
            process_kind: snapshot.process_kind.clone(),
            scope: snapshot.scope.clone(),
            occurred_at: Some(Utc::now()),
            owner_user_id: snapshot.owner_user_id.clone(),
            status: snapshot.status,
            kind,
            suspension: snapshot.suspension.clone(),
            sanitized_reason: None,
            retryable: None,
            detail: None,
            metadata: snapshot.metadata.clone(),
            committed_state: Some(Box::new(snapshot.clone())),
        }
    }
}

fn legacy_command_log_path() -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new(LEGACY_COMMAND_LOG_PATH)
        .map_err(|error| ProcessJournalStoreError::InvalidPath(invalid_path(error).to_string()))
}

fn processes_prefix() -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new("/processes")
        .map_err(|error| ProcessJournalStoreError::InvalidPath(invalid_path(error).to_string()))
}

fn legacy_journal_state_path() -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new(LEGACY_JOURNAL_STATE_PATH)
        .map_err(|error| ProcessJournalStoreError::InvalidPath(invalid_path(error).to_string()))
}

fn process_journal_sequence_path() -> Result<ScopedPath, ProcessJournalStoreError> {
    ScopedPath::new("/processes/materialized/journal-sequence")
        .map_err(|error| ProcessJournalStoreError::InvalidPath(error.to_string()))
}

fn unexpected_outcome(operation: &str, outcome: StoredCommandOutcome) -> ProcessJournalStoreError {
    ProcessJournalStoreError::Deserialization(format!(
        "process journal {operation} produced unexpected outcome {outcome:?}"
    ))
}
