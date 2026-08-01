// arch-exempt: large_file, process journal persistence invariants stay in one caller-level contract suite, plan #5274
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, DiskFilesystem, Entry, Fault, FaultInjecting, FaultKind, FilesystemError,
    FilesystemOperation, Filter, InMemoryBackend, IndexKey, LibSqlRootFilesystem, Page,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProcessId, ProjectId, TenantId, ThreadId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{HostPath, MountAlias, ScopedPath, VirtualPath},
    resource::ResourceScope,
    turn::{SanitizedFailure, TurnCheckpointId, TurnGateRef, TurnId, TurnRunId},
};
use ironclaw_processes::{
    CancelProcessRequest, ClaimProcessesRequest, CloseProcessDependencyRequest, FailProcessRequest,
    GetProcessCheckpointRequest, GetProcessInputRequest, GetProcessSnapshotRequest,
    JournaledProcessSnapshot, KillProcessRequest, MAX_CRASH_RECOVERY_RECLAIMS,
    MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES, MAX_PROCESS_INPUT_PAYLOAD_BYTES,
    OpenProcessDependencyRequest, ProcessCheckpointId, ProcessCheckpointPayload,
    ProcessCheckpointPort, ProcessCheckpointRef, ProcessConcurrencyClass, ProcessConcurrencyLimits,
    ProcessControlPort, ProcessDependencyPort, ProcessDependencyQuery, ProcessDependencyState,
    ProcessDependencySubmission, ProcessFailureRecovery, ProcessGateOwnerMatch, ProcessGateQuery,
    ProcessGateQuerySource, ProcessGateScopeMatch, ProcessInputPayload, ProcessInputPort,
    ProcessInputRef, ProcessInputSubmission, ProcessJournalCommit, ProcessJournalCommitObserver,
    ProcessJournalCursor, ProcessJournalEntry, ProcessJournalError, ProcessJournalKind,
    ProcessJournalObserverRegistry, ProcessJournalSource, ProcessJournalStore,
    ProcessJournalStoreError, ProcessKind, ProcessLeaseRequest, ProcessLeaseToken,
    ProcessLifecycleLookupBatchRequest, ProcessLifecycleLookupRequest,
    ProcessLifecycleLookupResult, ProcessLifecycleLookupSource, ProcessLifecycleStatus,
    ProcessOperationId, ProcessSnapshotSource, ProcessStateTransitionRequest,
    ProcessSubmissionPort, ProcessSuspension, ProcessSuspensionKind, ProcessTerminalEvidence,
    ProcessTransitionPort, ProcessTreePort, ProcessWorkerId, PruneReleasedProcessRequest,
    RecordProcessCheckpointRequest, ReleaseProcessTreeRequest, ReserveProcessTreeRequest,
    ResumeProcessRequest, SettleProcessDependencyRequest, StopProcessRequest, SubmitProcessRequest,
    SuspendProcessRequest,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct RecordingProcessObserver {
    commits: Mutex<Vec<ProcessJournalCommit>>,
}

#[async_trait]
impl ProcessJournalCommitObserver for RecordingProcessObserver {
    fn process_observer_id(&self) -> &'static str {
        "recording-process-observer"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        self.commits
            .lock()
            .map_err(|_| "observer mutex poisoned".to_string())?
            .push(commit);
        Ok(())
    }
}

/// Recording observer whose callback suspends before it records.
///
/// Delivery therefore cannot complete within the flusher's own scheduling slot,
/// so a store that answered its callers before delivery finished would let them
/// observe a commit the observer has not seen yet.
#[derive(Default)]
struct SuspendingProcessObserver {
    commits: Mutex<Vec<ProcessJournalCommit>>,
    /// Size of every batch this observer was handed, in delivery order.
    batch_sizes: Mutex<Vec<usize>>,
}

#[async_trait]
impl ProcessJournalCommitObserver for SuspendingProcessObserver {
    fn process_observer_id(&self) -> &'static str {
        "suspending-process-observer"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        tokio::task::yield_now().await;
        self.commits
            .lock()
            .map_err(|_| "observer mutex poisoned".to_string())?
            .push(commit);
        Ok(())
    }

    async fn observe_process_commits(
        &self,
        commits: Vec<ProcessJournalCommit>,
    ) -> Result<(), String> {
        self.batch_sizes
            .lock()
            .map_err(|_| "observer mutex poisoned".to_string())?
            .push(commits.len());
        for commit in commits {
            self.observe_process_commit(commit).await?;
        }
        Ok(())
    }
}

struct FailingProcessObserver;

#[async_trait]
impl ProcessJournalCommitObserver for FailingProcessObserver {
    fn process_observer_id(&self) -> &'static str {
        "failing-process-observer"
    }

    async fn observe_process_commit(&self, _commit: ProcessJournalCommit) -> Result<(), String> {
        Err("deterministic observer failure".to_string())
    }
}

struct FailOnceProcessObserver {
    attempts: AtomicUsize,
}

#[async_trait]
impl ProcessJournalCommitObserver for FailOnceProcessObserver {
    fn process_observer_id(&self) -> &'static str {
        "fail-once-process-observer"
    }

    async fn observe_process_commit(&self, _commit: ProcessJournalCommit) -> Result<(), String> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            Err("transient observer failure".to_string())
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn process_journal_fails_closed_when_backend_lacks_multi_key_records() {
    let storage = tempfile::tempdir().expect("temporary process journal directory");
    let mut backend = DiskFilesystem::new();
    backend
        .mount_local(
            VirtualPath::new("/engine").expect("engine path"),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .expect("mount process journal directory");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("mount alias"),
        VirtualPath::new("/engine/processes").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let store = ProcessJournalStore::new(Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(backend),
        mounts,
    )));
    let scope = scope();

    let error = store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::new(),
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect_err("journal must require queryable multi-key records");
    assert!(matches!(
        error,
        ironclaw_processes::ProcessJournalStoreError::Filesystem(
            FilesystemError::Unsupported { .. }
        )
    ));
}

#[tokio::test]
async fn process_journal_rows_serialize_concurrent_store_handles() {
    let filesystem = in_memory_backed_processes_filesystem();
    let first = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let second = Arc::new(ProcessJournalStore::new(filesystem));
    let scope = scope();
    let request = |process_id, exclusive_within_scope| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::Internal,
        scope: scope.clone(),
        exclusive_within_scope,
        operation_id: None,
        owner_user_id: Some(scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };

    let (first_result, second_result) = tokio::join!(
        first.submit_process(request(ProcessId::new(), false)),
        second.submit_process(request(ProcessId::new(), false)),
    );
    first_result.expect("first concurrent submission");
    second_result.expect("second concurrent submission");
    let page = first
        .read_process_journal_log_after(None, 10)
        .await
        .expect("read concurrent journal");
    assert_eq!(page.entries.len(), 2);

    let exclusive_scope = ResourceScope {
        thread_id: Some(ThreadId::new("exclusive-thread").expect("exclusive thread")),
        ..scope
    };
    let exclusive_request = |process_id| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::AgentTurn,
        scope: exclusive_scope.clone(),
        exclusive_within_scope: true,
        operation_id: None,
        owner_user_id: Some(exclusive_scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };
    let (first_result, second_result) = tokio::join!(
        first.submit_process(exclusive_request(ProcessId::new())),
        second.submit_process(exclusive_request(ProcessId::new())),
    );
    assert_ne!(first_result.is_ok(), second_result.is_ok());
}

/// A one-shot backend write fault must be observed by a caller.
///
/// Group commit made this a durability question: when the batch transaction
/// fails non-retryably, replaying its commands individually re-runs their
/// externally observable semantics against a backend whose fault the aborted
/// batch already consumed. The command whose write was supposed to fail then
/// succeeds on the replay, and its caller is told a durable state was reached
/// that never was. The batch therefore fails as a whole instead of replaying.
#[tokio::test]
async fn group_commit_does_not_replay_a_consumed_one_shot_write_fault() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("process alias"),
            VirtualPath::new("/engine/processes").expect("process target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("process mount"),
    ));
    let store = ProcessJournalStore::new(filesystem);
    let resource_scope = scope();
    // Materialize the store and its funnel before arming, so the fault lands
    // on a batch of real submissions rather than on startup bookkeeping.
    submit_internal_process(&store, &resource_scope, ProcessId::new()).await;

    // A non-retryable write failure somewhere inside the next batch.
    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .nth(1)
            .backend("one-shot journal write failure"),
    );

    let process_ids = (0..16).map(|_| ProcessId::new()).collect::<Vec<_>>();
    let submissions = process_ids.iter().map(|process_id| {
        let store = &store;
        let scope = resource_scope.clone();
        let process_id = *process_id;
        async move {
            store
                .submit_process(SubmitProcessRequest {
                    process_id,
                    process_kind: ProcessKind::Internal,
                    scope,
                    exclusive_within_scope: false,
                    operation_id: None,
                    owner_user_id: None,
                    concurrency_class: None,
                    parent_process_id: None,
                    root_process_id: None,
                    spawn_tree_descendant_cap: None,
                    dependency: None,
                    checkpoint_ref: None,
                    input: None,
                    created_at: Utc::now(),
                    metadata: serde_json::Value::Null,
                })
                .await
        }
    });
    let outcomes = futures::future::join_all(submissions).await;

    assert!(
        outcomes.iter().any(|outcome| outcome.is_err()),
        "an armed one-shot write fault must surface to a caller; replaying the \
         batch would consume it silently and report durable state that was \
         never committed"
    );
    // Re-submitting a failed command is the caller's own decision and must
    // still work: the batch rolled back, so nothing conflicts.
    let failed = process_ids
        .iter()
        .zip(outcomes.iter())
        .filter_map(|(process_id, outcome)| outcome.as_ref().err().map(|_| *process_id))
        .collect::<Vec<_>>();
    for process_id in failed {
        submit_internal_process(&store, &resource_scope, process_id).await;
    }
}

#[tokio::test]
async fn process_journal_retries_transient_transaction_setup_contention() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("process alias"),
            VirtualPath::new("/engine/processes").expect("process target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("process mount"),
    ));
    let store = ProcessJournalStore::new(filesystem);
    let resource_scope = scope();
    submit_internal_process(&store, &resource_scope, ProcessId::new()).await;

    for operation in [
        FilesystemOperation::ReadFile,
        FilesystemOperation::BeginTxn,
        FilesystemOperation::ReserveSeq,
    ] {
        backend.add_fault(
            Fault::on(operation)
                .nth(1)
                .returning(FaultKind::BackendBusy),
        );
        submit_internal_process(&store, &resource_scope, ProcessId::new()).await;
    }
}

