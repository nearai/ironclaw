#![cfg(any(feature = "libsql", feature = "postgres"))]
#![cfg_attr(
    all(feature = "postgres", not(feature = "libsql")),
    allow(dead_code, unused_imports)
)]

use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CheckpointSchemaId, DefaultTurnCoordinator,
    GetLoopCheckpointRequest, IdempotencyKey, InMemoryRunProfileResolver,
    InMemoryTurnStateStoreLimits, LoopCancelled, LoopCancelledReasonKind, LoopCheckpointStateRef,
    LoopCheckpointStore, LoopCompleted, LoopCompletionKind, LoopDiagnosticRef, LoopExit,
    LoopExitId, LoopExitValidationPolicy, LoopFailed, LoopFailureKind, LoopMessageRef,
    LoopUsageSummaryRef, PutLoopCheckpointRequest, ReplyTargetBindingRef, ResolvedRunProfile,
    RunProfileRequest, RunProfileResolutionError, RunProfileResolutionRequest, RunProfileResolver,
    RunProfileVersion, SanitizedCancelReason, SanitizedFailure, SourceBindingRef,
    StaticTurnAdmissionLimitProvider, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor,
    TurnAdmissionAxisKind, TurnAdmissionCapacityDenial, TurnCoordinator, TurnError, TurnEventKind,
    TurnEventProjectionCursor, TurnEventProjectionError, TurnEventProjectionRequest,
    TurnEventProjectionService, TurnEventProjectionSource, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    events::EventCursor,
    runner::{
        ApplyLoopExitRequest, ClaimRunRequest, CompleteRunRequest, HeartbeatRequest,
        RecoverExpiredLeasesRequest, TurnRunTransitionPort, apply_loop_exit,
    },
};

#[cfg(feature = "libsql")]
use ironclaw_turns::LibSqlTurnStateStore;
#[cfg(feature = "postgres")]
use ironclaw_turns::PostgresTurnStateStore;

struct BlockingRunProfileResolver {
    started: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl BlockingRunProfileResolver {
    fn new(started: mpsc::Sender<()>, release: mpsc::Receiver<()>) -> Self {
        Self {
            started,
            release: Mutex::new(release),
        }
    }
}

#[async_trait::async_trait]
impl RunProfileResolver for BlockingRunProfileResolver {
    async fn resolve_run_profile(
        &self,
        request: RunProfileResolutionRequest,
    ) -> Result<ResolvedRunProfile, RunProfileResolutionError> {
        let _ = self.started.send(());
        self.release.lock().unwrap().recv().unwrap();
        InMemoryRunProfileResolver::default()
            .resolve_run_profile(request)
            .await
    }
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_event_projection_replays_submit_after_reopen_without_raw_refs() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let mut request = submit_request("thread-turn-event-db", "idem-turn-event-db");
    request.accepted_message_ref =
        AcceptedMessageRef::new("message-DB_TURN_RAW_SENTINEL_3022 /tmp/db-turn-private").unwrap();
    request.source_binding_ref =
        SourceBindingRef::new("source-DB_TURN_SOURCE_SENTINEL_3022").unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new("reply-DB_TURN_REPLY_SENTINEL_3022").unwrap();

    let accepted = coordinator.submit_turn(request.clone()).await.unwrap();
    let run_id = accepted_run_id(&accepted);

    let reopened = Arc::new(LibSqlTurnStateStore::new(db));
    let projection = TurnEventProjectionService::new(reopened);
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope,
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.entries[0].kind, TurnEventKind::Submitted);
    assert_eq!(snapshot.entries[0].run_id, run_id);
    assert_eq!(snapshot.entries[0].status, TurnStatus::Queued);

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert_no_forbidden_turn_event_content(
        "libSQL turn lifecycle projection",
        &serialized,
        &[
            "DB_TURN_RAW_SENTINEL_3022",
            "/tmp/db-turn-private",
            "DB_TURN_SOURCE_SENTINEL_3022",
            "DB_TURN_REPLY_SENTINEL_3022",
        ],
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_event_projection_replays_failed_terminal_after_reopen_without_raw_refs() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();

    assert_db_turn_event_projection_replays_failed_terminal_after_reopen_without_raw_refs(
        store,
        move || Arc::new(LibSqlTurnStateStore::new(db)),
        "thread-turn-failed-events-db",
        "idem-turn-failed-db",
        "DB_TURN_FAILED",
    )
    .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_event_projection_replays_cancelled_terminal_after_reopen_without_raw_refs() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();

    assert_db_turn_event_projection_replays_cancelled_terminal_after_reopen_without_raw_refs(
        store,
        move || Arc::new(LibSqlTurnStateStore::new(db)),
        "thread-turn-cancelled-events-db",
        "idem-turn-cancelled-db",
        "DB_TURN_CANCELLED",
    )
    .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_event_projection_replays_gap_rebase_after_reopen() {
    let (db, _dir) = libsql_db().await;
    let limits = InMemoryTurnStateStoreLimits {
        max_events: 2,
        ..InMemoryTurnStateStoreLimits::default()
    };
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()).with_limits(limits));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let request = submit_request("thread-turn-event-db-gap", "idem-turn-event-db-gap");
    let accepted = coordinator.submit_turn(request.clone()).await.unwrap();
    let run_id = accepted_run_id(&accepted);
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    store
        .heartbeat(HeartbeatRequest {
            run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let projection = TurnEventProjectionService::new(Arc::new(LibSqlTurnStateStore::new(db)));
    let pruned_origin = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: Some(TurnEventProjectionCursor::origin_for_scope(
                request.scope.clone(),
            )),
            limit: 10,
        })
        .await
        .expect_err("libSQL retained turn event tail must persist rebase metadata");
    assert!(matches!(
        pruned_origin,
        TurnEventProjectionError::RebaseRequired { .. }
    ));

