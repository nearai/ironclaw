// arch-exempt: large_file, bounded transaction retry handling stays with the row-native journal transaction owner, plan #6696
use std::{
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, FilesystemError, Filter, Page, RootFilesystem, ScopedFilesystem, SeqNo,
};
use ironclaw_host_api::{ids::ProcessId, path::ScopedPath, resource::ResourceScope};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{Mutex, OnceCell};

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
mod flusher;
mod migration;
mod observer;
mod rows;
mod state;

pub use state::MAX_CRASH_RECOVERY_RECLAIMS;
mod validation;
use command::StoredProcessCommand;
use migration::{
    import_deployed_legacy_authorities, legacy_run_state_records, legacy_turn_record_contains_data,
};
use observer::RegisteredProcessObserver;
use state::ProcessJournalMaterializedState;
use validation::{
    ensure_lease, ensure_transition, process_claim_within_limits, process_gate_snapshot_matches,
    process_scope_visible, same_lineage_scope, validate_tree_root,
};

const LEGACY_COMMAND_LOG_PATH: &str = "/processes/journal/records";
const LEGACY_JOURNAL_STATE_PATH: &str = "/processes/journal/state.json";
pub const DEFAULT_PROCESS_LEASE_DURATION: Duration = Duration::from_secs(90);
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
    /// A group commit failed as a whole and was deliberately not replayed.
    ///
    /// Nothing in the batch committed. Replaying to attribute the failure
    /// would let a command whose write was meant to fail succeed on the second
    /// attempt, so every command in the batch observes this instead.
    #[error("process journal group commit failed and was not replayed: {reason}")]
    GroupCommitFailed { reason: String },
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
    Controlled(ProcessControlResult),
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
    /// Group-commit funnel, started on the first write so `new` stays sync.
    flusher: Arc<OnceCell<flusher::FlusherHandle>>,
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
            flusher: Arc::clone(&self.flusher),
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
            flusher: Arc::new(OnceCell::new()),
            lease_duration: DEFAULT_PROCESS_LEASE_DURATION,
            concurrency_limits: ProcessConcurrencyLimits::default(),
        }
    }

    /// A handle whose funnel slot is empty.
    ///
    /// The flusher and delivery tasks own a store handle. Sharing the funnel's
    /// `OnceCell` with them would make the command sender reachable from the
    /// task that reads it, so the channel would never close and the tasks would
    /// outlive every real store handle. Detached handles are used only for
    /// reads and observer replay, never to submit commands.
    fn detached(&self) -> Self {
        Self {
            flusher: Arc::new(OnceCell::new()),
            ..self.clone()
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

    /// The configured lease TTL in the journal's millisecond representation.
    ///
    /// Claim, heartbeat, and expiry recovery all carry the same TTL into the
    /// journal command, so they share one conversion and one rejection.
    fn lease_duration_millis(&self) -> Result<u64, ProcessJournalStoreError> {
        u64::try_from(self.lease_duration.as_millis()).map_err(|_| {
            ProcessJournalStoreError::InvalidRequest(
                "process lease duration exceeds journal representation".to_string(),
            )
        })
    }

    async fn submit_process_inner(
        &self,
        request: SubmitProcessRequest,
    ) -> Result<(JournaledProcessSnapshot, bool), ProcessJournalStoreError>
    where
        F: Send + Sync + 'static,
    {
        match self
            .execute(StoredProcessCommand::Submit(Box::new(request)))
            .await?
        {
            StoredCommandOutcome::Submitted(snapshot, changed) => Ok((snapshot, changed)),
            outcome => Err(unexpected_outcome("submit", outcome)),
        }
    }

    /// Submit one lifecycle command to the group-commit funnel and wait for its
    /// durable outcome.
    ///
    /// The funnel batches concurrent commands into one transaction; see
    /// [`flusher`] for why the write path and observer delivery run on separate
    /// tasks.
    async fn execute(
        &self,
        command: StoredProcessCommand,
    ) -> Result<StoredCommandOutcome, ProcessJournalStoreError>
    where
        F: Send + Sync + 'static,
    {
        self.ensure_materialized().await?;
        let (responder, response) = tokio::sync::oneshot::channel();
        let queued = flusher::QueuedCommand {
            command,
            responder,
            awaits_observer_delivery: !observer::inside_observer_delivery(),
        };
        if self.flusher().await.submit(queued).await.is_err() {
            return Err(flusher::backend_busy(self));
        }
        response
            .await
            .unwrap_or_else(|_| Err(flusher::backend_busy(self)))
    }

    async fn flusher(&self) -> &flusher::FlusherHandle
    where
        F: Send + Sync + 'static,
    {
        self.flusher
            .get_or_init(|| async { flusher::spawn(self.detached()) })
            .await
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
        owner_user_id: Option<&ironclaw_host_api::ids::UserId>,
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
        let import_legacy = self.legacy_process_journal_present().await?;
        self.initialize_materialized(import_legacy).await?;
        self.materialized_ready.store(true, Ordering::Release);
        Ok(())
    }

    /// Initialize the row-native journal and import any legacy authority.
    ///
    /// Production composition invokes this during startup so migration failure
    /// fails startup closed instead of surfacing inside the first user request.
    pub async fn migrate_legacy_journal(&self) -> Result<usize, ProcessJournalStoreError> {
        let _guard = self.migration.lock().await;
        rows::ensure_indexes(self.filesystem.as_ref()).await?;
        if rows::is_initialized(self.filesystem.as_ref()).await? {
            self.materialized_ready.store(true, Ordering::Release);
            return Ok(0);
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
        Ok(!legacy_run_state_records(self.filesystem.as_ref())
            .await?
            .is_empty())
    }

    async fn initialize_materialized(
        &self,
        import_legacy: bool,
    ) -> Result<usize, ProcessJournalStoreError> {
        'transaction: for attempt in 0..MAX_TRANSACTION_RETRIES {
            let mut loaded = match rows::load(
                self.filesystem.as_ref(),
                &rows::LoadReferences::default(),
            )
            .await
            {
                Ok(loaded) => loaded,
                Err(ProcessJournalStoreError::Filesystem(error))
                    if rows::retryable_transaction_error(&error) =>
                {
                    rows::retry_transaction(attempt).await;
                    continue;
                }
                Err(error) => return Err(error),
            };
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
                import_deployed_legacy_authorities(self.filesystem.as_ref(), &mut state).await?;
            }
            let entries = std::mem::take(&mut state.journal);
            let imported = entries.len();
            let prefix = processes_prefix()?;
            let mut txn = match self
                .filesystem
                .begin(&ResourceScope::system(), &prefix)
                .await
            {
                Ok(txn) => txn,
                Err(error) if rows::retryable_transaction_error(&error) => {
                    rows::retry_transaction(attempt).await;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            let result = async {
                if import_legacy {
                    let sequence_path = self
                        .filesystem
                        .resolve(&ResourceScope::system(), &process_journal_sequence_path()?)?;
                    if let Err(error) = txn
                        .reserve_sequence_range(&sequence_path, state.next_cursor.saturating_sub(1))
                        .await
                    {
                        if rows::retryable_transaction_error(&error) {
                            return Err(ProcessJournalStoreError::Filesystem(error));
                        }
                        return Err(error.into());
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
                    continue 'transaction;
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
        // Observer delivery is driven from the journal by the group-commit
        // funnel, so it happens before this call returns without the wrapper
        // replaying each outcome.
        let (snapshot, _changed) = self.submit_process_inner(request).await?;
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
        let StoredCommandOutcome::Submitted(snapshot, _changed) = outcome else {
            return Err(unexpected_outcome(
                "submit_process_with_checkpoint",
                outcome,
            ));
        };
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
        let lease_duration_millis = self.lease_duration_millis()?;
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
        Ok(claimed)
    }

    async fn heartbeat_process(
        &self,
        request: ProcessLeaseRequest,
    ) -> Result<ProcessJournalCursor, Self::Error> {
        let lease_duration_millis = self.lease_duration_millis()?;
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
        Ok(snapshot.journal_cursor)
    }

    async fn recover_expired_process_leases(
        &self,
        request: RecoverExpiredProcessLeasesRequest,
    ) -> Result<RecoverExpiredProcessLeasesResponse, Self::Error> {
        let lease_duration_millis = self.lease_duration_millis()?;
        let response = match self
            .execute(StoredProcessCommand::RecoverExpired {
                request,
                lease_duration_millis,
            })
            .await?
        {
            StoredCommandOutcome::Recovered(response) => response,
            outcome => return Err(unexpected_outcome("recover_expired", outcome)),
        };
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
                failure_recovery: crate::ProcessFailureRecovery::Terminal,
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
                failure_recovery: request.recovery,
                checkpoint_ref: request.checkpoint_ref,
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
        match self
            .execute(StoredProcessCommand::Control(mutation))
            .await?
        {
            // Observer delivery derives kinds from the committed journal
            // entries, so the control outcome carries only its result.
            StoredCommandOutcome::Controlled(result) => Ok(result),
            outcome => Err(unexpected_outcome("control", outcome)),
        }
    }

    async fn leased_transition(
        &self,
        request: ProcessLeaseRequest,
        mutation: ProcessTransitionMutation,
    ) -> Result<JournaledProcessSnapshot, ProcessJournalStoreError> {
        let snapshot = match self
            .execute(StoredProcessCommand::LeasedTransition { request, mutation })
            .await?
        {
            StoredCommandOutcome::Transitioned(snapshot) => snapshot,
            outcome => return Err(unexpected_outcome("leased_transition", outcome)),
        };
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
    failure: Option<ironclaw_host_api::turn::SanitizedFailure>,
    #[serde(default)]
    failure_recovery: crate::ProcessFailureRecovery,
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
            failure_recovery: crate::ProcessFailureRecovery::Terminal,
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
        owner_user_id: Option<&ironclaw_host_api::ids::UserId>,
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
        let failure = (kind == ProcessJournalKind::Failed)
            .then_some(snapshot.failure.as_ref())
            .flatten();
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
            sanitized_reason: failure.map(|failure| failure.category().to_string()),
            retryable: failure.map(|_| snapshot.checkpoint_ref.is_some()),
            detail: failure.and_then(|failure| failure.detail().map(str::to_string)),
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