#[tokio::test]
async fn each_process_lifecycle_event_is_an_individual_libsql_row() {
    let storage = tempfile::tempdir().expect("temporary process journal database");
    let database_path = storage.path().join("process-journal.db");
    let database = Arc::new(
        libsql::Builder::new_local(&database_path)
            .build()
            .await
            .expect("build libsql database"),
    );
    let backend = Arc::new(
        LibSqlRootFilesystem::new(Arc::clone(&database)).expect("libSQL filesystem runtime"),
    );
    backend
        .run_migrations()
        .await
        .expect("migrate libsql filesystem");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("mount alias"),
            VirtualPath::new("/engine/processes").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view"),
    ));
    let store = Arc::new(ProcessJournalStore::new(Arc::clone(&filesystem)));
    let scope = scope();

    let submissions = (0..32).map(|_| {
        let store = Arc::clone(&store);
        let scope = scope.clone();
        async move { submit_internal_process(&store, &scope, ProcessId::new()).await }
    });
    let snapshots = futures::future::join_all(submissions).await;
    assert_eq!(snapshots.len(), 32);
    store
        .record_process_checkpoint(RecordProcessCheckpointRequest {
            checkpoint_id: ProcessCheckpointId::from_trusted("row-checkpoint"),
            process_id: snapshots[0].process_id,
            scope: scope.clone(),
            state_ref: ProcessCheckpointRef::from_trusted("row-state"),
            payload: ProcessCheckpointPayload::new(b"atomic-payload".to_vec())
                .expect("bounded payload"),
            created_at: Utc::now(),
            link_to_process: true,
            metadata: json!({"schema": "agent-loop-v1"}),
        })
        .await
        .expect("record checkpoint row");

    let page = store
        .read_process_journal_log_after(None, 64)
        .await
        .expect("read process journal");
    assert_eq!(
        page.entries.len(),
        32,
        "checkpoint payloads stay out of the lifecycle event projection"
    );

    let records = filesystem
        .query(
            &ResourceScope::system(),
            &ScopedPath::new("/processes/materialized/journal").expect("journal path"),
            &Filter::All,
            Page::default(),
        )
        .await
        .expect("query journal records");
    assert_eq!(records.len(), 32);

    for record in records {
        let parsed: serde_json::Value =
            serde_json::from_slice(&record.entry.body).expect("journal row is JSON");
        assert_eq!(parsed["row_type"], "journal");
    }

    let connection = database.connect().expect("connect to libsql");
    let mut rows = connection
        .query(
            "SELECT COUNT(*) FROM root_filesystem_entries WHERE path LIKE ?1",
            libsql::params!["/engine/processes/materialized/journal/%"],
        )
        .await
        .expect("count journal rows");
    let row = rows
        .next()
        .await
        .expect("read count row")
        .expect("count row exists");
    let count: i64 = row.get(0).expect("read count");
    assert_eq!(count, 32);
}

#[tokio::test]
async fn checkpointless_failure_redrive_is_bounded_and_durable_on_libsql() {
    let storage = tempfile::tempdir().expect("temporary process journal database");
    let database = Arc::new(
        libsql::Builder::new_local(storage.path().join("failure-redrive.db"))
            .build()
            .await
            .expect("build libsql database"),
    );
    let backend = Arc::new(LibSqlRootFilesystem::new(database).expect("libSQL filesystem runtime"));
    backend
        .run_migrations()
        .await
        .expect("migrate libsql filesystem");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        backend,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("mount alias"),
            VirtualPath::new("/engine/processes").expect("virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view"),
    ));
    let request_scope = scope();
    let process_id = ProcessId::new();
    let failure = SanitizedFailure::new("host_stage_unavailable_prompt")
        .expect("failure")
        .with_detail("safe prompt construction detail");
    let mut store = ProcessJournalStore::new(Arc::clone(&filesystem));
    submit_internal_process(&store, &request_scope, process_id).await;

    for claim_count in 1..=MAX_CRASH_RECOVERY_RECLAIMS {
        let worker_id = ProcessWorkerId::from_trusted(format!("redrive-worker-{claim_count}"));
        let claim = store
            .claim_next_processes(ClaimProcessesRequest {
                worker_id,
                scope_filter: Some(request_scope.clone()),
                process_id_filter: Some(process_id),
                process_kind_filter: Some(ProcessKind::Internal),
                max_processes: 1,
            })
            .await
            .expect("claim checkpointless process")
            .pop()
            .expect("checkpointless process remains claimable until exhausted");
        assert_eq!(
            claim
                .state
                .lease
                .as_ref()
                .expect("claimed process has lease")
                .claim_count,
            claim_count
        );
        let state = store
            .fail_process(FailProcessRequest {
                process_id,
                worker_id: claim.worker_id,
                lease_token: claim.lease_token,
                failure: failure.clone(),
                recovery: ProcessFailureRecovery::RedriveIfCheckpointless,
                checkpoint_ref: None,
                metadata: None,
            })
            .await
            .expect("record checkpointless runner failure");

        if claim_count < MAX_CRASH_RECOVERY_RECLAIMS {
            assert_eq!(state.status, ProcessLifecycleStatus::Queued);
            assert_eq!(state.crash_reclaim_count, claim_count);
            assert_eq!(state.failure, None);
            drop(store);
            store = ProcessJournalStore::new(Arc::clone(&filesystem));
            let reopened = store
                .get_process_snapshot(GetProcessSnapshotRequest {
                    scope: request_scope.clone(),
                    process_id,
                })
                .await
                .expect("load reopened process");
            assert_eq!(reopened.status, ProcessLifecycleStatus::Queued);
            assert_eq!(reopened.crash_reclaim_count, claim_count);
        } else {
            assert_eq!(state.status, ProcessLifecycleStatus::Failed);
            assert_eq!(state.failure, Some(failure.clone()));
            assert!(state.lease.is_none());
        }
    }

    let checkpointed_id = ProcessId::new();
    let checkpoint_ref = ProcessCheckpointRef::from_trusted("before-model-checkpoint");
    store
        .submit_process(SubmitProcessRequest {
            process_id: checkpointed_id,
            process_kind: ProcessKind::Internal,
            scope: request_scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(request_scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: Some(checkpoint_ref.clone()),
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit checkpointed process");
    let checkpointed_claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("checkpointed-worker"),
            scope_filter: Some(request_scope),
            process_id_filter: Some(checkpointed_id),
            process_kind_filter: Some(ProcessKind::Internal),
            max_processes: 1,
        })
        .await
        .expect("claim checkpointed process")
        .pop()
        .expect("checkpointed process claimed");
    let checkpointed = store
        .fail_process(FailProcessRequest {
            process_id: checkpointed_id,
            worker_id: checkpointed_claim.worker_id,
            lease_token: checkpointed_claim.lease_token,
            failure: failure.clone(),
            recovery: ProcessFailureRecovery::RedriveIfCheckpointless,
            checkpoint_ref: Some(checkpoint_ref),
            metadata: None,
        })
        .await
        .expect("checkpointed failure is terminal");
    assert_eq!(checkpointed.status, ProcessLifecycleStatus::Failed);
    assert_eq!(checkpointed.failure, Some(failure));
}

#[tokio::test]
async fn process_journal_pages_database_rows_beyond_backend_page_limit() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(filesystem);
    let scope = scope();
    for _ in 0..1_030 {
        submit_internal_process(&store, &scope, ProcessId::new()).await;
    }

    let page = store
        .read_process_journal_log_after(Some(ProcessJournalCursor(1_020)), 5)
        .await
        .expect("read bounded journal page");

    assert_eq!(page.entries.len(), 5);
    assert_eq!(page.entries[0].cursor, ProcessJournalCursor(1_021));
    assert_eq!(page.next_cursor, ProcessJournalCursor(1_025));
    assert!(page.truncated);
    assert_eq!(
        store
            .process_snapshots(&scope)
            .await
            .expect("scope query paginates beyond one backend page")
            .len(),
        1_030
    );
}

#[tokio::test]
async fn empty_claim_does_not_consume_process_journal_cursors() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());

    for _ in 0..3 {
        let claimed = store
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: ProcessWorkerId::from_trusted("idle-worker"),
                scope_filter: None,
                process_id_filter: None,
                process_kind_filter: Some(ProcessKind::AgentTurn),
                max_processes: 128,
            })
            .await
            .expect("empty claim succeeds");
        assert!(claimed.is_empty());
    }

    let submitted = submit_internal_process(&store, &scope(), ProcessId::new()).await;
    assert_eq!(submitted.journal_cursor, ProcessJournalCursor(1));
}

#[tokio::test]
async fn explicit_legacy_materialized_state_imports_before_row_native_commands() {
    let filesystem = in_memory_backed_processes_filesystem();
    let scope = scope();
    let process_id = ProcessId::new();
    let cursor = ProcessJournalCursor(1);
    let snapshot = JournaledProcessSnapshot {
        process_id,
        process_kind: ProcessKind::Internal,
        scope: scope.clone(),
        status: ProcessLifecycleStatus::Queued,
        suspension: None,
        checkpoint_ref: None,
        input_ref: None,
        failure: None,
        journal_cursor: cursor,
        lease: None,
        crash_reclaim_count: 0,
        created_at: Utc::now(),
        owner_user_id: Some(scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        metadata: serde_json::Value::Null,
    };
    let entry = ProcessJournalEntry {
        cursor,
        process_id,
        process_kind: ProcessKind::Internal,
        scope: scope.clone(),
        occurred_at: Some(snapshot.created_at),
        owner_user_id: snapshot.owner_user_id.clone(),
        status: ProcessLifecycleStatus::Queued,
        kind: ProcessJournalKind::Submitted,
        suspension: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
        metadata: serde_json::Value::Null,
        committed_state: Some(Box::new(snapshot.clone())),
    };
    let legacy = json!({
        "next_cursor": 2,
        "processes": { process_id.to_string(): snapshot },
        "journal": [entry]
    });
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new("/processes/journal/state.json").expect("legacy path"),
            Entry::bytes(serde_json::to_vec(&legacy).expect("serialize legacy state")),
            CasExpectation::Absent,
        )
        .await
        .expect("seed legacy state");

    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    assert_eq!(
        store
            .migrate_legacy_journal()
            .await
            .expect("explicit legacy migration"),
        1
    );
    let imported = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
        .expect("read imported process");
    assert_eq!(imported.journal_cursor, cursor);
    let next = submit_internal_process(&store, &scope, ProcessId::new()).await;
    assert_eq!(next.journal_cursor, ProcessJournalCursor(2));

    let records = store
        .read_process_journal_log_after(None, 10)
        .await
        .expect("read imported journal");
    assert_eq!(records.entries.len(), 2);
}

#[tokio::test]
async fn normal_process_request_runs_pre_start_migration_and_rejects_malformed_legacy_state() {
    let filesystem = in_memory_backed_processes_filesystem();
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new("/processes/journal/state.json").expect("legacy path"),
            Entry::bytes(b"not valid legacy json".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed malformed legacy state");

    let store = ProcessJournalStore::new(filesystem);
    let scope = scope();
    let error = store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::new(),
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect_err("pre-start migration must reject malformed legacy state");
    assert!(matches!(
        error,
        ProcessJournalStoreError::Deserialization(_)
    ));
}

#[tokio::test]
async fn malformed_legacy_journal_fails_without_initializing_row_native_state() {
    for legacy_path in [
        "/processes/journal/state.json",
        "/processes/journal/records",
    ] {
        let filesystem = in_memory_backed_processes_filesystem();
        let path = ScopedPath::new(legacy_path).expect("legacy path");
        if legacy_path.ends_with("state.json") {
            filesystem
                .put(
                    &ResourceScope::system(),
                    &path,
                    Entry::bytes(b"not valid legacy json".to_vec()),
                    CasExpectation::Absent,
                )
                .await
                .expect("seed malformed legacy state");
        } else {
            filesystem
                .append(
                    &ResourceScope::system(),
                    &path,
                    b"not valid legacy command".to_vec(),
                )
                .await
                .expect("seed malformed legacy command");
        }
        let store = ProcessJournalStore::new(Arc::clone(&filesystem));

        let error = store
            .migrate_legacy_journal()
            .await
            .expect_err("malformed legacy input must fail");
        assert!(matches!(
            error,
            ProcessJournalStoreError::Deserialization(_)
        ));
        assert!(
            filesystem
                .get(
                    &ResourceScope::system(),
                    &ScopedPath::new("/processes/materialized/metadata").expect("metadata path"),
                )
                .await
                .expect("read metadata")
                .is_none()
        );
    }
}

#[tokio::test]
async fn normal_process_request_cannot_hide_deployed_turn_authority() {
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/processes").expect("processes alias"),
            VirtualPath::new("/engine/processes").expect("processes target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/turns").expect("turns alias"),
            VirtualPath::new("/engine/turns").expect("turns target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/run-state").expect("run-state alias"),
            VirtualPath::new("/engine/run-state").expect("run-state target"),
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .expect("legacy migration mount view");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ));
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new("/turns/state.json").expect("legacy turn state path"),
            Entry::bytes(b"{\"runs\":[{\"legacy\":\"present\"}]}".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed deployed legacy authority");
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));

    let error = store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::new(),
            process_kind: ProcessKind::AgentTurn,
            scope: scope(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: None,
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect_err("pre-start migration must not initialize over malformed deployed turn state");
    assert!(matches!(
        error,
        ProcessJournalStoreError::Deserialization(_)
    ));
    assert!(
        filesystem
            .get(
                &ResourceScope::system(),
                &ScopedPath::new("/processes/materialized/metadata").expect("metadata path"),
            )
            .await
            .expect("metadata lookup")
            .is_none()
    );
}