    let fabricated = projection
        .updates(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: Some(TurnEventProjectionCursor::for_scope(
                request.scope,
                EventCursor(999),
            )),
            limit: 10,
        })
        .await
        .expect_err("libSQL turn event projection must reject beyond-head cursors");
    assert!(matches!(
        fabricated,
        TurnEventProjectionError::RebaseRequired { .. }
    ));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_persists_submit_and_busy_across_instances() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    let accepted = coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    let run_id = accepted_run_id(&accepted);

    let reopened = Arc::new(LibSqlTurnStateStore::new(db));
    let reopened_coordinator = DefaultTurnCoordinator::new(reopened);
    let busy = reopened_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        busy,
        TurnError::ThreadBusy(ThreadBusy {
            active_run_id,
            status: TurnStatus::Queued,
            ..
        }) if active_run_id == run_id
    ));

    let duplicate = reopened_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-a"))
        .await
        .unwrap();
    assert_eq!(duplicate, accepted);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_loop_checkpoint_store_persists_mapping_across_instances() {
    let (db, _dir) = libsql_db().await;
    let store = LibSqlTurnStateStore::new(db.clone());
    store.run_migrations().await.unwrap();
    let scope = scope("thread-loop-checkpoint-db");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_ref = LoopCheckpointStateRef::new("checkpoint:db-loop-state").unwrap();
    let schema_id = CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap();
    let schema_version = RunProfileVersion::new(3);

    let checkpoint = store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_ref.clone(),
            schema_id: schema_id.clone(),
            schema_version,
            kind: ironclaw_turns::run_profile::LoopCheckpointKind::BeforeModel,
        })
        .await
        .unwrap();

    let reopened = LibSqlTurnStateStore::new(db);
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap()
        .expect("libSQL checkpoint id mapping should survive store reopen");

    assert_eq!(loaded, checkpoint);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_loop_checkpoint_store_rejects_cross_scope_after_reopen() {
    let (db, _dir) = libsql_db().await;
    let store = LibSqlTurnStateStore::new(db.clone());
    store.run_migrations().await.unwrap();
    let checkpoint_scope = scope("thread-loop-checkpoint-db-scope-a");
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let checkpoint = store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: checkpoint_scope.clone(),
            turn_id,
            run_id,
            state_ref: LoopCheckpointStateRef::new("checkpoint:db-loop-state-scope").unwrap(),
            schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            schema_version: RunProfileVersion::new(3),
            kind: ironclaw_turns::run_profile::LoopCheckpointKind::BeforeBlock,
        })
        .await
        .unwrap();

    let reopened = LibSqlTurnStateStore::new(db);
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: scope("thread-loop-checkpoint-db-scope-b"),
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_persists_admission_reservations_across_instances() {
    let (db, _dir) = libsql_db().await;
    let limits = Arc::new(
        StaticTurnAdmissionLimitProvider::default()
            .with_total_limit(TurnAdmissionAxisKind::Tenant, 1),
    );
    let store = Arc::new(
        LibSqlTurnStateStore::new(db.clone()).with_admission_limit_provider(limits.clone()),
    );
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );

    let reopened = Arc::new(
        LibSqlTurnStateStore::new(db.clone()).with_admission_limit_provider(limits.clone()),
    );
    let reopened_coordinator = DefaultTurnCoordinator::new(reopened.clone());
    let denied = reopened_coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b"))
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(ironclaw_turns::AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Tenant,
                limit: 1,
                active_count: 1,
                ..
            }),
            ..
        })
    ));
    assert_eq!(
        reopened
            .persistence_snapshot()
            .await
            .unwrap()
            .admission_reservations
            .iter()
            .filter(|reservation| !reservation.released)
            .count(),
        1
    );

    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    reopened
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    reopened
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let after_release =
        Arc::new(LibSqlTurnStateStore::new(db).with_admission_limit_provider(limits));
    let after_release_coordinator = DefaultTurnCoordinator::new(after_release.clone());
    after_release_coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-b-after-release"))
        .await
        .unwrap();
    assert_eq!(
        after_release
            .persistence_snapshot()
            .await
            .unwrap()
            .admission_reservations
            .iter()
            .filter(|reservation| !reservation.released)
            .count(),
        1
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_serializes_concurrent_submits_for_same_thread() {
    let (db, _dir) = libsql_db().await;
    let store_a = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store_a.run_migrations().await.unwrap();
    let store_b = Arc::new(LibSqlTurnStateStore::new(db));
    let coordinator_a = DefaultTurnCoordinator::new(store_a.clone());
    let coordinator_b = DefaultTurnCoordinator::new(store_b);

    let (first, second) = tokio::join!(
        coordinator_a.submit_turn(submit_request("thread-a", "idem-submit-a")),
        coordinator_b.submit_turn(submit_request("thread-a", "idem-submit-b")),
    );

    let accepted = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| matches!(result, Ok(SubmitTurnResponse::Accepted { .. })))
        .count();
    let busy = [first.as_ref(), second.as_ref()]
        .into_iter()
        .filter(|result| matches!(result, Err(TurnError::ThreadBusy(_))))
        .count();
    assert_eq!(accepted, 1);
    assert_eq!(busy, 1);

    let snapshot = store_a.persistence_snapshot().await.unwrap();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.runs.len(), 1);
    assert_eq!(snapshot.active_locks.len(), 1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_submit_does_not_hold_write_lock_while_resolving_run_profile() {
    let (db, _dir) = libsql_db().await;
    let store_a = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store_a.run_migrations().await.unwrap();
    let store_b = Arc::new(LibSqlTurnStateStore::new(db));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let resolver = Arc::new(BlockingRunProfileResolver::new(started_tx, release_rx));
    let blocking_coordinator =
        DefaultTurnCoordinator::new(store_a).with_run_profile_resolver(resolver.clone());
    let independent_coordinator = DefaultTurnCoordinator::new(store_b);

    let pending = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(
            blocking_coordinator
                .submit_turn(submit_request("thread-a", "idem-submit-blocking-profile")),
        )
    });
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first submit should start profile resolution");

    let independent = independent_coordinator
        .submit_turn(submit_request("thread-b", "idem-submit-independent"))
        .await
        .unwrap();

    assert!(matches!(independent, SubmitTurnResponse::Accepted { .. }));
    release_tx.send(()).unwrap();
    let pending = pending.join().unwrap().unwrap();
    assert!(matches!(pending, SubmitTurnResponse::Accepted { .. }));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_persists_apply_loop_exit_recovery_across_instances() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope("thread-a")),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);

    let recovered = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            exit: completed_exit("exit:unverified-completed"),
            validation_policy: LoopExitValidationPolicy::recovery_required(),
        },
    )
    .await
    .unwrap();
    assert_eq!(recovered.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        recovered.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_protocol_violation")
    );

    let reopened = Arc::new(LibSqlTurnStateStore::new(db));
    let snapshot = reopened.persistence_snapshot().await.unwrap();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        run.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_protocol_violation")
    );
    let lock = snapshot
        .active_locks
        .iter()
        .find(|lock| lock.run_id == run_id)
        .unwrap();
    assert_eq!(lock.status, TurnStatus::RecoveryRequired);

    let reopened_coordinator = DefaultTurnCoordinator::new(reopened.clone());
    assert!(matches!(
        reopened_coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-after-recovery"))
            .await
            .unwrap_err(),
        TurnError::ThreadBusy(_)
    ));

    let cancelled = reopened_coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-recovered"))
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_persists_cancelled_loop_exit_application() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let recovery_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-cancel-recovery", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let recovery_runner_id = ironclaw_turns::TurnRunnerId::new();
    let recovery_lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: recovery_runner_id,
            lease_token: recovery_lease_token,
            scope_filter: Some(scope("thread-cancel-recovery")),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, recovery_run_id);

    let recovered = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id: recovery_run_id,
            runner_id: recovery_runner_id,
            lease_token: recovery_lease_token,
            exit: LoopExit::cancelled_for_observed_interrupt(
                ironclaw_turns::LoopExitId::new("exit:cancelled-before-recorded").unwrap(),
            ),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_cancellation_observed(),
        },
    )
    .await
    .unwrap();
    assert_eq!(recovered.status, TurnStatus::RecoveryRequired);

    let cancel_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-cancel-recorded", "idem-submit-b"))
            .await
            .unwrap(),
    );
    let cancel_runner_id = ironclaw_turns::TurnRunnerId::new();
    let cancel_lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
            scope_filter: Some(scope("thread-cancel-recorded")),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, cancel_run_id);
    coordinator
        .cancel_run(cancel_request(
            "thread-cancel-recorded",
            cancel_run_id,
            "idem-cancel-recorded",
        ))
        .await
        .unwrap();

    let cancelled = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id: cancel_run_id,
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
            exit: LoopExit::cancelled_for_observed_interrupt(
                ironclaw_turns::LoopExitId::new("exit:cancelled-recorded").unwrap(),
            ),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_cancellation_observed(),
        },
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);

    let reopened = Arc::new(LibSqlTurnStateStore::new(db));
    let snapshot = reopened.persistence_snapshot().await.unwrap();
    let recovery = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == recovery_run_id)
        .unwrap();
    assert_eq!(recovery.status, TurnStatus::RecoveryRequired);
    let cancelled = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == cancel_run_id)
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert_eq!(cancelled.failure, None);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_turn_state_store_persists_runner_recovery_and_cancel_flow() {
    let (db, _dir) = libsql_db().await;
    let store = Arc::new(LibSqlTurnStateStore::new(db.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request("thread-a", "idem-submit-a"))
            .await
            .unwrap(),
    );
    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        })
        .await
        .unwrap()
        .unwrap();

    let snapshot = store.persistence_snapshot().await.unwrap();
    let lease_expires_at = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap()
        .lease_expires_at
        .unwrap();

    let reopened = Arc::new(LibSqlTurnStateStore::new(db));
    let recovered = reopened
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: lease_expires_at + ChronoDuration::milliseconds(1),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(recovered.recovered.len(), 1);
    assert_eq!(recovered.recovered[0].status, TurnStatus::RecoveryRequired);

    let reopened_coordinator = DefaultTurnCoordinator::new(reopened.clone());
    let busy = reopened_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-after-recovery"))
        .await
        .unwrap_err();
    assert!(matches!(busy, TurnError::ThreadBusy(_)));

    let cancelled = reopened_coordinator
        .cancel_run(cancel_request("thread-a", run_id, "idem-cancel-recovered"))
        .await
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);

    let replacement = reopened_coordinator
        .submit_turn(submit_request("thread-a", "idem-submit-replacement"))
        .await
        .unwrap();
    assert!(matches!(replacement, SubmitTurnResponse::Accepted { .. }));
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_event_projection_replays_failed_terminal_after_reopen_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let store = Arc::new(PostgresTurnStateStore::new(pool.clone()));
    store.run_migrations().await.unwrap();

    assert_db_turn_event_projection_replays_failed_terminal_after_reopen_without_raw_refs(
        store,
        move || Arc::new(PostgresTurnStateStore::new(pool)),
        &format!("pg-turn-failed-events-{suffix}"),
        &format!("idem-turn-failed-pg-{suffix}"),
        "PG_TURN_FAILED",
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_event_projection_replays_cancelled_terminal_after_reopen_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let store = Arc::new(PostgresTurnStateStore::new(pool.clone()));
    store.run_migrations().await.unwrap();

    assert_db_turn_event_projection_replays_cancelled_terminal_after_reopen_without_raw_refs(
        store,
        move || Arc::new(PostgresTurnStateStore::new(pool)),
        &format!("pg-turn-cancelled-events-{suffix}"),
        &format!("idem-turn-cancelled-pg-{suffix}"),
        "PG_TURN_CANCELLED",
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_loop_checkpoint_store_persists_mapping_across_instances_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let store = PostgresTurnStateStore::new(pool.clone());
    store.run_migrations().await.unwrap();
    let scope = scope(&format!("thread-loop-checkpoint-postgres-{suffix}"));
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let state_ref =
        LoopCheckpointStateRef::new(format!("checkpoint:pg-loop-state:{suffix}")).unwrap();
    let schema_id = CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap();
    let schema_version = RunProfileVersion::new(3);

    let checkpoint = store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: scope.clone(),
            turn_id,
            run_id,
            state_ref: state_ref.clone(),
            schema_id: schema_id.clone(),
            schema_version,
            kind: ironclaw_turns::run_profile::LoopCheckpointKind::BeforeModel,
        })
        .await
        .unwrap();

    let reopened = PostgresTurnStateStore::new(pool);
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope,
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap()
        .expect("Postgres checkpoint id mapping should survive store reopen");

    assert_eq!(loaded, checkpoint);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_loop_checkpoint_store_rejects_cross_scope_after_reopen_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let store = PostgresTurnStateStore::new(pool.clone());
    store.run_migrations().await.unwrap();
    let checkpoint_scope = scope(&format!("thread-loop-checkpoint-postgres-scope-a-{suffix}"));
    let turn_id = TurnId::new();
    let run_id = TurnRunId::new();
    let checkpoint = store
        .put_loop_checkpoint(PutLoopCheckpointRequest {
            scope: checkpoint_scope.clone(),
            turn_id,
            run_id,
            state_ref: LoopCheckpointStateRef::new(format!(
                "checkpoint:pg-loop-state-scope:{suffix}"
            ))
            .unwrap(),
            schema_id: CheckpointSchemaId::new("interactive_checkpoint_v1").unwrap(),
            schema_version: RunProfileVersion::new(3),
            kind: ironclaw_turns::run_profile::LoopCheckpointKind::BeforeBlock,
        })
        .await
        .unwrap();

    let reopened = PostgresTurnStateStore::new(pool);
    let loaded = reopened
        .get_loop_checkpoint(GetLoopCheckpointRequest {
            scope: scope(&format!("thread-loop-checkpoint-postgres-scope-b-{suffix}")),
            turn_id,
            run_id,
            checkpoint_id: checkpoint.checkpoint_id,
        })
        .await
        .unwrap();

    assert!(loaded.is_none());
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_state_store_persists_submit_and_busy_across_instances_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let thread = format!("pg-thread-{suffix}");
    let store = Arc::new(PostgresTurnStateStore::new(pool.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());

    let accepted = coordinator
        .submit_turn(submit_request(&thread, &format!("idem-submit-a-{suffix}")))
        .await
        .unwrap();
    let run_id = accepted_run_id(&accepted);

    let reopened = Arc::new(PostgresTurnStateStore::new(pool));
    let reopened_coordinator = DefaultTurnCoordinator::new(reopened);
    let busy = reopened_coordinator
        .submit_turn(submit_request(&thread, &format!("idem-submit-b-{suffix}")))
        .await
        .unwrap_err();
    assert!(matches!(
        busy,
        TurnError::ThreadBusy(ThreadBusy {
            active_run_id,
            status: TurnStatus::Queued,
            ..
        }) if active_run_id == run_id
    ));
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_state_store_persists_admission_reservations_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let limits = Arc::new(
        StaticTurnAdmissionLimitProvider::default()
            .with_total_limit(TurnAdmissionAxisKind::Tenant, 1),
    );
    let store = Arc::new(
        PostgresTurnStateStore::new(pool.clone()).with_admission_limit_provider(limits.clone()),
    );
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let tenant_id = TenantId::new(format!("tenant-admission-{suffix}")).unwrap();
    let tenant_submit_request = |thread: &str, idempotency_key: &str| {
        let mut request = submit_request(thread, idempotency_key);
        request.scope.tenant_id = tenant_id.clone();
        request
    };
    let first_thread = format!("pg-admission-a-{suffix}");
    let second_thread = format!("pg-admission-b-{suffix}");
    let first_run_id = accepted_run_id(
        &coordinator
            .submit_turn(tenant_submit_request(
                &first_thread,
                &format!("idem-submit-a-{suffix}"),
            ))
            .await
            .unwrap(),
    );

    let reopened = Arc::new(
        PostgresTurnStateStore::new(pool.clone()).with_admission_limit_provider(limits.clone()),
    );
    let reopened_coordinator = DefaultTurnCoordinator::new(reopened.clone());
    let denied = reopened_coordinator
        .submit_turn(tenant_submit_request(
            &second_thread,
            &format!("idem-submit-b-{suffix}"),
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        denied,
        TurnError::AdmissionRejected(ironclaw_turns::AdmissionRejection {
            capacity_denial: Some(TurnAdmissionCapacityDenial {
                axis_kind: TurnAdmissionAxisKind::Tenant,
                limit: 1,
                active_count: 1,
                ..
            }),
            ..
        })
    ));

    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    let mut first_scope = scope(&first_thread);
    first_scope.tenant_id = tenant_id.clone();
    reopened
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(first_scope),
        })
        .await
        .unwrap()
        .unwrap();
    reopened
        .complete_run(CompleteRunRequest {
            run_id: first_run_id,
            runner_id,
            lease_token,
        })
        .await
        .unwrap();

    let after_release =
        Arc::new(PostgresTurnStateStore::new(pool).with_admission_limit_provider(limits));
    let after_release_coordinator = DefaultTurnCoordinator::new(after_release);
    after_release_coordinator
        .submit_turn(tenant_submit_request(
            &second_thread,
            &format!("idem-submit-b-after-release-{suffix}"),
        ))
        .await
        .unwrap();
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_state_store_persists_apply_loop_exit_recovery_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let thread = format!("pg-recovery-thread-{suffix}");
    let store = Arc::new(PostgresTurnStateStore::new(pool.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(&thread, &format!("idem-submit-{suffix}")))
            .await
            .unwrap(),
    );
    let runner_id = ironclaw_turns::TurnRunnerId::new();
    let lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(scope(&thread)),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);

    apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            exit: completed_exit(&format!("exit:unverified-{suffix}")),
            validation_policy: LoopExitValidationPolicy::recovery_required(),
        },
    )
    .await
    .unwrap();

    let reopened = Arc::new(PostgresTurnStateStore::new(pool));
    let snapshot = reopened.persistence_snapshot().await.unwrap();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .unwrap();
    assert_eq!(run.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        run.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_protocol_violation")
    );
    let lock = snapshot
        .active_locks
        .iter()
        .find(|lock| lock.run_id == run_id)
        .unwrap();
    assert_eq!(lock.status, TurnStatus::RecoveryRequired);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_turn_state_store_persists_cancelled_loop_exit_application_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let suffix = unique_suffix();
    let recovery_thread = format!("pg-cancelled-loop-exit-recovery-{suffix}");
    let cancel_thread = format!("pg-cancelled-loop-exit-recorded-{suffix}");
    let store = Arc::new(PostgresTurnStateStore::new(pool.clone()));
    store.run_migrations().await.unwrap();
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let recovery_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                &recovery_thread,
                &format!("idem-submit-recovery-{suffix}"),
            ))
            .await
            .unwrap(),
    );
    let recovery_runner_id = ironclaw_turns::TurnRunnerId::new();
    let recovery_lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: recovery_runner_id,
            lease_token: recovery_lease_token,
            scope_filter: Some(scope(&recovery_thread)),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, recovery_run_id);

    let recovered = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id: recovery_run_id,
            runner_id: recovery_runner_id,
            lease_token: recovery_lease_token,
            exit: LoopExit::cancelled_for_observed_interrupt(
                ironclaw_turns::LoopExitId::new(format!("exit:cancelled-before-recorded-{suffix}"))
                    .unwrap(),
            ),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_cancellation_observed(),
        },
    )
    .await
    .unwrap();
    assert_eq!(recovered.status, TurnStatus::RecoveryRequired);

    let cancel_run_id = accepted_run_id(
        &coordinator
            .submit_turn(submit_request(
                &cancel_thread,
                &format!("idem-submit-cancel-{suffix}"),
            ))
            .await
            .unwrap(),
    );
    let cancel_runner_id = ironclaw_turns::TurnRunnerId::new();
    let cancel_lease_token = ironclaw_turns::TurnLeaseToken::new();
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
            scope_filter: Some(scope(&cancel_thread)),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, cancel_run_id);
    coordinator
        .cancel_run(cancel_request(
            &cancel_thread,
            cancel_run_id,
            &format!("idem-cancel-recorded-{suffix}"),
        ))
        .await
        .unwrap();

    let cancelled = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id: cancel_run_id,
            runner_id: cancel_runner_id,
            lease_token: cancel_lease_token,
            exit: LoopExit::cancelled_for_observed_interrupt(
                ironclaw_turns::LoopExitId::new(format!("exit:cancelled-recorded-{suffix}"))
                    .unwrap(),
            ),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_cancellation_observed(),
        },
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);

    let reopened = Arc::new(PostgresTurnStateStore::new(pool));
    let snapshot = reopened.persistence_snapshot().await.unwrap();
    let recovery = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == recovery_run_id)
        .unwrap();
    assert_eq!(recovery.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        recovery.failure.as_ref().map(SanitizedFailure::category),
        Some("interrupted_unexpectedly")
    );
    let cancelled = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == cancel_run_id)
        .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);
    assert_eq!(cancelled.failure, None);
    assert!(
        snapshot
            .active_locks
            .iter()
            .all(|lock| lock.run_id != cancel_run_id)
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_turn_state_store_implements_turn_contract_traits() {
    fn assert_state_store<T: ironclaw_turns::TurnStateStore>() {}
    fn assert_runner_port<T: TurnRunTransitionPort>() {}
    fn assert_loop_checkpoint_store<T: LoopCheckpointStore>() {}
    assert_state_store::<PostgresTurnStateStore>();
    assert_runner_port::<PostgresTurnStateStore>();
    assert_loop_checkpoint_store::<PostgresTurnStateStore>();
}

async fn assert_db_turn_event_projection_replays_failed_terminal_after_reopen_without_raw_refs<
    S,
    Reopen,
>(
    store: Arc<S>,
    reopen: Reopen,
    thread: &str,
    idempotency_key_prefix: &str,
    marker_prefix: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + TurnEventProjectionSource + 'static,
    Reopen: FnOnce() -> Arc<S>,
{
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let submit_key = format!("{idempotency_key_prefix}-submit");
    let accepted_marker = format!("{marker_prefix}_ACCEPTED_SENTINEL_3022");
    let source_marker = format!("{marker_prefix}_SOURCE_SENTINEL_3022");
    let reply_marker = format!("{marker_prefix}_REPLY_SENTINEL_3022");
    let diagnostic_marker = format!("{marker_prefix}_FAILURE_REASON_SENTINEL_3022");
    let usage_marker = format!("{marker_prefix}_USAGE_SENTINEL_3022");
    let exit_marker = format!("{marker_prefix}_EXIT_SENTINEL_3022");
    let private_path = format!("/tmp/{marker_prefix}-failed-private");
    let mut request = submit_request(thread, &submit_key);
    request.accepted_message_ref =
        AcceptedMessageRef::new(format!("message-{accepted_marker} {private_path}")).unwrap();
    request.source_binding_ref = SourceBindingRef::new(format!("source-{source_marker}")).unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new(format!("reply-{reply_marker}")).unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();

    let failed = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            exit: LoopExit::Failed(LoopFailed {
                reason_kind: LoopFailureKind::DriverBug,
                checkpoint_id: None,
                usage_summary_ref: Some(
                    LoopUsageSummaryRef::new(format!("usage:{usage_marker}")).unwrap(),
                ),
                diagnostic_ref: Some(
                    LoopDiagnosticRef::new(format!("diag:{diagnostic_marker}")).unwrap(),
                ),
                exit_id: LoopExitId::new(format!("exit:{exit_marker}")).unwrap(),
            }),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_verified_failure_evidence(),
        },
    )
    .await
    .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(SanitizedFailure::category),
        Some("driver_bug")
    );

    let projection = TurnEventProjectionService::new(reopen());
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::Failed,
        ]
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.scope == request.scope)
    );
    assert!(snapshot.entries.iter().all(|entry| entry.run_id == run_id));
    let failed_entry = snapshot.entries.last().unwrap();
    assert_eq!(failed_entry.status, TurnStatus::Failed);
    assert_eq!(failed_entry.sanitized_reason.as_deref(), Some("driver_bug"));

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(serialized.contains("driver_bug"));
    assert_no_forbidden_turn_event_content(
        "durable failed turn lifecycle projection",
        &serialized,
        &[
            accepted_marker,
            source_marker,
            reply_marker,
            diagnostic_marker,
            usage_marker,
            exit_marker,
            private_path,
        ],
    );
}

