use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, HostPath, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId,
    ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, CheckpointSchemaId, FilesystemTurnStateStore,
    GetLoopCheckpointRequest, GetRunStateRequest, IdempotencyKey, InMemoryRunProfileResolver,
    InMemoryTurnStateStore, LoopCheckpointKind, LoopCheckpointStateRef, LoopCheckpointStore,
    LoopExitMapping, PutLoopCheckpointRequest, ReplyTargetBindingRef, RetryTurnRequest,
    RunProfileRequest, RunProfileVersion, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, ThreadBusy, TurnActor, TurnError, TurnLeaseToken, TurnRunId, TurnRunnerId,
    TurnScope, TurnStateStore, TurnStatus,
    runner::{
        ApplyValidatedLoopExitRequest, ClaimRunRequest, ClaimedTurnRun, TurnRunTransitionPort,
        TurnRunnerOutcome,
    },
};

fn engine_filesystem() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn scoped_turns_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("alias"),
        VirtualPath::new("/engine/tenants/test-tenant/users/test-user/turns").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
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

async fn fail_claimed_run<S>(
    store: &S,
    claimed: &ClaimedTurnRun,
    resume_checkpoint_id: Option<ironclaw_turns::TurnCheckpointId>,
) -> ironclaw_turns::TurnRunState
where
    S: TurnRunTransitionPort + ?Sized,
{
    store
        .apply_validated_loop_exit(ApplyValidatedLoopExitRequest {
            run_id: claimed.state.run_id,
            runner_id: claimed.runner_id,
            lease_token: claimed.lease_token,
            mapping: LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed {
                failure: SanitizedFailure::new("model_error").unwrap(),
                explanation_message_refs: Vec::new(),
                resume_checkpoint_id,
            }),
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
    let failed = fail_claimed_run(store, &claimed, Some(checkpoint.checkpoint_id)).await;
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.checkpoint_id, Some(checkpoint.checkpoint_id));
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
    fail_claimed_run(store, &no_checkpoint_claimed, None).await;
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

#[tokio::test]
async fn inmemory_retry_failed_turn_spawns_claimable_checkpointed_run() {
    let store = InMemoryTurnStateStore::default();
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
    let backend = Arc::new(engine_filesystem());
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
async fn inmemory_retry_failed_turn_rejects_invalid_sources_and_replays_idempotency() {
    let store = InMemoryTurnStateStore::default();
    assert_retry_rejections_and_idempotency(&store, "memory-retry-reject").await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_rejects_invalid_sources_and_replays_idempotency() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_rejections_and_idempotency(&store, "filesystem-retry-reject").await;
}

#[tokio::test]
async fn inmemory_retry_failed_turn_reacquires_thread_active_lock() {
    let store = InMemoryTurnStateStore::default();
    assert_retry_reacquires_thread_active_lock(&store, "memory").await;
}

#[tokio::test]
async fn filesystem_retry_failed_turn_reacquires_thread_active_lock() {
    let backend = Arc::new(engine_filesystem());
    let store = FilesystemTurnStateStore::new(scoped_turns_fs(backend));
    assert_retry_reacquires_thread_active_lock(&store, "filesystem").await;
}