#[tokio::test]
async fn deployed_turn_blob_and_run_state_import_before_first_process_request() {
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/processes").expect("processes alias"),
            VirtualPath::new("/engine/processes").expect("processes target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/turns").expect("turns alias"),
            VirtualPath::new("/engine/turns").expect("turns target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/run-state").expect("run-state alias"),
            VirtualPath::new("/engine/run-state").expect("run-state target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/checkpoint-state").expect("checkpoint-state alias"),
            VirtualPath::new("/engine/checkpoint-state").expect("checkpoint-state target"),
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/legacy-tenants").expect("legacy tenants alias"),
            VirtualPath::new("/tenants").expect("legacy tenants target"),
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .expect("legacy migration mount view");
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ));
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let checkpoint_id = TurnCheckpointId::new();
    let external_checkpoint_id = TurnCheckpointId::new();
    let root_run_id = TurnRunId::new();
    let capability_invocation_id = InvocationId::new();
    let per_user_capability_invocation_id = InvocationId::new();
    let turn_scope = json!({
        "tenant_id": "tenant-process-test",
        "agent_id": "agent-process-test",
        "project_id": "project-process-test",
        "thread_id": "thread-process-test",
        "thread_owner": {
            "mode": "explicit_user",
            "owner_user_id": "user-process-test"
        }
    });
    let legacy = json!({
        "turns": [{
            "turn_id": turn_id,
            "actor": {"user_id": "user-process-test"},
            "created_at": "2026-01-01T00:00:00Z"
        }],
        "runs": [{
            "run_id": run_id,
            "turn_id": turn_id,
            "scope": turn_scope,
            "accepted_message_ref": "accepted:migration",
            "source_binding_ref": "source:migration",
            "reply_target_binding_ref": "reply:migration",
            "status": "BlockedApproval",
            "profile": {
                "id": "default",
                "version": 1,
                "allow_steering": false,
                "auto_queue_followups": false
            },
            "checkpoint_id": checkpoint_id,
            "gate_ref": "gate:migration-approval",
            "credential_requirements": [],
            "failure": null,
            "event_cursor": 41,
            "runner_id": null,
            "lease_token": null,
            "lease_expires_at": null,
            "last_heartbeat_at": null,
            "claim_count": 2,
            "received_at": "2026-01-01T00:00:00Z",
            "parent_run_id": null,
            "subagent_depth": 0,
            "spawn_tree_root_run_id": root_run_id,
            "product_context": null
        }],
        "loop_checkpoints": [
            {
                "checkpoint_id": checkpoint_id,
                "scope": turn_scope,
                "turn_id": turn_id,
                "run_id": run_id,
                "state_ref": "state:migration",
                "payload": [114, 101, 115, 117, 109, 101],
                "schema_id": "interactive_checkpoint_v1",
                "schema_version": 1,
                "kind": "Gate",
                "gate_ref": "gate:migration-approval",
                "created_at": "2026-01-01T00:00:01Z"
            },
            {
                "checkpoint_id": external_checkpoint_id,
                "scope": turn_scope,
                "turn_id": turn_id,
                "run_id": run_id,
                "state_ref": "state:external",
                "schema_id": "interactive_checkpoint_v1",
                "schema_version": 1,
                "kind": "Gate",
                "gate_ref": "gate:migration-approval",
                "created_at": "2026-01-01T00:00:02Z"
            }
        ],
        "idempotency_records": [
            {
                "scope": turn_scope,
                "operation": "Submit",
                "key": "migration-submit-key",
                "run_id": run_id,
                "created_at": "2026-01-01T00:00:00Z"
            },
            {
                "scope": turn_scope,
                "operation": "Cancel",
                "key": "migration-cancel-key",
                "run_id": run_id,
                "created_at": "2026-01-01T00:00:00Z"
            }
        ],
        "spawn_tree_reservations": [{
            "scope": turn_scope,
            "root_run_id": root_run_id,
            "descendant_count": 1,
            "released_children": []
        }]
    });
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new("/turns/state.json").expect("legacy turn state path"),
            Entry::bytes(serde_json::to_vec(&legacy).expect("serialize legacy turns")),
            CasExpectation::Absent,
        )
        .await
        .expect("seed deployed turn blob");
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new(
                "/legacy-tenants/tenant-process-test/users/user-process-test/checkpoint-state/threads/thread-process-test/states/state/external.json",
            )
            .expect("per-user legacy checkpoint-state path"),
            Entry::bytes(
                serde_json::to_vec(&json!({
                    "state_ref": "state:external",
                    "scope": turn_scope,
                    "turn_id": turn_id,
                    "run_id": run_id,
                    "schema_id": "interactive_checkpoint_v1",
                    "schema_version": 1,
                    "kind": "Gate",
                    "payload_hex": "65787465726e616c",
                    "created_at": "2026-01-01T00:00:02Z"
                }))
                .expect("serialize checkpoint-state record"),
            ),
            CasExpectation::Absent,
        )
        .await
        .expect("seed deployed per-user checkpoint-state");
    let capability_scope = scope();
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new(format!(
                "/run-state/agents/agent-process-test/runs/{capability_invocation_id}.json"
            ))
            .expect("legacy capability path"),
            Entry::bytes(
                serde_json::to_vec(&json!({
                    "invocation_id": capability_invocation_id,
                    "capability_id": "builtin.echo",
                    "scope": capability_scope,
                    "authenticated_actor_user_id": "user-process-test",
                    "status": "BlockedAuth",
                    "approval_request_id": null,
                    "error_kind": null
                }))
                .expect("serialize capability run"),
            ),
            CasExpectation::Absent,
        )
        .await
        .expect("seed deployed capability run");
    filesystem
        .put(
            &ResourceScope::system(),
            &ScopedPath::new(format!(
                "/legacy-tenants/tenant-process-test/users/user-process-test/run-state/agents/agent-process-test/runs/{per_user_capability_invocation_id}.json"
            ))
            .expect("per-user legacy capability path"),
            Entry::bytes(
                serde_json::to_vec(&json!({
                    "invocation_id": per_user_capability_invocation_id,
                    "capability_id": "builtin.per-user",
                    "scope": capability_scope,
                    "authenticated_actor_user_id": "user-process-test",
                    "status": "Completed",
                    "approval_request_id": null,
                    "error_kind": null
                }))
                .expect("serialize per-user capability run"),
            ),
            CasExpectation::Absent,
        )
        .await
        .expect("seed deployed per-user capability run");

    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    assert_eq!(
        store
            .migrate_legacy_journal()
            .await
            .expect("pre-start deployed migration"),
        3
    );
    let imported_rows = filesystem
        .query(
            &ResourceScope::system(),
            &ScopedPath::new("/processes/materialized/process").expect("process rows"),
            &Filter::All,
            Page::default(),
        )
        .await
        .expect("query imported process rows");
    assert_eq!(imported_rows.len(), 3, "all imported snapshots persist");
    let turn_process_id = ProcessId::from_uuid(run_id.as_uuid());
    let mut imported_scope = ResourceScope::system();
    imported_scope.tenant_id = TenantId::new("tenant-process-test").expect("tenant");
    imported_scope.user_id = UserId::new("user-process-test").expect("user");
    imported_scope.agent_id = Some(AgentId::new("agent-process-test").expect("agent"));
    imported_scope.project_id = Some(ProjectId::new("project-process-test").expect("project"));
    imported_scope.thread_id = Some(ThreadId::new("thread-process-test").expect("thread"));
    imported_scope.invocation_id = InvocationId::from_uuid(run_id.as_uuid());
    let turn = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: imported_scope.clone(),
            process_id: turn_process_id,
        })
        .await
        .expect("first process request imports deployed state");
    assert_eq!(turn.status, ProcessLifecycleStatus::Suspended);
    assert_eq!(
        turn.suspension
            .as_ref()
            .and_then(|suspension| suspension.gate_ref.as_ref())
            .map(TurnGateRef::as_str),
        Some("gate:migration-approval")
    );
    let checkpoint = store
        .get_process_checkpoint(GetProcessCheckpointRequest {
            checkpoint_id: ProcessCheckpointId::from_trusted(checkpoint_id.as_uuid().to_string()),
            process_id: turn_process_id,
            scope: imported_scope.clone(),
        })
        .await
        .expect("read imported checkpoint")
        .expect("checkpoint exists");
    assert_eq!(checkpoint.payload.as_bytes(), b"resume");
    let external_checkpoint = store
        .get_process_checkpoint(GetProcessCheckpointRequest {
            checkpoint_id: ProcessCheckpointId::from_trusted(
                external_checkpoint_id.as_uuid().to_string(),
            ),
            process_id: turn_process_id,
            scope: imported_scope.clone(),
        })
        .await
        .expect("read externally stored checkpoint")
        .expect("external checkpoint exists");
    assert_eq!(external_checkpoint.payload.as_bytes(), b"external");

    let replay = store
        .submit_process(SubmitProcessRequest {
            process_id: turn_process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: imported_scope.clone(),
            exclusive_within_scope: true,
            operation_id: Some(ProcessOperationId::from_trusted("migration-submit-key")),
            owner_user_id: Some(UserId::new("user-process-test").expect("owner")),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: json!({}),
        })
        .await
        .expect("legacy submit idempotency replays");
    assert_eq!(replay.status, ProcessLifecycleStatus::Suspended);
    let cancel_replay = store
        .request_cancel_process(CancelProcessRequest {
            scope: imported_scope.clone(),
            process_id: turn_process_id,
            operation_id: Some(ProcessOperationId::from_trusted("migration-cancel-key")),
            reason: Some("must not mutate an imported replay".to_string()),
        })
        .await
        .expect("legacy cancel idempotency replays");
    assert_eq!(
        cancel_replay.state.status,
        ProcessLifecycleStatus::Suspended
    );

    let capability = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: capability_scope.clone(),
            process_id: ProcessId::from_uuid(capability_invocation_id.as_uuid()),
        })
        .await
        .expect("read imported capability invocation");
    assert_eq!(capability.status, ProcessLifecycleStatus::Suspended);
    let per_user_capability = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: capability_scope,
            process_id: ProcessId::from_uuid(per_user_capability_invocation_id.as_uuid()),
        })
        .await
        .expect("read imported per-user capability invocation");
    assert_eq!(
        per_user_capability.status,
        ProcessLifecycleStatus::Completed
    );
    assert_eq!(
        store
            .migrate_legacy_journal()
            .await
            .expect("migration rerun is idempotent"),
        0
    );
}

