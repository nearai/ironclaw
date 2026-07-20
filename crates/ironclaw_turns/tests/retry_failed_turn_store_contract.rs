use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{
    BackendId, BackendKind, CompositeRootFilesystem, ContentKind, InMemoryBackend, IndexPolicy,
    MountDescriptor, RootFilesystem, ScopedFilesystem, StorageClass,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    UserId, VirtualPath,
};
use ironclaw_turns::test_support::in_memory_turn_state_store;
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, CheckpointSchemaId, FilesystemTurnStateStore,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, InMemoryRunProfileResolver,
    LoopCheckpointKind, LoopCheckpointStateRef, LoopCheckpointStore, LoopExitMapping,
    PutLoopCheckpointRequest, ReplyTargetBindingRef, RetryTurnRequest, RunProfileRequest,
    RunProfileVersion, SanitizedFailure, SourceBindingRef, StaticTurnAdmissionLimitProvider,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor, TurnAdmissionAxisKind, TurnError,
    TurnIdempotencyOperationKind, TurnIdempotencyOutcomeKind, TurnIdempotencyReplay,
    TurnLeaseToken, TurnPersistenceSnapshot, TurnRunId, TurnRunnerId, TurnScope, TurnStateStore,
    TurnStatus,
    runner::{
        ApplyValidatedLoopExitRequest, ClaimRunRequest, ClaimedTurnRun, FailRunRequest,
        RecoverExpiredLeasesRequest, TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

fn engine_filesystem() -> InMemoryBackend {
    InMemoryBackend::new()
}

fn engine_mount_descriptor<F>(backend: &F) -> MountDescriptor
where
    F: RootFilesystem,
{
    MountDescriptor {
        virtual_root: VirtualPath::new("/engine").unwrap(),
        backend_id: BackendId::new("test-retry-turn-state").unwrap(),
        backend_kind: BackendKind::MemoryDocuments,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::StructuredRecord,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: backend.capabilities(),
    }
}

fn catalog_root<F>(backend: Arc<F>) -> Arc<CompositeRootFilesystem>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    root.mount(engine_mount_descriptor(backend.as_ref()), backend)
        .unwrap();
    Arc::new(root)
}

fn scoped_turns_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<CompositeRootFilesystem>>
where
    F: RootFilesystem + 'static,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("alias"),
        VirtualPath::new("/engine/tenants/test-tenant/users/test-user/turns").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        catalog_root(backend),
        mounts,
    ))
}

fn scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant1").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

fn submit_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        requested_model: None,
        scope: scope(thread),
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{thread}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 6, 12, 9, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

fn retry_request(thread: &str, run_id: TurnRunId, idempotency_key: &str) -> RetryTurnRequest {
    RetryTurnRequest {
        scope: scope(thread),
        actor: actor(),
        run_id,
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

async fn submit_and_claim<S>(store: &S, thread: &str, idempotency_key: &str) -> ClaimedTurnRun
where
    S: TurnStateStore + TurnRunTransitionPort + ?Sized,
{
    let resolver = InMemoryRunProfileResolver::default();
    let response = store
        .submit_turn(
            submit_request(thread, idempotency_key),
            &AllowAllTurnAdmissionPolicy,
            &resolver,
        )
        .await
        .unwrap();
    let run_id = accepted_run_id(&response);
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(thread)),
        })
        .await
        .unwrap()
        .expect("submitted run should be claimable");
    assert_eq!(claimed.state.run_id, run_id);
    claimed
}

async fn put_loop_checkpoint<S>(
    store: &S,
    claimed: &ClaimedTurnRun,
    kind: LoopCheckpointKind,
) -> ironclaw_turns::LoopCheckpointRecord
where
    S: LoopCheckpointStore + ?Sized,
{
    store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: claimed.state.scope.clone(),
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            state_ref: LoopCheckpointStateRef::new(format!(
                "checkpoint:{}:retry_state",
                claimed.state.run_id
            ))
            .unwrap(),
            schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            schema_version: RunProfileVersion::new(1),
            kind,
            gate_ref: None,
        })
        .await
        .unwrap()
}

