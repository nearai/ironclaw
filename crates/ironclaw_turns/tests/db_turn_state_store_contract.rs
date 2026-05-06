#![cfg(any(feature = "libsql", feature = "postgres"))]
#![cfg_attr(
    all(feature = "postgres", not(feature = "libsql")),
    allow(dead_code, unused_imports)
)]

use std::sync::Arc;

use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, DefaultTurnCoordinator, IdempotencyKey,
    ReplyTargetBindingRef, RunProfileRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor, TurnCoordinator, TurnError,
    TurnRunId, TurnScope, TurnStatus,
    runner::{ClaimRunRequest, RecoverExpiredLeasesRequest, TurnRunTransitionPort},
};

#[cfg(feature = "libsql")]
use ironclaw_turns::LibSqlTurnStateStore;
#[cfg(feature = "postgres")]
use ironclaw_turns::PostgresTurnStateStore;

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
#[test]
fn postgres_turn_state_store_implements_turn_contract_traits() {
    fn assert_state_store<T: ironclaw_turns::TurnStateStore>() {}
    fn assert_runner_port<T: TurnRunTransitionPort>() {}
    assert_state_store::<PostgresTurnStateStore>();
    assert_runner_port::<PostgresTurnStateStore>();
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