#[tokio::test]
async fn deployed_turn_row_layout_imports_materialized_run_rows() {
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/processes").expect("processes alias"),
                VirtualPath::new("/engine/processes").expect("processes target"),
                MountPermissions::read_write_list_delete(),
            ),
            MountGrant::new(
                MountAlias::new("/turns").expect("turns alias"),
                VirtualPath::new("/engine/turns").expect("turns target"),
                MountPermissions::read_write_list_delete(),
            ),
        ])
        .expect("migration mounts"),
    ));
    let run_id = TurnRunId::new();
    let turn_id = TurnId::new();
    let row = json!({
        "run_id": run_id,
        "turn_id": turn_id,
        "scope": {
            "tenant_id": "tenant-process-test",
            "agent_id": "agent-process-test",
            "project_id": "project-process-test",
            "thread_id": "thread-row-migration",
            "thread_owner": {
                "mode": "explicit_user",
                "owner_user_id": "user-process-test"
            }
        },
        "accepted_message_ref": "accepted:row-migration",
        "source_binding_ref": "source:row-migration",
        "reply_target_binding_ref": "reply:row-migration",
        "status": "Completed",
        "profile": {
            "id": "default",
            "version": 1,
            "allow_steering": false,
            "auto_queue_followups": false
        },
        "checkpoint_id": null,
        "gate_ref": null,
        "credential_requirements": [],
        "failure": null,
        "event_cursor": 7,
        "runner_id": null,
        "lease_token": null,
        "lease_expires_at": null,
        "last_heartbeat_at": null,
        "claim_count": 0,
        "received_at": "2026-01-02T00:00:00Z",
        "parent_run_id": null,
        "subagent_depth": 0,
        "spawn_tree_root_run_id": null,
        "product_context": null
    });
    for (path, body) in [
        (
            "/turns/rows/v1/meta/state.json".to_string(),
            json!({"journal_seq": 1, "event_retention_floor": 0}),
        ),
        (
            format!("/turns/rows/v1/runs/{run_id}.json"),
            json!({"journal_seq": 1, "value": row}),
        ),
    ] {
        filesystem
            .put(
                &ResourceScope::system(),
                &ScopedPath::new(path).expect("legacy row path"),
                Entry::bytes(serde_json::to_vec(&body).expect("serialize legacy row")),
                CasExpectation::Absent,
            )
            .await
            .expect("seed legacy row");
    }
    let store = ProcessJournalStore::new(filesystem);
    let mut imported_scope = ResourceScope::system();
    imported_scope.tenant_id = TenantId::new("tenant-process-test").expect("tenant");
    imported_scope.user_id = UserId::new("user-process-test").expect("user");
    imported_scope.agent_id = Some(AgentId::new("agent-process-test").expect("agent"));
    imported_scope.project_id = Some(ProjectId::new("project-process-test").expect("project"));
    imported_scope.thread_id = Some(ThreadId::new("thread-row-migration").expect("thread"));
    imported_scope.invocation_id = InvocationId::from_uuid(run_id.as_uuid());
    let imported = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: imported_scope,
            process_id: ProcessId::from_uuid(run_id.as_uuid()),
        })
        .await
        .expect("row-native deployed run imports");
    assert_eq!(imported.status, ProcessLifecycleStatus::Completed);
    assert_eq!(imported.journal_cursor, ProcessJournalCursor(7));
}

#[tokio::test]
async fn explicit_row_native_migration_rebuilds_sparse_process_indexes() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = scope();
    let process_id = ProcessId::new();
    submit_internal_process(&store, &scope, process_id).await;
    let path = ScopedPath::new(format!(
        "/processes/materialized/process/{}",
        process_id.as_uuid()
    ))
    .expect("process row path");
    let mut row = filesystem
        .get(&ResourceScope::system(), &path)
        .await
        .expect("read process row")
        .expect("process row exists");
    row.entry
        .indexed
        .remove(&IndexKey::new("queue_status").expect("queue status key"));
    filesystem
        .put(
            &ResourceScope::system(),
            &path,
            row.entry,
            CasExpectation::Version(row.version),
        )
        .await
        .expect("damage queue projection");
    let request = ClaimProcessesRequest {
        worker_id: ProcessWorkerId::from_trusted("migration-worker"),
        scope_filter: None,
        process_id_filter: None,
        process_kind_filter: None,
        max_processes: 1,
    };
    assert!(
        store
            .claim_next_processes(request.clone())
            .await
            .expect("query damaged queue")
            .is_empty()
    );

    assert_eq!(
        store
            .migrate_row_native_indexes()
            .await
            .expect("rebuild row-native indexes"),
        2
    );
    let claimed = store
        .claim_next_processes(request)
        .await
        .expect("claim rebuilt queue");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].state.process_id, process_id);
}

#[tokio::test]
async fn interrupted_row_native_index_migration_retries_from_a_stable_boundary() {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()));
    let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&backend),
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/processes").expect("process alias"),
            VirtualPath::new("/engine/processes").expect("process target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("process mount"),
    ));
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let resource_scope = scope();
    let mut last_process_id = None;
    for _ in 0..=Page::MAX_LIMIT {
        let process_id = ProcessId::new();
        submit_internal_process(&store, &resource_scope, process_id).await;
        last_process_id = Some(process_id);
    }

    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("/processes/materialized/process")
            .nth(Page::MAX_LIMIT as usize + 1)
            .backend("interrupt after first committed migration batch"),
    );
    let error = store
        .migrate_row_native_indexes()
        .await
        .expect_err("injected second-batch write interrupts migration");
    assert!(matches!(
        error,
        ProcessJournalStoreError::Filesystem(FilesystemError::Backend { .. })
    ));

    let migrated = store
        .migrate_row_native_indexes()
        .await
        .expect("retry converges after the one-shot interruption");
    assert!(
        migrated >= (Page::MAX_LIMIT as usize + 1) * 2,
        "journal and process collections are both replayed"
    );
    let last_process_id = last_process_id.expect("at least one process");
    let snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: resource_scope,
            process_id: last_process_id,
        })
        .await
        .expect("last row beyond the first migration batch remains queryable");
    assert_eq!(snapshot.process_id, last_process_id);
}

#[tokio::test]
async fn process_checkpoint_records_are_durable_scoped_and_idempotent() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = scope();
    let process_id = ProcessId::new();
    submit_internal_process(&store, &scope, process_id).await;
    let checkpoint_id = ProcessCheckpointId::from_trusted("checkpoint-1");
    let request = RecordProcessCheckpointRequest {
        checkpoint_id: checkpoint_id.clone(),
        process_id,
        scope: scope.clone(),
        state_ref: ProcessCheckpointRef::from_trusted("state-1"),
        payload: ProcessCheckpointPayload::new(b"checkpoint-body".to_vec())
            .expect("bounded payload"),
        created_at: Utc::now(),
        link_to_process: true,
        metadata: json!({"schema": "agent-loop-v1"}),
    };

    let recorded = store
        .record_process_checkpoint(request.clone())
        .await
        .expect("record checkpoint");
    assert_eq!(
        store
            .record_process_checkpoint(request)
            .await
            .expect("idempotent record"),
        recorded
    );
    let terminal_evidence_id = ProcessCheckpointId::from_trusted("checkpoint-final-evidence");
    store
        .record_process_checkpoint(RecordProcessCheckpointRequest {
            checkpoint_id: terminal_evidence_id.clone(),
            process_id,
            scope: scope.clone(),
            state_ref: ProcessCheckpointRef::from_trusted("state-final-evidence"),
            payload: ProcessCheckpointPayload::new(b"terminal evidence".to_vec())
                .expect("bounded payload"),
            created_at: Utc::now(),
            link_to_process: false,
            metadata: json!({"kind": "final"}),
        })
        .await
        .expect("record unlinked terminal evidence");

    let reopened = ProcessJournalStore::new(filesystem);
    let loaded = reopened
        .get_process_checkpoint(GetProcessCheckpointRequest {
            checkpoint_id: checkpoint_id.clone(),
            process_id,
            scope: scope.clone(),
        })
        .await
        .expect("load checkpoint");
    assert_eq!(loaded, Some(recorded));
    let loaded = loaded.expect("checkpoint exists");
    assert_eq!(loaded.payload.as_bytes(), b"checkpoint-body");
    let debug = format!("{:?}", loaded.payload);
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("checkpoint-body"));
    assert!(
        reopened
            .get_process_checkpoint(GetProcessCheckpointRequest {
                checkpoint_id: terminal_evidence_id,
                process_id,
                scope: scope.clone(),
            })
            .await
            .expect("load terminal evidence")
            .is_some(),
        "unlinked terminal evidence must remain durable"
    );
    assert_eq!(
        reopened
            .get_process_snapshot(GetProcessSnapshotRequest {
                scope: scope.clone(),
                process_id,
            })
            .await
            .expect("load checkpointed process")
            .checkpoint_ref,
        Some(ProcessCheckpointRef::from_trusted(
            checkpoint_id.as_str().to_string()
        )),
        "recording terminal evidence must not replace the active resume checkpoint"
    );

    let mut wrong_scope = scope;
    wrong_scope.user_id = UserId::new("other-user").expect("other user");
    assert!(
        reopened
            .get_process_checkpoint(GetProcessCheckpointRequest {
                checkpoint_id,
                process_id,
                scope: wrong_scope,
            })
            .await
            .expect("wrong-scope lookup")
            .is_none()
    );
}

#[test]
fn process_checkpoint_payload_rejects_oversized_bytes() {
    let error = ProcessCheckpointPayload::new(vec![0; MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES + 1])
        .expect_err("oversized checkpoint payload");
    assert!(matches!(
        error,
        ironclaw_processes::ProcessJournalError::CheckpointPayloadTooLong { actual }
            if actual == MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES + 1
    ));
}

#[tokio::test]
async fn process_observer_receives_commits_once_not_idempotency_replays() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let observer = Arc::new(RecordingProcessObserver::default());
    store
        .subscribe_process_observer(observer.clone())
        .expect("subscribe observer");
    let scope = scope();
    let request = SubmitProcessRequest {
        process_id: ProcessId::new(),
        process_kind: ProcessKind::Internal,
        scope: scope.clone(),
        exclusive_within_scope: false,
        operation_id: Some(ProcessOperationId::from_trusted("submit-once")),
        owner_user_id: Some(scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };

    store
        .submit_process(request.clone())
        .await
        .expect("submit process");
    store
        .submit_process(request)
        .await
        .expect("replay process submission");

    let commits = observer.commits.lock().expect("observer commits");
    assert_eq!(commits.len(), 1);
    assert_eq!(
        commits[0].kind,
        ironclaw_processes::ProcessJournalKind::Submitted
    );
}

#[tokio::test]
async fn committed_process_mutation_is_not_reported_failed_when_wake_hint_fails() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    store
        .subscribe_process_observer(Arc::new(FailingProcessObserver))
        .expect("subscribe observer");
    let scope = scope();
    let process_id = ProcessId::new();

    let submitted = store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("durably committed submission must remain successful");

    assert_eq!(submitted.process_id, process_id);
}