async fn fail_claimed_run<S>(store: &S, claimed: &ClaimedTurnRun) -> ironclaw_turns::TurnRunState
where
    S: TurnRunTransitionPort + ?Sized,
{
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            model_usage: None,
            run_id: claimed.state.run_id,
            runner_id: claimed.runner_id,
            lease_token: claimed.lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed {
                failure: SanitizedFailure::new("model_error").unwrap(),
            }),
        })
        .await
        .unwrap()
}

async fn complete_claimed_run<S>(
    store: &S,
    claimed: &ClaimedTurnRun,
) -> ironclaw_turns::TurnRunState
where
    S: TurnRunTransitionPort + ?Sized,
{
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            model_usage: None,
            run_id: claimed.state.run_id,
            runner_id: claimed.runner_id,
            lease_token: claimed.lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed),
        })
        .await
        .unwrap()
}

async fn seed_failed_run_with_checkpoint<S>(
    store: &S,
    thread: &str,
    idempotency_key: &str,
    kind: LoopCheckpointKind,
) -> (TurnRunId, ironclaw_turns::LoopCheckpointRecord)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let claimed = submit_and_claim(store, thread, idempotency_key).await;
    let checkpoint = put_loop_checkpoint(store, &claimed, kind).await;
    let failed = fail_claimed_run(store, &claimed).await;
    assert_eq!(failed.status, TurnStatus::Failed);
    // Failed runs advertise only a stored resumable checkpoint
    // (BeforeModel/BeforeBlock). A non-resumable checkpoint is ignored so
    // retryability matches what retry_turn would accept.
    let expected_checkpoint = matches!(
        kind,
        LoopCheckpointKind::BeforeModel | LoopCheckpointKind::BeforeBlock
    )
    .then_some(checkpoint.checkpoint_id);
    assert_eq!(failed.checkpoint_id, expected_checkpoint);
    assert_eq!(
        failed.failure.as_ref().map(|failure| failure.category()),
        Some("model_error")
    );
    (claimed.state.run_id, checkpoint)
}

async fn assert_retry_happy_path_spawns_claimable_checkpointed_run<S>(
    store: &S,
    thread: &str,
    submit_idem: &str,
    retry_idem: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let (failed_run_id, source_checkpoint) = seed_failed_run_with_checkpoint(
        store,
        thread,
        submit_idem,
        LoopCheckpointKind::BeforeModel,
    )
    .await;

    let response = store
        .retry_turn(retry_request(thread, failed_run_id, retry_idem))
        .await
        .unwrap();

    assert_ne!(response.run_id, failed_run_id);
    assert_eq!(response.status, TurnStatus::Queued);

    let retry_state = store
        .get_run_state(GetRunStateRequest {
            scope: scope(thread),
            run_id: response.run_id,
        })
        .await
        .unwrap();
    assert_eq!(retry_state.status, TurnStatus::Queued);
    let retry_checkpoint_id = retry_state
        .checkpoint_id
        .expect("retry run should be seeded with a checkpoint");

    let linked = store
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: scope(thread),
            turn_id: retry_state.turn_id,
            run_id: response.run_id,
            checkpoint_id: retry_checkpoint_id,
        })
        .await
        .unwrap()
        .expect("retry checkpoint metadata should be linked to the new run");
    assert_eq!(linked.kind, source_checkpoint.kind);
    assert_eq!(linked.state_ref, source_checkpoint.state_ref);

    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(thread)),
        })
        .await
        .unwrap()
        .expect("retry run should be claimable");
    assert_eq!(claimed.state.run_id, response.run_id);
    assert_eq!(claimed.state.checkpoint_id, Some(retry_checkpoint_id));
}