async fn assert_db_turn_event_projection_replays_cancelled_terminal_after_reopen_without_raw_refs<
    S,
    Reopen,
>(
    store: Arc<S>,
    reopen: Reopen,
    thread: &str,
    idempotency_key_prefix: &str,
    marker_prefix: &str,
) where
    S: TurnStateStore + TurnRunTransitionPort + TurnEventProjectionSource + 'static,
    Reopen: FnOnce() -> Arc<S>,
{
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let submit_key = format!("{idempotency_key_prefix}-submit");
    let cancel_key = format!("{idempotency_key_prefix}-cancel-running");
    let accepted_marker = format!("{marker_prefix}_ACCEPTED_SENTINEL_3022");
    let source_marker = format!("{marker_prefix}_SOURCE_SENTINEL_3022");
    let reply_marker = format!("{marker_prefix}_REPLY_SENTINEL_3022");
    let reason_marker = format!("{marker_prefix}_REASON_SENTINEL_3022");
    let exit_marker = format!("{marker_prefix}_EXIT_SENTINEL_3022");
    let private_path = format!("/tmp/{marker_prefix}-cancelled-private");
    let mut request = submit_request(thread, &submit_key);
    request.accepted_message_ref =
        AcceptedMessageRef::new(format!("message-{accepted_marker} {private_path}")).unwrap();
    request.source_binding_ref = SourceBindingRef::new(format!("source-{source_marker}")).unwrap();
    request.reply_target_binding_ref =
        ReplyTargetBindingRef::new(format!("reply-{reply_marker}")).unwrap();

    let run_id = accepted_run_id(&coordinator.submit_turn(request.clone()).await.unwrap());
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: Some(request.scope.clone()),
        })
        .await
        .unwrap()
        .unwrap();
    let cancel_requested = coordinator
        .cancel_run(CancelRunRequest {
            scope: request.scope.clone(),
            actor: actor(),
            run_id,
            reason: SanitizedCancelReason::OperatorRequested,
            idempotency_key: IdempotencyKey::new(cancel_key).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(cancel_requested.status, TurnStatus::CancelRequested);

    let cancelled = apply_loop_exit(
        store.as_ref(),
        ApplyLoopExitRequest {
            run_id,
            runner_id,
            lease_token,
            exit: LoopExit::Cancelled(LoopCancelled {
                reason_kind: LoopCancelledReasonKind::HostCancellation,
                checkpoint_id: None,
                interrupted_message_refs: vec![
                    LoopMessageRef::new(format!("msg:{reason_marker}")).unwrap(),
                ],
                exit_id: LoopExitId::new(format!("exit:{exit_marker}")).unwrap(),
            }),
            validation_policy: LoopExitValidationPolicy::recovery_required()
                .with_host_cancellation_observed(),
        },
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, TurnStatus::Cancelled);

    let projection = TurnEventProjectionService::new(reopen());
    let snapshot = projection
        .snapshot(TurnEventProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();

    assert_eq!(
        snapshot
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::CancelRequested,
            TurnEventKind::Cancelled,
        ]
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|entry| entry.scope == request.scope)
    );
    assert!(snapshot.entries.iter().all(|entry| entry.run_id == run_id));
    let cancel_requested_entry = snapshot
        .entries
        .iter()
        .find(|entry| entry.kind == TurnEventKind::CancelRequested)
        .unwrap();
    assert_eq!(
        cancel_requested_entry.sanitized_reason.as_deref(),
        Some("operator_requested")
    );
    let cancelled_entry = snapshot.entries.last().unwrap();
    assert_eq!(cancelled_entry.status, TurnStatus::Cancelled);
    assert_eq!(cancelled_entry.sanitized_reason.as_deref(), None);

    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(serialized.contains("operator_requested"));
    assert_no_forbidden_turn_event_content(
        "durable cancelled turn lifecycle projection",
        &serialized,
        &[
            accepted_marker,
            source_marker,
            reply_marker,
            reason_marker,
            exit_marker,
            private_path,
        ],
    );
}