#[tokio::test]
async fn observer_registration_replays_commits_durably_after_restart() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = scope();
    let process_id = ProcessId::new();
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit before observer registration");

    let reopened = ProcessJournalStore::new(filesystem);
    let observer = Arc::new(RecordingProcessObserver::default());
    reopened
        .subscribe_process_observer(observer.clone())
        .expect("subscribe observer after restart");

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if observer
                .commits
                .lock()
                .expect("observer commits")
                .iter()
                .any(|commit| commit.state.process_id == process_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("durable observer replay");
}

/// Two store instances can share one observer id across a rolling restart, and
/// each caches its own view of the shared cursor row. The instance holding the
/// older view must never overwrite the newer durable cursor: rewinding it
/// silently redelivers the difference after the next restart, and the persisted
/// acknowledgement stops representing real progress.
///
/// The newer position is written directly here so the stale-cache window is
/// deterministic — the contiguity check hides it whenever the other instance's
/// entries also land in this instance's journal view.
#[tokio::test]
async fn observer_cursor_never_rewinds_from_a_stale_cache() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = scope();
    let submit = async |store: &ProcessJournalStore<_>| {
        store
            .submit_process(SubmitProcessRequest {
                process_id: ProcessId::new(),
                process_kind: ProcessKind::Internal,
                scope: scope.clone(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id: Some(scope.user_id.clone()),
                concurrency_class: None,
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit");
    };
    let observer = Arc::new(RecordingProcessObserver::default());
    store
        .subscribe_process_observer(observer.clone())
        .expect("subscribe observer");

    // Deliver once so this instance caches the cursor row it just wrote.
    submit(&store).await;
    let cursor_path = observer_cursor_path();
    await_observer_cursor(&filesystem, &cursor_path).await;

    // A second instance races ahead of this one's cached view.
    const AHEAD: u64 = 9_999;
    filesystem
        .put(
            &ResourceScope::system(),
            &cursor_path,
            Entry::bytes(serde_json::to_vec(&AHEAD).expect("cursor body")),
            CasExpectation::Any,
        )
        .await
        .expect("simulate a second instance advancing the shared cursor");

    // This instance commits again on its stale cache. Its next entry is
    // contiguous with what it cached, so it takes the in-memory delivery path.
    submit(&store).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if read_observer_cursor(&filesystem, &cursor_path).await != Some(AHEAD) {
                tokio::task::yield_now().await;
                continue;
            }
            // Give a rewind a chance to appear before declaring success.
            tokio::task::yield_now().await;
            if read_observer_cursor(&filesystem, &cursor_path).await == Some(AHEAD) {
                break;
            }
        }
    })
    .await
    .unwrap_or(());
    let observed = read_observer_cursor(&filesystem, &cursor_path).await;
    assert_eq!(
        observed,
        Some(AHEAD),
        "a stale cache must never rewind the durable observer cursor"
    );
}

fn observer_cursor_path() -> ScopedPath {
    let digest = blake3::hash(b"recording-process-observer").to_hex();
    ScopedPath::new(format!("/processes/materialized/observer-cursor/{digest}"))
        .expect("cursor path")
}

async fn read_observer_cursor(
    filesystem: &Arc<ScopedFilesystem<InMemoryBackend>>,
    path: &ScopedPath,
) -> Option<u64> {
    filesystem
        .get(&ResourceScope::system(), path)
        .await
        .expect("read cursor")
        .map(|versioned| serde_json::from_slice::<u64>(&versioned.entry.body).expect("cursor body"))
}

async fn await_observer_cursor(
    filesystem: &Arc<ScopedFilesystem<InMemoryBackend>>,
    path: &ScopedPath,
) -> u64 {
    for _ in 0..2_000 {
        if let Some(cursor) = read_observer_cursor(filesystem, path).await {
            return cursor;
        }
        tokio::task::yield_now().await;
    }
    panic!("observer cursor row was never written");
}

#[tokio::test]
async fn observer_failure_retries_until_durable_cursor_is_acknowledged() {
    let filesystem = in_memory_backed_processes_filesystem();
    let store = ProcessJournalStore::new(Arc::clone(&filesystem));
    let scope = scope();
    store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::new(),
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit before observer registration");

    let observer = Arc::new(FailOnceProcessObserver {
        attempts: AtomicUsize::new(0),
    });
    ProcessJournalStore::new(Arc::clone(&filesystem))
        .subscribe_process_observer(observer.clone())
        .expect("subscribe transient observer");
    tokio::time::timeout(Duration::from_secs(2), async {
        while observer.attempts.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("observer retry succeeds");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        observer.attempts.load(Ordering::SeqCst),
        2,
        "overlapping replay tasks must not redeliver an acknowledged commit"
    );

    let after_restart = Arc::new(FailOnceProcessObserver {
        attempts: AtomicUsize::new(0),
    });
    ProcessJournalStore::new(filesystem)
        .subscribe_process_observer(after_restart.clone())
        .expect("subscribe after acknowledged replay");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(after_restart.attempts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn group_committed_submissions_deliver_every_entry_once_before_returning() {
    let store = Arc::new(ProcessJournalStore::new(
        in_memory_backed_processes_filesystem(),
    ));
    let observer = Arc::new(SuspendingProcessObserver::default());
    store
        .subscribe_process_observer(observer.clone())
        .expect("subscribe observer");
    let scope = scope();
    // Materialize the store and start the group-commit funnel so the
    // submissions below are all queued before the flusher drains them.
    let warmup = submit_internal_process(store.as_ref(), &scope, ProcessId::new()).await;

    let process_ids = (0..32).map(|_| ProcessId::new()).collect::<Vec<_>>();
    let submissions = process_ids.iter().copied().map(|process_id| {
        let store = Arc::clone(&store);
        let observer = Arc::clone(&observer);
        let scope = scope.clone();
        async move {
            let snapshot = submit_internal_process(store.as_ref(), &scope, process_id).await;
            // Read-your-writes: a caller that reads projections right after
            // this call returns must never miss its own commit.
            let delivered = observer
                .commits
                .lock()
                .expect("observer commits")
                .iter()
                .any(|commit| commit.state.process_id == process_id);
            assert!(
                delivered,
                "observer must have seen {process_id} before its submission returned"
            );
            snapshot
        }
    });
    let snapshots = futures::future::join_all(submissions).await;
    assert_eq!(snapshots.len(), process_ids.len());

    // The group-committed submissions reach the observer batched, not one call
    // per entry: delivery sits between the commit and the caller's response, so
    // a per-entry hand-off there costs a round trip per entry. Asserted as a
    // property rather than an exact batch size, which depends on how far the
    // runtime lets submissions queue before the flusher drains them.
    let batch_sizes = observer.batch_sizes.lock().expect("observer batch sizes");
    let largest = batch_sizes.iter().copied().max().unwrap_or_default();
    assert!(
        largest > 1,
        "expected batched delivery, saw only per-entry calls {batch_sizes:?}"
    );
    assert!(
        batch_sizes.len() < process_ids.len(),
        "expected fewer deliveries than commits, saw batches {batch_sizes:?}"
    );
    drop(batch_sizes);

    let commits = observer.commits.lock().expect("observer commits");
    assert_eq!(
        commits.len(),
        process_ids.len() + 1,
        "every committed entry is delivered exactly once"
    );
    let mut previous = 0;
    for commit in commits.iter() {
        assert!(
            commit.state.journal_cursor.0 > previous,
            "observer deliveries must be strictly ordered by cursor"
        );
        previous = commit.state.journal_cursor.0;
    }
    let delivered = commits
        .iter()
        .map(|commit| commit.state.process_id)
        .collect::<Vec<_>>();
    for process_id in process_ids
        .iter()
        .chain(std::iter::once(&warmup.process_id))
    {
        assert_eq!(
            delivered
                .iter()
                .filter(|delivered| *delivered == process_id)
                .count(),
            1,
            "{process_id} must be delivered exactly once"
        );
    }
}

#[tokio::test]
async fn group_commit_isolates_a_rejected_command_from_its_batch() {
    let store = Arc::new(ProcessJournalStore::new(
        in_memory_backed_processes_filesystem(),
    ));
    let scope = scope();
    let leased_process_id = ProcessId::new();
    submit_internal_process(store.as_ref(), &scope, leased_process_id).await;
    let claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string()),
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process")
        .pop()
        .expect("claimed process");

    let rejected = {
        let store = Arc::clone(&store);
        async move {
            store
                .complete_process(ProcessStateTransitionRequest {
                    lease: ProcessLeaseRequest {
                        process_id: leased_process_id,
                        worker_id: claim.worker_id,
                        lease_token: ProcessLeaseToken::from_trusted(
                            ProcessId::new().as_uuid().to_string(),
                        ),
                    },
                    metadata: None,
                })
                .await
        }
    };
    let accepted_ids = (0..8).map(|_| ProcessId::new()).collect::<Vec<_>>();
    let accepted = futures::future::join_all(accepted_ids.iter().copied().map(|process_id| {
        let store = Arc::clone(&store);
        let scope = scope.clone();
        async move { submit_internal_process(store.as_ref(), &scope, process_id).await }
    }));
    // Both futures are queued before the flusher runs, so the rejected command
    // and the valid submissions share one group-commit transaction.
    let (rejected, accepted) = tokio::join!(rejected, accepted);

    let error = rejected.expect_err("wrong lease must fail");
    assert!(error.to_string().contains("lease is invalid"));
    for (process_id, snapshot) in accepted_ids.iter().zip(accepted.iter()) {
        assert_eq!(&snapshot.process_id, process_id);
        assert_eq!(snapshot.status, ProcessLifecycleStatus::Queued);
    }
    let persisted = store
        .process_snapshots(&scope)
        .await
        .expect("read persisted process snapshots")
        .into_iter()
        .map(|snapshot| snapshot.process_id)
        .collect::<Vec<_>>();
    for process_id in &accepted_ids {
        assert!(
            persisted.contains(process_id),
            "a batch member rejected for an invalid lease must not drop {process_id}"
        );
    }
}

#[tokio::test]
async fn process_submission_idempotency_ignores_fresh_invocation_identity() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let first_scope = scope();
    let mut retry_scope = first_scope.clone();
    retry_scope.invocation_id = InvocationId::new();
    let original_process_id = ProcessId::new();
    let owner_user_id = first_scope.user_id.clone();
    let request = |process_id, scope| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::Internal,
        scope,
        exclusive_within_scope: false,
        operation_id: Some(ProcessOperationId::from_trusted("stable-operation")),
        owner_user_id: Some(owner_user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };

    let submitted = store
        .submit_process(request(original_process_id, first_scope))
        .await
        .expect("initial submission");
    let replayed = store
        .submit_process(request(ProcessId::new(), retry_scope))
        .await
        .expect("logical retry with fresh invocation identity");

    assert_eq!(replayed.process_id, submitted.process_id);
    assert_eq!(replayed.journal_cursor, submitted.journal_cursor);
}

#[tokio::test]
async fn process_claim_enforces_owner_and_class_concurrency_limits_atomically() {
    let owner_store = ProcessJournalStore::new(in_memory_backed_processes_filesystem())
        .with_concurrency_limits(ProcessConcurrencyLimits {
            max_running_per_owner: Some(1),
            max_running_by_class: BTreeMap::new(),
        });
    let scope = scope();
    submit_internal_process(&owner_store, &scope, ProcessId::new()).await;
    submit_internal_process(&owner_store, &scope, ProcessId::new()).await;
    let owner_claims = owner_store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("owner-worker"),
            scope_filter: None,
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 10,
        })
        .await
        .expect("claim owner-limited processes");
    assert_eq!(owner_claims.len(), 1);

    let class = ProcessConcurrencyClass::from_trusted("scheduled_trigger");
    let class_store = ProcessJournalStore::new(in_memory_backed_processes_filesystem())
        .with_concurrency_limits(ProcessConcurrencyLimits {
            max_running_per_owner: None,
            max_running_by_class: BTreeMap::from([(class.clone(), 1)]),
        });
    for (process_id, user_id) in [
        (ProcessId::new(), "class-user-a"),
        (ProcessId::new(), "class-user-b"),
    ] {
        let mut process_scope = scope.clone();
        process_scope.user_id = UserId::new(user_id).expect("class user");
        class_store
            .submit_process(SubmitProcessRequest {
                process_id,
                process_kind: ProcessKind::AgentTurn,
                scope: process_scope.clone(),
                exclusive_within_scope: false,
                operation_id: None,
                owner_user_id: Some(process_scope.user_id.clone()),
                concurrency_class: Some(class.clone()),
                parent_process_id: None,
                root_process_id: None,
                spawn_tree_descendant_cap: None,
                dependency: None,
                checkpoint_ref: None,
                input: None,
                created_at: Utc::now(),
                metadata: serde_json::Value::Null,
            })
            .await
            .expect("submit class-limited process");
    }
    let class_claims = class_store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("class-worker"),
            scope_filter: None,
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 10,
        })
        .await
        .expect("claim class-limited processes");
    assert_eq!(class_claims.len(), 1);
}