async fn assert_retry_leaves_source_failed_run_unchanged<S>(
    store: &S,
    thread: &str,
    submit_idem: &str,
    retry_idem: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let (failed_run_id, _) = seed_failed_run_with_checkpoint(
        store,
        thread,
        submit_idem,
        LoopCheckpointKind::BeforeModel,
    )
    .await;
    let before_retry = store
        .get_run_state(GetRunStateRequest {
            scope: scope(thread),
            run_id: failed_run_id,
        })
        .await
        .unwrap();

    let retry = store
        .retry_turn(retry_request(thread, failed_run_id, retry_idem))
        .await
        .unwrap();
    assert_ne!(retry.run_id, failed_run_id);

    let after_retry = store
        .get_run_state(GetRunStateRequest {
            scope: scope(thread),
            run_id: failed_run_id,
        })
        .await
        .unwrap();
    assert_eq!(
        after_retry, before_retry,
        "retry must create a new run without mutating the source failed run"
    );
}

async fn assert_failed_transition_uses_latest_resumable_checkpoint<S>(
    store: &S,
    thread: &str,
    submit_idem: &str,
    retry_idem: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let claimed = submit_and_claim(store, thread, submit_idem).await;
    let checkpoint = put_loop_checkpoint(store, &claimed, LoopCheckpointKind::BeforeModel).await;

    let failed = fail_claimed_run(store, &claimed).await;
    assert_eq!(
        failed.checkpoint_id,
        Some(checkpoint.checkpoint_id),
        "failed transition should preserve the latest resumable checkpoint"
    );

    let retry = store
        .retry_turn(retry_request(thread, claimed.state.run_id, retry_idem))
        .await
        .expect("stored retry checkpoint should make failed run retryable");
    assert_ne!(retry.run_id, claimed.state.run_id);
    assert_eq!(retry.status, TurnStatus::Queued);
}

async fn assert_retry_admission_rejection_is_not_idempotently_replayed<S>(
    store: &S,
    failed_thread: &str,
    blocker_thread: &str,
    retry_idem: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let failed_claimed =
        submit_and_claim(store, failed_thread, "idem-retry-admission-failed").await;
    put_loop_checkpoint(store, &failed_claimed, LoopCheckpointKind::BeforeModel).await;
    let failed = fail_claimed_run(store, &failed_claimed).await;
    assert_eq!(failed.status, TurnStatus::Failed);
    assert!(failed.checkpoint_id.is_some());

    let blocker = submit_and_claim(store, blocker_thread, "idem-retry-admission-blocker").await;
    let denied = store
        .retry_turn(retry_request(failed_thread, failed.run_id, retry_idem))
        .await
        .unwrap_err();
    assert!(matches!(denied, TurnError::AdmissionRejected(_)));

    complete_claimed_run(store, &blocker).await;

    let retried = store
        .retry_turn(retry_request(failed_thread, failed.run_id, retry_idem))
        .await
        .expect("admission rejection must not be cached for retry idempotency");
    assert_ne!(retried.run_id, failed.run_id);
    assert_eq!(retried.status, TurnStatus::Queued);
}

