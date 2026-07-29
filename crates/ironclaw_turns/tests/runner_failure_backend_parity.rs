//! Real-backend parity for durable checkpointless runner-failure re-drive.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_filesystem::{
    LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, GetRunStateRequest, IdempotencyKey,
    InMemoryRunProfileResolver, ReplyTargetBindingRef, RunProfileRequest, SanitizedFailure,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnLeaseToken,
    TurnRunnerId, TurnScope, TurnStateRowStore, TurnStateStore, TurnStateStoreLimits, TurnStatus,
    runner::{
        ClaimRunRequest, RecordRunnerFailureRequest, RunnerFailureRecovery, TurnRunTransitionPort,
    },
};

async fn build_libsql_scoped() -> (
    tempfile::TempDir,
    Arc<ScopedFilesystem<LibSqlRootFilesystem>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("runner-failure-parity.db"))
            .build()
            .await
            .unwrap(),
    );
    let root = Arc::new(LibSqlRootFilesystem::new(db));
    root.run_migrations().await.unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").unwrap(),
        VirtualPath::new("/turns").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    (
        dir,
        Arc::new(ScopedFilesystem::with_fixed_view(root, mounts)),
    )
}

async fn build_postgres_scoped() -> Option<Arc<ScopedFilesystem<PostgresRootFilesystem>>> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .ok()?;
    let root = Arc::new(PostgresRootFilesystem::new(pool));
    root.run_migrations().await.ok()?;
    let unique_root = format!("/turn-redrive-test/{}", uuid::Uuid::new_v4().simple());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").ok()?,
        VirtualPath::new(unique_root).ok()?,
        MountPermissions::read_write_list_delete(),
    )])
    .ok()?;
    Some(Arc::new(ScopedFilesystem::with_fixed_view(root, mounts)))
}

fn scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("redrive-parity-tenant").unwrap(),
        Some(AgentId::new("redrive-parity-agent").unwrap()),
        Some(ProjectId::new("redrive-parity-project").unwrap()),
        ThreadId::new("redrive-parity-thread").unwrap(),
    )
}

fn submit_request() -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope: scope(),
        actor: TurnActor::new(UserId::new("redrive-parity-user").unwrap()),
        accepted_message_ref: AcceptedMessageRef::new("message-redrive-parity").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-redrive-parity").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-redrive-parity").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        requested_model: None,
        idempotency_key: IdempotencyKey::new("idem-redrive-parity").unwrap(),
        received_at: Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap(),
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

async fn assert_checkpointless_redrive_backend_parity<F>(scoped: Arc<ScopedFilesystem<F>>)
where
    F: RootFilesystem + 'static,
{
    let limits = TurnStateStoreLimits::default().set_max_crash_recovery_reclaims(2);
    let store = TurnStateRowStore::new(Arc::clone(&scoped)).with_limits(limits);
    let response = store
        .submit_turn(
            submit_request(),
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted {
        turn_id,
        run_id,
        accepted_message_ref,
        ..
    } = response;
    let first_runner_id = TurnRunnerId::new();
    let first_lease_token = TurnLeaseToken::new();
    store
        .claim_next_run(ClaimRunRequest {
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            scope_filter: Some(scope()),
        })
        .await
        .unwrap()
        .expect("first claim");
    let failure = SanitizedFailure::new("host_stage_unavailable_input")
        .unwrap()
        .with_detail("safe input drain detail");
    let queued = store
        .record_runner_failure(RecordRunnerFailureRequest {
            run_id,
            runner_id: first_runner_id,
            lease_token: first_lease_token,
            failure: failure.clone(),
            recovery: RunnerFailureRecovery::RedriveIfCheckpointless,
        })
        .await
        .unwrap();
    assert_eq!(queued.status, TurnStatus::Queued);
    store.drain().await.unwrap();
    drop(store);

    let reopened = TurnStateRowStore::new(Arc::clone(&scoped)).with_limits(limits);
    let reopened_state = reopened
        .get_run_state(GetRunStateRequest {
            scope: scope(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(reopened_state.status, TurnStatus::Queued);
    assert_eq!(reopened_state.turn_id, turn_id);
    assert_eq!(reopened_state.accepted_message_ref, accepted_message_ref);
    let second_runner_id = TurnRunnerId::new();
    let second_lease_token = TurnLeaseToken::new();
    let reclaimed = reopened
        .claim_next_run(ClaimRunRequest {
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            scope_filter: Some(scope()),
        })
        .await
        .unwrap()
        .expect("reclaim after reopen");
    assert_eq!(reclaimed.state.run_id, run_id);
    assert_eq!(reclaimed.state.turn_id, turn_id);
    assert_eq!(reclaimed.state.accepted_message_ref, accepted_message_ref);
    let failed = reopened
        .record_runner_failure(RecordRunnerFailureRequest {
            run_id,
            runner_id: second_runner_id,
            lease_token: second_lease_token,
            failure: failure.clone(),
            recovery: RunnerFailureRecovery::RedriveIfCheckpointless,
        })
        .await
        .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.failure, Some(failure.clone()));
    reopened.drain().await.unwrap();
    drop(reopened);

    let terminal = TurnStateRowStore::new(scoped).with_limits(limits);
    let terminal_state = terminal
        .get_run_state(GetRunStateRequest {
            scope: scope(),
            run_id,
        })
        .await
        .unwrap();
    assert_eq!(terminal_state.status, TurnStatus::Failed);
    assert_eq!(terminal_state.failure, Some(failure));
    assert!(
        terminal
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(scope()),
            })
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn checkpointless_runner_failure_redrive_has_libsql_parity() {
    let (_dir, scoped) = build_libsql_scoped().await;
    assert_checkpointless_redrive_backend_parity(scoped).await;
}

#[tokio::test]
async fn checkpointless_runner_failure_redrive_has_postgres_parity() {
    let Some(scoped) = build_postgres_scoped().await else {
        return;
    };
    assert_checkpointless_redrive_backend_parity(scoped).await;
}