#[tokio::test]
async fn process_claim_pages_past_a_quota_blocked_prefix() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem())
        .with_concurrency_limits(ProcessConcurrencyLimits {
            max_running_per_owner: Some(1),
            max_running_by_class: BTreeMap::new(),
        });
    let scope = scope();
    let blocked_owner = UserId::new("blocked-owner").expect("blocked owner");
    let eligible_owner = UserId::new("eligible-owner").expect("eligible owner");
    let submit = |process_id, owner_user_id| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::Internal,
        scope: scope.clone(),
        exclusive_within_scope: false,
        operation_id: None,
        owner_user_id: Some(owner_user_id),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };

    let running_id = ProcessId::new();
    store
        .submit_process(submit(running_id, blocked_owner.clone()))
        .await
        .expect("submit running quota holder");
    store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("quota-holder"),
            scope_filter: None,
            process_id_filter: Some(running_id),
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim quota holder");

    for _ in 0..64 {
        store
            .submit_process(submit(ProcessId::new(), blocked_owner.clone()))
            .await
            .expect("submit quota-blocked prefix");
    }
    let eligible_id = ProcessId::new();
    store
        .submit_process(submit(eligible_id, eligible_owner))
        .await
        .expect("submit eligible process after blocked prefix");

    let claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("paging-worker"),
            scope_filter: None,
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("page beyond blocked candidates");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].state.process_id, eligible_id);
}

#[tokio::test]
async fn process_tree_submission_reserves_and_releases_capacity_atomically() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let root_scope = scope();
    let root_id = ProcessId::new();
    submit_internal_process(&store, &root_scope, root_id).await;
    let mut child_scope = root_scope.clone();
    child_scope.thread_id = Some(ThreadId::new("thread-child").expect("child thread"));
    let child_request = |process_id, operation: &str| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::Internal,
        scope: child_scope.clone(),
        exclusive_within_scope: false,
        operation_id: Some(ProcessOperationId::from_trusted(operation)),
        owner_user_id: Some(child_scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: Some(root_id),
        root_process_id: Some(root_id),
        spawn_tree_descendant_cap: Some(1),
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };
    let first_child_id = ProcessId::new();
    store
        .submit_process(child_request(first_child_id, "first-child"))
        .await
        .expect("submit first child");
    let capacity_error = store
        .submit_process(child_request(ProcessId::new(), "over-cap"))
        .await
        .expect_err("tree cap must reject second live reservation");
    assert!(capacity_error.to_string().contains("capacity 1 exceeded"));

    let children = store
        .child_processes(&root_scope, root_id)
        .await
        .expect("list child processes");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].process_id, first_child_id);

    let release = ReleaseProcessTreeRequest {
        scope: root_scope,
        root_process_id: root_id,
        delta: 1,
        idempotency_process_id: first_child_id,
    };
    store
        .release_process_tree(release.clone())
        .await
        .expect("release child reservation");
    store
        .release_process_tree(release)
        .await
        .expect("release replay is idempotent");
    store
        .submit_process(child_request(ProcessId::new(), "replacement-child"))
        .await
        .expect("released capacity admits replacement child");
}

#[tokio::test]
async fn explicit_tree_reservation_release_and_prune_preserve_capacity_invariants() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let root_scope = scope();
    let root_id = ProcessId::new();
    submit_internal_process(&store, &root_scope, root_id).await;

    let reserved = store
        .reserve_process_tree(ReserveProcessTreeRequest {
            scope: root_scope.clone(),
            root_process_id: root_id,
            delta: 2,
            cap: 2,
        })
        .await
        .expect("reserve tree capacity");
    assert_eq!(reserved.descendant_count, 2);
    assert!(reserved.released_processes.is_empty());

    let capacity_error = store
        .reserve_process_tree(ReserveProcessTreeRequest {
            scope: root_scope.clone(),
            root_process_id: root_id,
            delta: 1,
            cap: 2,
        })
        .await
        .expect_err("reservation above cap must fail");
    assert!(matches!(
        capacity_error,
        ProcessJournalStoreError::ProcessTreeCapacityExceeded { cap: 2 }
    ));

    let child_id = ProcessId::new();
    let release = ReleaseProcessTreeRequest {
        scope: root_scope.clone(),
        root_process_id: root_id,
        delta: 1,
        idempotency_process_id: child_id,
    };
    store
        .release_process_tree(release.clone())
        .await
        .expect("release reservation");
    store
        .release_process_tree(release.clone())
        .await
        .expect("release replay");
    let refilled = store
        .reserve_process_tree(ReserveProcessTreeRequest {
            scope: root_scope.clone(),
            root_process_id: root_id,
            delta: 1,
            cap: 2,
        })
        .await
        .expect("refill released capacity");
    assert_eq!(refilled.descendant_count, 2);
    assert!(refilled.released_processes.contains(&child_id));

    store
        .prune_released_process(PruneReleasedProcessRequest {
            scope: root_scope,
            root_process_id: root_id,
            process_id: child_id,
        })
        .await
        .expect("prune released child marker");
    store
        .release_process_tree(release)
        .await
        .expect("a pruned marker permits a new release for the same process id");
    let final_reservation = store
        .reserve_process_tree(ReserveProcessTreeRequest {
            scope: scope(),
            root_process_id: root_id,
            delta: 1,
            cap: 2,
        })
        .await
        .expect("capacity reflects post-prune release");
    assert_eq!(final_reservation.descendant_count, 2);
}

#[tokio::test]
async fn explicit_dependency_open_is_idempotent_scope_bound_and_abandonable() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let root_scope = scope();
    let root_id = ProcessId::new();
    submit_internal_process(&store, &root_scope, root_id).await;
    let dependency_id = ProcessId::new();
    let request = OpenProcessDependencyRequest {
        dependent_process_id: root_id,
        dependency_process_id: dependency_id,
        root_process_id: root_id,
        scope: root_scope.clone(),
        group_ref: Some("gate:explicit-open".to_string()),
        created_at: Utc::now(),
        metadata: json!({"owner": "runner"}),
    };

    let opened = store
        .open_process_dependency(request.clone())
        .await
        .expect("open dependency");
    assert_eq!(opened.state, ProcessDependencyState::Open);
    assert_eq!(opened.group_ref.as_deref(), Some("gate:explicit-open"));
    assert_eq!(
        store
            .open_process_dependency(request)
            .await
            .expect("open replay"),
        opened
    );

    let mut foreign_scope = root_scope.clone();
    foreign_scope.user_id = UserId::new("foreign-dependency-user").expect("foreign user");
    let error = store
        .open_process_dependency(OpenProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: ProcessId::new(),
            root_process_id: root_id,
            scope: foreign_scope,
            group_ref: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect_err("foreign scope must not open dependency");
    assert!(matches!(error, ProcessJournalStoreError::UnauthorizedScope));

    let abandoned = store
        .abandon_process_dependency(CloseProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: dependency_id,
            scope: root_scope.clone(),
            closed_at: Utc::now(),
        })
        .await
        .expect("abandon dependency")
        .expect("dependency exists");
    assert_eq!(abandoned.state, ProcessDependencyState::Abandoned);
    assert!(
        store
            .query_process_dependencies(ProcessDependencyQuery {
                scope: root_scope,
                dependent_process_id: Some(root_id),
                group_ref: Some("gate:explicit-open".to_string()),
                include_closed: false,
            })
            .await
            .expect("query open dependencies")
            .is_empty()
    );
}