async fn assert_retry_rejections_and_idempotency<S>(store: &S, prefix: &str)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let queued = store
        .submit_turn(
            submit_request(
                &format!("{prefix}-non-failed"),
                &format!("idem-{prefix}-non-failed"),
            ),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    let queued_run_id = accepted_run_id(&queued);
    let queued_error = store
        .retry_turn(retry_request(
            &format!("{prefix}-non-failed"),
            queued_run_id,
            &format!("idem-{prefix}-non-failed-retry"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        queued_error,
        TurnError::RunNotRetryable {
            run_id: queued_run_id
        }
    );
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(&format!("{prefix}-non-failed"))),
        })
        .await
        .unwrap()
        .expect("cleanup claim should consume queued non-failed run");

    let no_checkpoint_thread = format!("{prefix}-no-checkpoint");
    let no_checkpoint_claimed = submit_and_claim(
        store,
        &no_checkpoint_thread,
        &format!("idem-{prefix}-no-checkpoint"),
    )
    .await;
    fail_claimed_run(store, &no_checkpoint_claimed).await;
    let no_checkpoint_error = store
        .retry_turn(retry_request(
            &no_checkpoint_thread,
            no_checkpoint_claimed.state.run_id,
            &format!("idem-{prefix}-no-checkpoint-retry"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        no_checkpoint_error,
        TurnError::RunNotRetryable {
            run_id: no_checkpoint_claimed.state.run_id
        }
    );

    let side_effect_thread = format!("{prefix}-side-effect");
    let (side_effect_run_id, _) = seed_failed_run_with_checkpoint(
        store,
        &side_effect_thread,
        &format!("idem-{prefix}-side-effect"),
        LoopCheckpointKind::BeforeSideEffect,
    )
    .await;
    let side_effect_error = store
        .retry_turn(retry_request(
            &side_effect_thread,
            side_effect_run_id,
            &format!("idem-{prefix}-side-effect-retry"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        side_effect_error,
        TurnError::RunNotRetryable {
            run_id: side_effect_run_id
        }
    );

    let idempotent_thread = format!("{prefix}-idempotent");
    let (failed_run_id, _) = seed_failed_run_with_checkpoint(
        store,
        &idempotent_thread,
        &format!("idem-{prefix}-idempotent-submit"),
        LoopCheckpointKind::BeforeBlock,
    )
    .await;
    let retry = retry_request(
        &idempotent_thread,
        failed_run_id,
        &format!("idem-{prefix}-retry-replay"),
    );
    let first = store.retry_turn(retry.clone()).await.unwrap();
    let second = store.retry_turn(retry).await.unwrap();
    assert_eq!(second, first);

    let stale_error = store
        .retry_turn(retry_request(
            &idempotent_thread,
            failed_run_id,
            &format!("idem-{prefix}-retry-stale"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        stale_error,
        TurnError::RunNotRetryable {
            run_id: failed_run_id
        }
    );

    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(&idempotent_thread)),
        })
        .await
        .unwrap()
        .expect("idempotent retry should spawn exactly one queued run");
    assert_eq!(claimed.state.run_id, first.run_id);
    let duplicate = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(&idempotent_thread)),
        })
        .await
        .unwrap();
    assert!(duplicate.is_none());
}

async fn assert_retry_reacquires_thread_active_lock<S>(store: &S, prefix: &str)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let thread = format!("{prefix}-thread-lock");
    let (failed_run_id, _) = seed_failed_run_with_checkpoint(
        store,
        &thread,
        &format!("idem-{prefix}-thread-lock-submit"),
        LoopCheckpointKind::BeforeModel,
    )
    .await;

    let retry = store
        .retry_turn(retry_request(
            &thread,
            failed_run_id,
            &format!("idem-{prefix}-thread-lock-retry"),
        ))
        .await
        .unwrap();
    let submit_error = store
        .submit_turn(
            submit_request(&thread, &format!("idem-{prefix}-thread-lock-second-submit")),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap_err();

    assert_eq!(
        submit_error,
        TurnError::ThreadBusy(ThreadBusy {
            active_run_id: retry.run_id,
            status: TurnStatus::Queued,
            event_cursor: retry.event_cursor,
        })
    );
}

struct RetryBusyScenario {
    thread: String,
    retry: RetryTurnRequest,
    busy: ThreadBusy,
}

async fn create_retry_thread_busy_record<S>(store: &S, prefix: &str) -> RetryBusyScenario
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let thread = format!("{prefix}-retry-busy");
    let (failed_run_id, _) = seed_failed_run_with_checkpoint(
        store,
        &thread,
        &format!("idem-{prefix}-retry-busy-submit"),
        LoopCheckpointKind::BeforeModel,
    )
    .await;

    let blocking = store
        .submit_turn(
            submit_request(&thread, &format!("idem-{prefix}-retry-busy-blocking")),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted {
        run_id: blocking_run_id,
        status: blocking_status,
        event_cursor: blocking_cursor,
        ..
    } = blocking;
    assert_eq!(blocking_status, TurnStatus::Queued);

    let retry = retry_request(
        &thread,
        failed_run_id,
        &format!("idem-{prefix}-retry-busy-retry"),
    );
    let busy = ThreadBusy {
        active_run_id: blocking_run_id,
        status: blocking_status,
        event_cursor: blocking_cursor,
    };
    let error = store.retry_turn(retry.clone()).await.unwrap_err();
    assert_eq!(error, TurnError::ThreadBusy(busy.clone()));

    RetryBusyScenario {
        thread,
        retry,
        busy,
    }
}