fn assert_no_forbidden_turn_event_content<T: AsRef<str>>(
    label: &str,
    serialized: &str,
    forbidden: &[T],
) {
    for value in forbidden {
        let marker = value.as_ref();
        assert!(
            !serialized.contains(marker),
            "{label} leaked forbidden marker {marker}"
        );
    }
}

#[cfg(feature = "libsql")]
async fn libsql_db() -> (Arc<libsql::Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("turns.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    (db, dir)
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    let Ok(url) = std::env::var("IRONCLAW_TURNS_POSTGRES_URL") else {
        eprintln!("skipping postgres turn-state contract: IRONCLAW_TURNS_POSTGRES_URL not set");
        return None;
    };
    let config: tokio_postgres::Config = match url.parse() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skipping postgres turn-state contract: invalid url ({error})");
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .unwrap();
    if let Err(error) = pool.get().await {
        eprintln!("skipping postgres turn-state contract: database unavailable ({error})");
        return None;
    }
    Some(pool)
}

#[cfg(feature = "postgres")]
fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos()
}

fn submit_request(thread: &str, idempotency_key: &str) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: scope(thread),
        actor: actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!(
            "message-{thread}-{idempotency_key}"
        ))
        .unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
        received_at: received_at(),
    }
}

fn cancel_request(thread: &str, run_id: TurnRunId, idempotency_key: &str) -> CancelRunRequest {
    CancelRunRequest {
        scope: scope(thread),
        actor: actor(),
        run_id,
        reason: SanitizedCancelReason::UserRequested,
        idempotency_key: IdempotencyKey::new(idempotency_key).unwrap(),
    }
}

fn accepted_run_id(response: &SubmitTurnResponse) -> TurnRunId {
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    *run_id
}

fn completed_exit(exit_id: &str) -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![LoopMessageRef::new("msg:assistant-final").unwrap()],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: ironclaw_turns::LoopExitId::new(exit_id).unwrap(),
    })
}

fn received_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap()
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