#[tokio::test]
async fn consuming_dependency_atomically_releases_tree_capacity() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let root_scope = scope();
    let root_id = ProcessId::new();
    submit_internal_process(&store, &root_scope, root_id).await;
    let child_id = ProcessId::new();
    let mut child_scope = root_scope.clone();
    child_scope.thread_id = Some(ThreadId::new("dependency-child").expect("child thread"));

    store
        .submit_process(SubmitProcessRequest {
            process_id: child_id,
            process_kind: ProcessKind::Internal,
            scope: child_scope.clone(),
            exclusive_within_scope: false,
            operation_id: Some(ProcessOperationId::from_trusted("dependency-child")),
            owner_user_id: Some(child_scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(root_id),
            root_process_id: Some(root_id),
            spawn_tree_descendant_cap: Some(1),
            dependency: Some(ProcessDependencySubmission {
                dependent_process_id: root_id,
                root_process_id: root_id,
                group_ref: Some("gate:batch".to_string()),
                metadata: json!({"projection": "runner-owned"}),
            }),
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit dependency process");

    let rejected_child_id = ProcessId::new();
    let mut rejected_scope = root_scope.clone();
    rejected_scope.thread_id =
        Some(ThreadId::new("rejected-dependency-child").expect("rejected child thread"));
    let error = store
        .submit_process(SubmitProcessRequest {
            process_id: rejected_child_id,
            process_kind: ProcessKind::Internal,
            scope: rejected_scope.clone(),
            exclusive_within_scope: false,
            operation_id: Some(ProcessOperationId::from_trusted(
                "rejected-dependency-child",
            )),
            owner_user_id: Some(rejected_scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(root_id),
            root_process_id: Some(root_id),
            spawn_tree_descendant_cap: Some(1),
            dependency: Some(ProcessDependencySubmission {
                dependent_process_id: root_id,
                root_process_id: root_id,
                group_ref: Some("gate:rejected".to_string()),
                metadata: json!({"must_not_persist": true}),
            }),
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect_err("capacity rejection must reject the whole child submission");
    assert!(error.to_string().contains("capacity 1 exceeded"));
    assert!(
        store
            .query_process_dependencies(ProcessDependencyQuery {
                scope: rejected_scope,
                dependent_process_id: Some(root_id),
                group_ref: Some("gate:rejected".to_string()),
                include_closed: true,
            })
            .await
            .expect("query rejected dependency")
            .is_empty(),
        "a rejected child submission must not leave an orphan dependency"
    );

    let settled = store
        .settle_process_dependency(SettleProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: child_id,
            scope: child_scope.clone(),
            terminal: ProcessTerminalEvidence {
                status: ProcessLifecycleStatus::Completed,
                output_bytes: Some(42),
                sanitized_reason: None,
            },
            settled_at: Utc::now(),
        })
        .await
        .expect("settle dependency")
        .expect("dependency exists");
    assert_eq!(settled.state, ProcessDependencyState::Settled);

    let consumed = store
        .consume_process_dependency(CloseProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: child_id,
            scope: child_scope.clone(),
            closed_at: Utc::now(),
        })
        .await
        .expect("consume dependency")
        .expect("dependency exists");
    assert_eq!(consumed.state, ProcessDependencyState::Consumed);

    let unresolved = store
        .unresolved_process_dependencies()
        .await
        .expect("list unresolved dependencies");
    assert!(unresolved.is_empty());
    let closed = store
        .query_process_dependencies(ProcessDependencyQuery {
            scope: child_scope.clone(),
            dependent_process_id: Some(root_id),
            group_ref: Some("gate:batch".to_string()),
            include_closed: true,
        })
        .await
        .expect("query closed dependency");
    assert_eq!(closed, vec![consumed]);

    store
        .consume_process_dependency(CloseProcessDependencyRequest {
            dependent_process_id: root_id,
            dependency_process_id: child_id,
            scope: child_scope.clone(),
            closed_at: Utc::now(),
        })
        .await
        .expect("consume replay");
    store
        .submit_process(SubmitProcessRequest {
            process_id: ProcessId::new(),
            process_kind: ProcessKind::Internal,
            scope: child_scope,
            exclusive_within_scope: false,
            operation_id: Some(ProcessOperationId::from_trusted("dependency-replacement")),
            owner_user_id: Some(root_scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: Some(root_id),
            root_process_id: Some(root_id),
            spawn_tree_descendant_cap: Some(1),
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("consumed dependency releases capacity");
}

#[tokio::test]
async fn process_journal_store_owns_lifecycle_and_gate_projection() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let owner = scope.user_id.clone();
    let process_id = ProcessId::new();
    let worker_id = ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string());

    let submitted = store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(owner.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: json!({
                "agent_turn": {
                    "source_binding_ref": "source:journal-contract",
                    "reply_target_binding_ref": "reply:journal-contract"
                }
            }),
        })
        .await
        .expect("submit process");
    assert_eq!(submitted.status, ProcessLifecycleStatus::Queued);

    let claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: worker_id.clone(),
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process");
    assert_eq!(claimed.len(), 1);
    let claim = &claimed[0];
    assert_eq!(claim.state.process_id, process_id);
    assert_eq!(claim.state.status, ProcessLifecycleStatus::Running);

    let lease = ProcessLeaseRequest {
        process_id,
        worker_id: claim.worker_id.clone(),
        lease_token: claim.lease_token.clone(),
    };
    store
        .heartbeat_process(lease.clone())
        .await
        .expect("heartbeat process");

    let gate_ref = TurnGateRef::new("gate:journal-contract").expect("gate ref");
    store
        .suspend_process(SuspendProcessRequest {
            process_id,
            worker_id: lease.worker_id.clone(),
            lease_token: lease.lease_token.clone(),
            checkpoint_ref: ProcessCheckpointRef::new("checkpoint:journal-contract")
                .expect("checkpoint ref"),
            suspension: ProcessSuspension {
                kind: ProcessSuspensionKind::Authorization,
                gate_ref: Some(gate_ref.clone()),
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
            metadata: None,
        })
        .await
        .expect("suspend process");

    let lifecycle = store
        .process_lifecycle_states(ProcessLifecycleLookupBatchRequest {
            processes: vec![ProcessLifecycleLookupRequest {
                tenant_id: scope.tenant_id.clone(),
                process_id,
            }],
        })
        .await
        .pop()
        .expect("one lifecycle result")
        .expect("lifecycle lookup");
    assert!(matches!(
        lifecycle,
        ProcessLifecycleLookupResult::Found {
            status: ProcessLifecycleStatus::Suspended,
            ..
        }
    ));

    let gates = store
        .query_process_gates(ProcessGateQuery {
            scope: scope.clone(),
            gate_kind: ProcessSuspensionKind::Authorization,
            scope_match: None,
            owner_user_id: Some(owner),
            gate_ref: Some(gate_ref.clone()),
            owner_match: Some(ProcessGateOwnerMatch::Explicit),
            include_historical: false,
        })
        .await
        .expect("query gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].process_id, process_id);
    assert_eq!(gates[0].suspension.gate_ref.as_ref(), Some(&gate_ref));
    assert_eq!(
        gates[0].resume_source_ref.as_deref(),
        Some("source:journal-contract")
    );
    assert_eq!(
        gates[0].reply_target_ref.as_deref(),
        Some("reply:journal-contract")
    );

    let mut owner_scope = scope.clone();
    owner_scope.project_id = None;
    owner_scope.thread_id = None;
    let owner_gates = store
        .query_process_gates(ProcessGateQuery {
            scope: owner_scope,
            gate_kind: ProcessSuspensionKind::Authorization,
            scope_match: Some(ProcessGateScopeMatch::Owner),
            owner_user_id: gates[0].owner_user_id.clone(),
            gate_ref: None,
            owner_match: Some(ProcessGateOwnerMatch::Explicit),
            include_historical: false,
        })
        .await
        .expect("query gates across the explicit owner's projects");
    assert_eq!(owner_gates.len(), 1);
    assert_eq!(owner_gates[0].process_id, process_id);

    let snapshot = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id,
        })
        .await
        .expect("process snapshot");
    assert_eq!(snapshot.status, ProcessLifecycleStatus::Suspended);

    let page = store
        .read_process_journal_after(&scope, None, Some(ProcessJournalCursor(0)), 10)
        .await
        .expect("journal page");
    assert_eq!(page.entries.len(), 4);
    assert_eq!(page.entries[0].status, ProcessLifecycleStatus::Queued);
    assert_eq!(page.entries[3].status, ProcessLifecycleStatus::Suspended);
}

#[tokio::test]
async fn process_journal_store_completes_claimed_process() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let process_id = ProcessId::new();
    let worker_id = ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string());
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit process");
    let mut claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id,
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process");
    let claim = claimed.pop().expect("claimed process");
    assert!(claim.lease_token.as_str().len() <= 64);
    let completed = store
        .complete_process(ProcessStateTransitionRequest {
            lease: ProcessLeaseRequest {
                process_id,
                worker_id: claim.worker_id,
                lease_token: claim.lease_token,
            },
            metadata: Some(json!({"projection": {"usage": 42}})),
        })
        .await
        .expect("complete process");
    assert_eq!(completed.status, ProcessLifecycleStatus::Completed);
    assert!(completed.lease.is_none());
    assert_eq!(completed.metadata["projection"]["usage"], 42);
}

#[tokio::test]
async fn process_journal_store_relinquishes_claim_with_fresh_reclaim_lease() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let process_id = ProcessId::new();
    let first_worker = ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string());
    let second_worker = ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string());
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit process");

    let mut first_claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: first_worker,
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process");
    let first_claim = first_claim.pop().expect("claimed process");
    let relinquished = store
        .relinquish_process(ProcessLeaseRequest {
            process_id,
            worker_id: first_claim.worker_id,
            lease_token: first_claim.lease_token.clone(),
        })
        .await
        .expect("relinquish process");
    assert_eq!(relinquished.status, ProcessLifecycleStatus::Queued);
    assert!(relinquished.lease.is_none());

    let mut second_claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: second_worker.clone(),
            scope_filter: Some(scope),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("reclaim process");
    let second_claim = second_claim.pop().expect("reclaimed process");
    assert_eq!(second_claim.worker_id, second_worker);
    assert_ne!(second_claim.lease_token, first_claim.lease_token);
}

#[tokio::test]
async fn expired_leases_cancel_requested_work_and_requeue_safe_crashes() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem())
        .with_lease_duration(Duration::from_millis(1));
    let scope = scope();
    let requeue_id = ProcessId::new();
    let cancel_id = ProcessId::new();
    submit_internal_process(&store, &scope, requeue_id).await;
    submit_internal_process(&store, &scope, cancel_id).await;
    let claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("recovery-worker"),
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: Some(ProcessKind::Internal),
            max_processes: 2,
        })
        .await
        .expect("claim processes");
    assert_eq!(claimed.len(), 2);
    store
        .request_cancel_process(CancelProcessRequest {
            scope: scope.clone(),
            process_id: cancel_id,
            operation_id: None,
            reason: Some("user_cancelled".to_string()),
        })
        .await
        .expect("request cancellation");
    tokio::time::sleep(Duration::from_millis(2)).await;

    let recovered = store
        .recover_expired_process_leases(ironclaw_processes::RecoverExpiredProcessLeasesRequest {
            now: Utc::now(),
            scope_filter: Some(scope.clone()),
            process_kind_filter: Some(ProcessKind::Internal),
        })
        .await
        .expect("recover expired leases");
    assert_eq!(recovered.recovered.len(), 2);
    let requeued = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id: requeue_id,
        })
        .await
        .expect("requeued snapshot");
    let cancelled = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope,
            process_id: cancel_id,
        })
        .await
        .expect("cancelled snapshot");
    assert_eq!(requeued.status, ProcessLifecycleStatus::Queued);
    assert_eq!(cancelled.status, ProcessLifecycleStatus::Cancelled);
    assert!(requeued.lease.is_none());
    assert!(cancelled.lease.is_none());
}

#[tokio::test]
async fn expired_checkpointed_and_reclaim_exhausted_work_fails_boundedly() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem())
        .with_lease_duration(Duration::from_millis(1));
    let scope = scope();
    let checkpointed_id = ProcessId::new();
    store
        .submit_process(SubmitProcessRequest {
            process_id: checkpointed_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: Some(ProcessCheckpointRef::from_trusted("checkpointed")),
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit checkpointed process");
    store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("checkpointed-worker"),
            scope_filter: Some(scope.clone()),
            process_id_filter: Some(checkpointed_id),
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim checkpointed process");
    tokio::time::sleep(Duration::from_millis(2)).await;
    store
        .recover_expired_process_leases(ironclaw_processes::RecoverExpiredProcessLeasesRequest {
            now: Utc::now(),
            scope_filter: Some(scope.clone()),
            process_kind_filter: None,
        })
        .await
        .expect("recover checkpointed process");
    let checkpointed = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope: scope.clone(),
            process_id: checkpointed_id,
        })
        .await
        .expect("checkpointed snapshot");
    assert_eq!(checkpointed.status, ProcessLifecycleStatus::Failed);
    assert_eq!(
        checkpointed
            .failure
            .as_ref()
            .map(|failure| failure.category()),
        Some("lease_expired")
    );

    let exhausted_id = ProcessId::new();
    submit_internal_process(&store, &scope, exhausted_id).await;
    for attempt in 1..=3 {
        store
            .claim_next_processes(ClaimProcessesRequest {
                worker_id: ProcessWorkerId::from_trusted(format!("crash-worker-{attempt}")),
                scope_filter: Some(scope.clone()),
                process_id_filter: Some(exhausted_id),
                process_kind_filter: None,
                max_processes: 1,
            })
            .await
            .expect("claim crash-recovery process");
        tokio::time::sleep(Duration::from_millis(2)).await;
        store
            .recover_expired_process_leases(
                ironclaw_processes::RecoverExpiredProcessLeasesRequest {
                    now: Utc::now(),
                    scope_filter: Some(scope.clone()),
                    process_kind_filter: None,
                },
            )
            .await
            .expect("recover crash-recovery process");
    }
    let exhausted = store
        .get_process_snapshot(GetProcessSnapshotRequest {
            scope,
            process_id: exhausted_id,
        })
        .await
        .expect("exhausted snapshot");
    assert_eq!(exhausted.status, ProcessLifecycleStatus::Failed);
    assert_eq!(
        exhausted.failure.as_ref().map(|failure| failure.category()),
        Some("crash_retry_exhausted")
    );
}