fn assert_retry_busy_record_is_not_permanent_error(
    snapshot: &TurnPersistenceSnapshot,
    scenario: &RetryBusyScenario,
) {
    let record = snapshot
        .idempotency_records
        .iter()
        .find(|record| {
            record.operation == TurnIdempotencyOperationKind::Retry
                && record.run_id == Some(scenario.retry.run_id)
                && record.key == scenario.retry.idempotency_key
        })
        .expect("retry ThreadBusy attempt should be retained as a non-replayable record");
    assert_eq!(record.outcome, TurnIdempotencyOutcomeKind::ThreadBusy);
    assert!(
        matches!(
            &record.replay,
            TurnIdempotencyReplay::RetryThreadBusy(busy) if busy == &scenario.busy
        ),
        "retry ThreadBusy must not be stored as permanent Error replay: {record:?}"
    );
    assert!(
        record.replay_retry().is_none(),
        "retry ThreadBusy must not be replayable"
    );
}

async fn assert_retry_succeeds_after_busy_run_completes<S>(store: &S, scenario: RetryBusyScenario)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let blocking = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope(&scenario.thread)),
        })
        .await
        .unwrap()
        .expect("blocking run should be claimable");
    assert_eq!(blocking.state.run_id, scenario.busy.active_run_id);
    let completed = complete_claimed_run(store, &blocking).await;
    assert_eq!(completed.status, TurnStatus::Completed);

    let retry = store.retry_turn(scenario.retry.clone()).await.unwrap();
    assert_ne!(retry.run_id, scenario.retry.run_id);
    assert_eq!(retry.status, TurnStatus::Queued);
}

#[tokio::test]
async fn inmemory_retry_failed_turn_spawns_claimable_checkpointed_run() {
    let store = in_memory_turn_state_store();
    assert_retry_happy_path_spawns_claimable_checkpointed_run(
        &store,
        "thread-memory-retry-happy",
        "idem-memory-retry-happy-submit",
        "idem-memory-retry-happy",
    )
    .await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_spawns_claimable_checkpointed_run() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_happy_path_spawns_claimable_checkpointed_run(
        &store,
        "thread-filesystem-retry-happy",
        "idem-filesystem-retry-happy-submit",
        "idem-filesystem-retry-happy",
    )
    .await;
}

#[tokio::test]
async fn inmemory_retry_failed_turn_leaves_source_failed_run_unchanged() {
    let store = in_memory_turn_state_store();
    assert_retry_leaves_source_failed_run_unchanged(
        &store,
        "thread-memory-retry-source-unchanged",
        "idem-memory-retry-source-unchanged-submit",
        "idem-memory-retry-source-unchanged",
    )
    .await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_leaves_source_failed_run_unchanged() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_leaves_source_failed_run_unchanged(
        &store,
        "thread-filesystem-retry-source-unchanged",
        "idem-filesystem-retry-source-unchanged-submit",
        "idem-filesystem-retry-source-unchanged",
    )
    .await;
}

#[tokio::test]
async fn inmemory_failed_transition_uses_latest_resumable_checkpoint() {
    let store = in_memory_turn_state_store();
    assert_failed_transition_uses_latest_resumable_checkpoint(
        &store,
        "thread-memory-failed-transition-checkpoint",
        "idem-memory-failed-transition-checkpoint-submit",
        "idem-memory-failed-transition-checkpoint",
    )
    .await;
}

#[tokio::test]
async fn filesystem_failed_transition_uses_latest_resumable_checkpoint() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_failed_transition_uses_latest_resumable_checkpoint(
        &store,
        "thread-filesystem-failed-transition-checkpoint",
        "idem-filesystem-failed-transition-checkpoint-submit",
        "idem-filesystem-failed-transition-checkpoint",
    )
    .await;
}