#[tokio::test]
async fn process_journal_store_rejects_wrong_lease() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let process_id = ProcessId::new();
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit process");
    let mut claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string()),
            scope_filter: Some(scope),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process");
    let claim = claimed.pop().expect("claimed process");
    let error = store
        .complete_process(ProcessStateTransitionRequest {
            lease: ProcessLeaseRequest {
                process_id,
                worker_id: claim.worker_id,
                lease_token: ProcessLeaseToken::from_trusted(
                    ProcessId::new().as_uuid().to_string(),
                ),
            },
            metadata: None,
        })
        .await
        .expect_err("wrong lease must fail");
    assert!(error.to_string().contains("lease is invalid"));
}

#[tokio::test]
async fn process_control_is_scoped_atomic_and_process_kind_neutral() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let process_id = ProcessId::new();
    let submitted = submit_internal_process(&store, &scope, process_id).await;
    let worker_id = ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string());
    let mut claimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id,
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim process");
    let claim = claimed.pop().expect("claimed process");
    let suspended = store
        .suspend_process(SuspendProcessRequest {
            process_id,
            worker_id: claim.worker_id,
            lease_token: claim.lease_token,
            checkpoint_ref: ProcessCheckpointRef::from_trusted("checkpoint:control"),
            suspension: ProcessSuspension {
                kind: ProcessSuspensionKind::ExternalProcess,
                gate_ref: None,
                activity_id: None,
                credential_requirements: Vec::new(),
                detail: None,
            },
            metadata: None,
        })
        .await
        .expect("suspend process");

    let stale = store
        .resume_process(ResumeProcessRequest {
            scope: scope.clone(),
            process_id,
            operation_id: None,
            expected_cursor: Some(submitted.journal_cursor),
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect_err("stale resume must fail");
    assert!(stale.to_string().contains("changed after cursor"));

    let mut wrong_scope = scope.clone();
    wrong_scope.user_id = UserId::new("other-user").expect("other user");
    let unauthorized = store
        .resume_process(ResumeProcessRequest {
            scope: wrong_scope,
            process_id,
            operation_id: None,
            expected_cursor: Some(suspended.journal_cursor),
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect_err("cross-scope resume must not disclose process");
    assert!(unauthorized.to_string().contains("unknown process"));

    let resumed = store
        .resume_process(ResumeProcessRequest {
            scope: scope.clone(),
            process_id,
            operation_id: Some(ironclaw_processes::ProcessOperationId::from_trusted(
                "resume:control",
            )),
            expected_cursor: Some(suspended.journal_cursor),
            checkpoint_ref: None,
            metadata: Some(json!({"resumed": true})),
        })
        .await
        .expect("resume process");
    assert!(resumed.changed);
    assert_eq!(resumed.state.status, ProcessLifecycleStatus::Queued);
    assert!(resumed.state.suspension.is_none());
    assert_eq!(resumed.state.metadata["resumed"], true);
    let replayed = store
        .resume_process(ResumeProcessRequest {
            scope: scope.clone(),
            process_id,
            operation_id: Some(ironclaw_processes::ProcessOperationId::from_trusted(
                "resume:control",
            )),
            expected_cursor: Some(suspended.journal_cursor),
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect("replay resume");
    assert_eq!(replayed, resumed);

    let mut reclaimed = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted(ProcessId::new().as_uuid().to_string()),
            scope_filter: Some(scope.clone()),
            process_id_filter: None,
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("reclaim process");
    let claim = reclaimed.pop().expect("reclaimed process");
    let cancel_requested = store
        .request_cancel_process(CancelProcessRequest {
            scope: scope.clone(),
            process_id,
            operation_id: None,
            reason: Some("operator request".to_string()),
        })
        .await
        .expect("request cancellation");
    assert_eq!(
        cancel_requested.state.status,
        ProcessLifecycleStatus::CancelRequested
    );
    assert!(cancel_requested.state.lease.is_some());
    let cancelled = store
        .cancel_process(ProcessStateTransitionRequest {
            lease: ProcessLeaseRequest {
                process_id,
                worker_id: claim.worker_id,
                lease_token: claim.lease_token,
            },
            metadata: None,
        })
        .await
        .expect("complete cancellation");
    assert_eq!(cancelled.status, ProcessLifecycleStatus::Cancelled);

    let failed_during_cancel_id = ProcessId::new();
    submit_internal_process(&store, &scope, failed_during_cancel_id).await;
    let failed_during_cancel_claim = store
        .claim_next_processes(ClaimProcessesRequest {
            worker_id: ProcessWorkerId::from_trusted("cancel-failure-worker"),
            scope_filter: Some(scope.clone()),
            process_id_filter: Some(failed_during_cancel_id),
            process_kind_filter: None,
            max_processes: 1,
        })
        .await
        .expect("claim cancellation-race process")
        .pop()
        .expect("cancellation-race claim");
    store
        .request_cancel_process(CancelProcessRequest {
            scope: scope.clone(),
            process_id: failed_during_cancel_id,
            operation_id: None,
            reason: Some("operator request".to_string()),
        })
        .await
        .expect("request cancellation before runner failure");
    let converged = store
        .fail_process(FailProcessRequest {
            process_id: failed_during_cancel_id,
            worker_id: failed_during_cancel_claim.worker_id,
            lease_token: failed_during_cancel_claim.lease_token,
            failure: SanitizedFailure::new("runner_failed_during_cancel")
                .expect("sanitized failure"),
            recovery: ironclaw_processes::ProcessFailureRecovery::Terminal,
            checkpoint_ref: None,
            metadata: None,
        })
        .await
        .expect("runner failure must converge cancellation");
    assert_eq!(converged.status, ProcessLifecycleStatus::Cancelled);
    assert!(converged.lease.is_none());
    assert!(converged.failure.is_none());

    let stopped_id = ProcessId::new();
    submit_internal_process(&store, &scope, stopped_id).await;
    let stopped = store
        .stop_process(StopProcessRequest {
            scope: scope.clone(),
            process_id: stopped_id,
            operation_id: None,
            reason: Some("shutdown".to_string()),
        })
        .await
        .expect("stop process");
    assert_eq!(stopped.state.status, ProcessLifecycleStatus::Stopped);

    let killed_id = ProcessId::new();
    submit_internal_process(&store, &scope, killed_id).await;
    let killed = store
        .kill_process(KillProcessRequest {
            scope,
            process_id: killed_id,
            operation_id: None,
            reason: Some("forced shutdown".to_string()),
        })
        .await
        .expect("kill process");
    assert_eq!(killed.state.status, ProcessLifecycleStatus::Killed);
}

#[tokio::test]
async fn exclusive_process_submission_uses_authoritative_live_projection() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let first_id = ProcessId::new();
    let request = |process_id| SubmitProcessRequest {
        process_id,
        process_kind: ProcessKind::AgentTurn,
        scope: scope.clone(),
        exclusive_within_scope: true,
        operation_id: None,
        owner_user_id: Some(scope.user_id.clone()),
        concurrency_class: None,
        parent_process_id: None,
        root_process_id: None,
        spawn_tree_descendant_cap: None,
        dependency: None,
        checkpoint_ref: None,
        input: None,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    };
    store
        .submit_process(request(first_id))
        .await
        .expect("submit exclusive process");
    let conflict = store
        .submit_process(request(ProcessId::new()))
        .await
        .expect_err("second live process in scope must conflict");
    assert!(conflict.to_string().contains(&first_id.to_string()));

    store
        .stop_process(StopProcessRequest {
            scope: scope.clone(),
            process_id: first_id,
            operation_id: None,
            reason: None,
        })
        .await
        .expect("stop first process");
    let replacement = store
        .submit_process(request(ProcessId::new()))
        .await
        .expect("terminal process releases exclusive scope");
    assert_eq!(replacement.status, ProcessLifecycleStatus::Queued);
}

#[tokio::test]
async fn process_input_is_atomic_private_and_scope_bound() {
    let store = ProcessJournalStore::new(in_memory_backed_processes_filesystem());
    let scope = scope();
    let process_id = ProcessId::new();
    let input_ref = ProcessInputRef::new("subagent-goal:v1").expect("input ref");
    let payload = br#"{"task":"inspect the process journal"}"#.to_vec();

    let snapshot = store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::AgentTurn,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: Some(ProcessInputSubmission {
                input_ref: input_ref.clone(),
                payload: ProcessInputPayload::new(payload.clone()).expect("bounded input"),
            }),
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit process with input");

    assert_eq!(snapshot.input_ref.as_ref(), Some(&input_ref));
    let serialized_snapshot = serde_json::to_vec(&snapshot).expect("serialize snapshot");
    assert!(
        !serialized_snapshot
            .windows(payload.len())
            .any(|window| window == payload)
    );

    let stored = store
        .get_process_input(GetProcessInputRequest {
            process_id,
            scope: scope.clone(),
        })
        .await
        .expect("read process input")
        .expect("process input exists");
    assert_eq!(stored.input_ref, input_ref);
    assert_eq!(stored.payload.as_bytes(), payload);

    let mut foreign_scope = scope;
    foreign_scope.user_id = UserId::new("other-user").expect("user");
    assert!(
        store
            .get_process_input(GetProcessInputRequest {
                process_id,
                scope: foreign_scope,
            })
            .await
            .expect("scope-bound input query")
            .is_none()
    );
}

#[test]
fn process_input_payload_is_bounded_and_redacted() {
    let error = ProcessInputPayload::new(vec![0; MAX_PROCESS_INPUT_PAYLOAD_BYTES + 1])
        .expect_err("oversized process input must fail");
    assert!(matches!(
        error,
        ProcessJournalError::InputPayloadTooLong {
            actual
        } if actual == MAX_PROCESS_INPUT_PAYLOAD_BYTES + 1
    ));

    let payload = ProcessInputPayload::new(b"private-goal".to_vec()).expect("bounded input");
    let debug = format!("{payload:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("private-goal"));
}

async fn submit_internal_process<F>(
    store: &ProcessJournalStore<F>,
    scope: &ResourceScope,
    process_id: ProcessId,
) -> ironclaw_processes::JournaledProcessSnapshot
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    store
        .submit_process(SubmitProcessRequest {
            process_id,
            process_kind: ProcessKind::Internal,
            scope: scope.clone(),
            exclusive_within_scope: false,
            operation_id: None,
            owner_user_id: Some(scope.user_id.clone()),
            concurrency_class: None,
            parent_process_id: None,
            root_process_id: None,
            spawn_tree_descendant_cap: None,
            dependency: None,
            checkpoint_ref: None,
            input: None,
            created_at: Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("submit internal process")
}

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-journal").expect("tenant"),
        user_id: UserId::new("user-journal").expect("user"),
        agent_id: Some(AgentId::new("agent-journal").expect("agent")),
        project_id: Some(ProjectId::new("project-journal").expect("project")),
        mission_id: None,
        thread_id: Some(ThreadId::new("thread-journal").expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn in_memory_backed_processes_filesystem() -> std::sync::Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("mount alias"),
        VirtualPath::new("/engine/processes").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    std::sync::Arc::new(ScopedFilesystem::with_fixed_view(
        std::sync::Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}