#[tokio::test]
async fn inmemory_retry_admission_rejection_is_not_idempotently_replayed() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let store = in_memory_turn_state_store().with_admission_limit_provider(Arc::new(limits));
    assert_retry_admission_rejection_is_not_idempotently_replayed(
        &store,
        "thread-memory-retry-admission-failed",
        "thread-memory-retry-admission-blocker",
        "idem-memory-retry-admission",
    )
    .await;
}

#[tokio::test]
async fn filesystem_retry_admission_rejection_is_not_idempotently_replayed() {
    let limits = StaticTurnAdmissionLimitProvider::default()
        .with_total_limit(TurnAdmissionAxisKind::Tenant, 1);
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend))
        .with_admission_limit_provider(Arc::new(limits));
    assert_retry_admission_rejection_is_not_idempotently_replayed(
        &store,
        "thread-filesystem-retry-admission-failed",
        "thread-filesystem-retry-admission-blocker",
        "idem-filesystem-retry-admission",
    )
    .await;
}

#[tokio::test]
async fn inmemory_retry_failed_turn_rejects_invalid_sources_and_replays_idempotency() {
    let store = in_memory_turn_state_store();
    assert_retry_rejections_and_idempotency(&store, "memory-retry-reject").await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_rejects_invalid_sources_and_replays_idempotency() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_rejections_and_idempotency(&store, "filesystem-retry-reject").await;
}

#[tokio::test]
async fn inmemory_retry_failed_turn_reacquires_thread_active_lock() {
    let store = in_memory_turn_state_store();
    assert_retry_reacquires_thread_active_lock(&store, "memory").await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_reacquires_thread_active_lock() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_reacquires_thread_active_lock(&store, "filesystem").await;
}

#[tokio::test]
async fn inmemory_retry_thread_busy_is_not_permanent_idempotency_replay() {
    let store = in_memory_turn_state_store();
    let scenario = create_retry_thread_busy_record(&store, "memory").await;
    assert_retry_busy_record_is_not_permanent_error(
        &store.persistence_snapshot().await.unwrap(),
        &scenario,
    );
    assert_retry_succeeds_after_busy_run_completes(&store, scenario).await;
}

#[tokio::test]
async fn filesystem_retry_thread_busy_is_not_permanent_idempotency_replay() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    let scenario = create_retry_thread_busy_record(&store, "filesystem").await;
    let snapshot = store.persistence_snapshot().await.unwrap();
    assert_retry_busy_record_is_not_permanent_error(&snapshot, &scenario);
    assert_retry_succeeds_after_busy_run_completes(&store, scenario).await;
}

/// Regression for the lease-expired / externally-failed path: `fail_run`
/// (and lease-expiry recovery, which shares `terminal_transition`) must keep
/// the run retryable from its latest resumable checkpoint, matching the
/// user-facing "Retry the run." summary. A failure with only a non-resumable
/// checkpoint resolves to non-retryable so the projected `retryable` flag and
/// `retry_turn` validation stay in agreement.
async fn assert_external_fail_preserves_retryability<S>(store: &S, thread: &str)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    // Resumable checkpoint present -> failed run stays retryable.
    let retryable_thread = format!("{thread}-extfail-retryable");
    let claimed = submit_and_claim(
        store,
        &retryable_thread,
        &format!("idem-{retryable_thread}"),
    )
    .await;
    let checkpoint = put_loop_checkpoint(store, &claimed, LoopCheckpointKind::BeforeModel).await;
    let failed = store
        .fail_run(FailRunRequest {
            run_id: claimed.state.run_id,
            runner_id: claimed.runner_id,
            lease_token: claimed.lease_token,
            failure: SanitizedFailure::new("lease_expired").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.checkpoint_id,
        Some(checkpoint.checkpoint_id),
        "external/lease failure must preserve the resumable checkpoint"
    );
    let response = store
        .retry_turn(retry_request(
            &retryable_thread,
            claimed.state.run_id,
            &format!("idem-{retryable_thread}-retry"),
        ))
        .await
        .unwrap();
    assert_ne!(response.run_id, claimed.state.run_id);
    assert_eq!(response.status, TurnStatus::Queued);

    // Only a non-resumable checkpoint present -> failed run is not retryable,
    // and the recorded checkpoint is cleared so the flag matches retry_turn.
    let nonretryable_thread = format!("{thread}-extfail-final");
    let claimed = submit_and_claim(
        store,
        &nonretryable_thread,
        &format!("idem-{nonretryable_thread}"),
    )
    .await;
    put_loop_checkpoint(store, &claimed, LoopCheckpointKind::Final).await;
    let failed = store
        .fail_run(FailRunRequest {
            run_id: claimed.state.run_id,
            runner_id: claimed.runner_id,
            lease_token: claimed.lease_token,
            failure: SanitizedFailure::new("lease_expired").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.checkpoint_id, None,
        "a run with only a non-resumable checkpoint must not advertise retryability"
    );
    let error = store
        .retry_turn(retry_request(
            &nonretryable_thread,
            claimed.state.run_id,
            &format!("idem-{nonretryable_thread}-retry"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TurnError::RunNotRetryable {
            run_id: claimed.state.run_id
        }
    );
}

async fn assert_lease_recovery_preserves_retryability<S>(store: &S, thread: &str)
where
    S: TurnStateStore + TurnRunTransitionPort + LoopCheckpointStore + ?Sized,
{
    let retryable_thread = format!("{thread}-lease-retryable");
    let claimed = submit_and_claim(
        store,
        &retryable_thread,
        &format!("idem-{retryable_thread}"),
    )
    .await;
    let checkpoint = put_loop_checkpoint(store, &claimed, LoopCheckpointKind::BeforeModel).await;
    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + chrono::Duration::seconds(120),
            scope_filter: Some(scope(&retryable_thread)),
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::Failed);
    assert_eq!(
        recovered.recovered[0].checkpoint_id,
        Some(checkpoint.checkpoint_id),
        "lease-expired failure must preserve the latest resumable checkpoint"
    );
    let response = store
        .retry_turn(retry_request(
            &retryable_thread,
            claimed.state.run_id,
            &format!("idem-{retryable_thread}-retry"),
        ))
        .await
        .unwrap();
    assert_ne!(response.run_id, claimed.state.run_id);
    assert_eq!(response.status, TurnStatus::Queued);

    let nonretryable_thread = format!("{thread}-lease-final");
    let claimed = submit_and_claim(
        store,
        &nonretryable_thread,
        &format!("idem-{nonretryable_thread}"),
    )
    .await;
    put_loop_checkpoint(store, &claimed, LoopCheckpointKind::Final).await;
    let recovered = store
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc::now() + chrono::Duration::seconds(120),
            scope_filter: Some(scope(&nonretryable_thread)),
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::Failed);
    assert_eq!(
        recovered.recovered[0].checkpoint_id, None,
        "lease-expired failure with only a final checkpoint must not advertise retryability"
    );
    let error = store
        .retry_turn(retry_request(
            &nonretryable_thread,
            claimed.state.run_id,
            &format!("idem-{nonretryable_thread}-retry"),
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        TurnError::RunNotRetryable {
            run_id: claimed.state.run_id
        }
    );
}

#[tokio::test]
async fn inmemory_external_fail_preserves_retryability() {
    let store = in_memory_turn_state_store();
    assert_external_fail_preserves_retryability(&store, "memory").await;
}

#[tokio::test]
async fn filesystem_external_fail_preserves_retryability() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_external_fail_preserves_retryability(&store, "filesystem").await;
}

#[tokio::test]
async fn inmemory_lease_recovery_preserves_retryability() {
    let store = in_memory_turn_state_store();
    assert_lease_recovery_preserves_retryability(&store, "memory").await;
}

#[tokio::test]
async fn filesystem_lease_recovery_preserves_retryability() {
    let backend = engine_filesystem();
    let backend = Arc::new(backend);
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_lease_recovery_preserves_retryability(&store, "filesystem").await;
}
