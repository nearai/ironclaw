#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::*;
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

#[tokio::test]
async fn in_memory_defaults_policy_progress_opt_in_and_subscription_scope() {
    let store = InMemoryOutboundStateStore::default();
    durable_policy_subscription_delivery_flow(&store).await;
    subscription_cursor_rejects_mismatched_scope(&store).await;
    subscription_ids_are_scoped_not_global(&store).await;
    subscription_cursor_rejects_backward_advancement(&store).await;
    delivery_status_rejects_inconsistent_failure_kind(&store).await;
    notification_policy_rejects_excessive_targets(&store).await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_persists_outbound_state_across_reopen() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let store = LibSqlOutboundStateStore::new(Arc::clone(&db));
    store.run_migrations().await.unwrap();
    durable_policy_subscription_delivery_flow(&store).await;

    let reopened = LibSqlOutboundStateStore::new(db);
    assert_reopened_state(&reopened).await;
    subscription_ids_are_scoped_not_global(&reopened).await;
    subscription_cursor_rejects_backward_advancement(&reopened).await;
    delivery_status_rejects_inconsistent_failure_kind(&reopened).await;
    notification_policy_rejects_excessive_targets(&reopened).await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_rejects_mismatched_subscription_scope_after_reopen() {
    let (db_path, _dir) = libsql_db_path();
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let store = LibSqlOutboundStateStore::new(Arc::clone(&db));
    store.run_migrations().await.unwrap();
    seed_subscription(&store).await;

    let reopened = LibSqlOutboundStateStore::new(db);
    subscription_cursor_rejects_mismatched_scope(&reopened).await;
    subscription_ids_are_scoped_not_global(&reopened).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_persists_outbound_state_across_reopen_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let store = PostgresOutboundStateStore::new(pool.clone());
    store.run_migrations().await.unwrap();
    durable_policy_subscription_delivery_flow(&store).await;

    let reopened = PostgresOutboundStateStore::new(pool);
    assert_reopened_state(&reopened).await;
    subscription_ids_are_scoped_not_global(&reopened).await;
    subscription_cursor_rejects_backward_advancement(&reopened).await;
    delivery_status_rejects_inconsistent_failure_kind(&reopened).await;
    notification_policy_rejects_excessive_targets(&reopened).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_rejects_mismatched_subscription_scope_after_reopen_when_configured() {
    let Some(pool) = postgres_pool().await else {
        return;
    };
    let store = PostgresOutboundStateStore::new(pool.clone());
    store.run_migrations().await.unwrap();
    seed_subscription(&store).await;

    let reopened = PostgresOutboundStateStore::new(pool);
    subscription_cursor_rejects_mismatched_scope(&reopened).await;
    subscription_ids_are_scoped_not_global(&reopened).await;
}

async fn durable_policy_subscription_delivery_flow(store: &impl OutboundStateStore) {
    let scope = turn_scope();
    let default_reply = reply_ref("reply-default");
    let extra_final = reply_ref("reply-extra-final");
    let progress_target = reply_ref("reply-progress");

    let default_final = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:final-1").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(targets(&default_final), vec![default_reply.clone()]);

    let default_progress = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: None,
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::Progress,
            projection_ref: ProjectionUpdateRef::new("projection:progress-1").unwrap(),
        })
        .await
        .unwrap();
    assert!(default_progress.candidates.is_empty());

    store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: scope.clone(),
            targets: vec![
                ThreadNotificationTarget {
                    target: extra_final.clone(),
                    final_replies: true,
                    progress: false,
                },
                ThreadNotificationTarget {
                    target: progress_target.clone(),
                    final_replies: false,
                    progress: true,
                },
                ThreadNotificationTarget {
                    target: default_reply.clone(),
                    final_replies: true,
                    progress: true,
                },
            ],
        })
        .await
        .unwrap();

    let final_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:final-2").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&final_plan),
        vec![default_reply.clone(), extra_final]
    );
    assert!(
        final_plan
            .candidates
            .iter()
            .all(|candidate| candidate.requires_reply_target_revalidation)
    );

    let progress_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: scope.clone(),
            turn_run_id: None,
            reply_target: default_reply.clone(),
            kind: OutboundPushKind::Progress,
            projection_ref: ProjectionUpdateRef::new("projection:progress-2").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&progress_plan),
        vec![progress_target, default_reply.clone()]
    );

    seed_subscription(store).await;
    let cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(42));
    store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: cursor.clone(),
        })
        .await
        .unwrap();
    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded, cursor);

    let delivery_id = OutboundDeliveryId::new();
    let initial_attempt = OutboundDeliveryAttempt {
        delivery_id,
        scope: scope.clone(),
        candidate: final_plan.candidates[0].clone(),
        status: OutboundDeliveryStatus::Pending,
        attempted_at: now(),
        failure_kind: None,
    };
    store
        .record_delivery_attempt(initial_attempt.clone())
        .await
        .unwrap();
    let wrong_scope_update = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: sibling_turn_scope(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await;
    assert!(matches!(
        wrong_scope_update,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));

    store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await
        .unwrap();

    store
        .record_delivery_attempt(initial_attempt)
        .await
        .unwrap();
    let after_duplicate_retry = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(after_duplicate_retry.len(), 1);
    assert_eq!(
        after_duplicate_retry[0].status,
        OutboundDeliveryStatus::Failed
    );
    assert_eq!(
        after_duplicate_retry[0].failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );

    let duplicate_different_candidate = store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id,
            scope: scope.clone(),
            candidate: progress_plan.candidates[0].clone(),
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await;
    assert!(matches!(
        duplicate_different_candidate,
        Err(OutboundError::Backend)
    ));

    let deliveries = store.list_delivery_attempts(scope.clone()).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        deliveries[0].failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );

    let policy_after_failure = store
        .load_thread_notification_policy(scope.clone())
        .await
        .unwrap();
    assert_eq!(policy_after_failure.targets.len(), 3);

    full_turn_scope_isolation(store, scope).await;
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn assert_reopened_state(store: &impl OutboundStateStore) {
    let final_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: turn_scope(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: reply_ref("reply-default"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:after-reopen").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&final_plan),
        vec![reply_ref("reply-default"), reply_ref("reply-extra-final")]
    );

    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.runtime, EventCursor::new(42));

    let deliveries = store.list_delivery_attempts(turn_scope()).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );
}

async fn seed_subscription(store: &impl OutboundStateStore) {
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::origin_for_scope(projection_scope())),
        })
        .await
        .unwrap();
}

async fn subscription_cursor_rejects_mismatched_scope(store: &impl OutboundStateStore) {
    let wrong_actor = TurnActor::new(UserId::new("user-other").unwrap());
    let result = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: wrong_actor,
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await;
    // Anti-enumeration: wrong actor/scope reads look identical to missing
    // subscription ids, so callers cannot distinguish an existing foreign row
    // from absence.
    assert!(matches!(result, Ok(None)));

    let mut wrong_scope = projection_scope();
    wrong_scope.read_scope.thread_id = Some(ThreadId::new("thread-other").unwrap());
    let result = store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: ProjectionCursor::for_scope(wrong_scope, EventCursor::new(7)),
        })
        .await;
    assert!(matches!(
        result,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));

    let rebind = store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id(),
            actor: TurnActor::new(UserId::new("user-other").unwrap()),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(99),
            )),
        })
        .await;
    assert!(matches!(
        rebind,
        Err(OutboundError::SubscriptionScopeMismatch)
    ));
}

async fn subscription_ids_are_scoped_not_global(store: &impl OutboundStateStore) {
    let shared_subscription_id =
        ProjectionSubscriptionId::new(format!("webui-scoped-subscription-{}", TurnRunId::new()))
            .unwrap();
    let base_cursor = ProjectionCursor::for_scope(projection_scope(), EventCursor::new(10));
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: shared_subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(base_cursor.clone()),
        })
        .await
        .unwrap();

    let sibling_actor = TurnActor::new(UserId::new("user-outbound-sibling").unwrap());
    let sibling_scope = projection_scope_for_user("user-outbound-sibling");
    let sibling_cursor = ProjectionCursor::for_scope(sibling_scope.clone(), EventCursor::new(3));
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: shared_subscription_id.clone(),
            actor: sibling_actor.clone(),
            scope: sibling_scope.clone(),
            thread_id: thread_id(),
            cursor: Some(sibling_cursor.clone()),
        })
        .await
        .unwrap();

    let base_loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(base_loaded, base_cursor);

    let sibling_loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: sibling_actor.clone(),
            scope: sibling_scope.clone(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sibling_loaded, sibling_cursor);

    let unrelated_actor = TurnActor::new(UserId::new("user-outbound-unrelated").unwrap());
    let unrelated_scope = projection_scope_for_user("user-outbound-unrelated");
    let unrelated_lookup = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: shared_subscription_id.clone(),
            actor: unrelated_actor,
            scope: unrelated_scope,
            thread_id: thread_id(),
        })
        .await;
    // Anti-enumeration: even when the id exists for sibling tuples, an
    // unrelated tuple receives the same `None` result as a missing id.
    assert!(matches!(unrelated_lookup, Ok(None)));
}

async fn subscription_cursor_rejects_backward_advancement(store: &impl OutboundStateStore) {
    let subscription_id =
        ProjectionSubscriptionId::new(format!("webui-subscription-backward-{}", TurnRunId::new()))
            .unwrap();
    store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(42),
            )),
        })
        .await
        .unwrap();

    let regression = store
        .advance_subscription_cursor(AdvanceSubscriptionCursorRequest {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            thread_id: thread_id(),
            cursor: ProjectionCursor::for_scope(projection_scope(), EventCursor::new(7)),
        })
        .await;
    assert!(matches!(
        regression,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let stale_upsert = store
        .upsert_subscription(ProjectionSubscriptionRecord {
            subscription_id: subscription_id.clone(),
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
            cursor: Some(ProjectionCursor::for_scope(
                projection_scope(),
                EventCursor::new(6),
            )),
        })
        .await;
    assert!(matches!(
        stale_upsert,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id,
            actor: actor(),
            scope: projection_scope(),
            thread_id: thread_id(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.runtime, EventCursor::new(42));
}

async fn delivery_status_rejects_inconsistent_failure_kind(store: &impl OutboundStateStore) {
    let scope = turn_scope();
    let delivery_id = OutboundDeliveryId::new();
    let attempt = OutboundDeliveryAttempt {
        delivery_id,
        scope: scope.clone(),
        candidate: OutboundPushCandidate {
            thread_id: scope.thread_id.clone(),
            turn_run_id: Some(TurnRunId::new()),
            target: reply_ref("reply-status-validation"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new(format!(
                "projection:status-validation:{}",
                TurnRunId::new()
            ))
            .unwrap(),
            requires_reply_target_revalidation: true,
        },
        status: OutboundDeliveryStatus::Pending,
        attempted_at: now(),
        failure_kind: None,
    };
    store.record_delivery_attempt(attempt).await.unwrap();

    let delivered_with_failure = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Delivered,
            updated_at: now(),
            failure_kind: Some(DeliveryFailureKind::AuthorizationRevoked),
        })
        .await;
    assert!(matches!(
        delivered_with_failure,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let failed_without_failure = store
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id,
            scope: scope.clone(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: now(),
            failure_kind: None,
        })
        .await;
    assert!(matches!(
        failed_without_failure,
        Err(OutboundError::InvalidRequest { .. })
    ));

    let deliveries = store.list_delivery_attempts(scope).await.unwrap();
    let stored = deliveries
        .iter()
        .find(|attempt| attempt.delivery_id == delivery_id)
        .unwrap();
    assert_eq!(stored.status, OutboundDeliveryStatus::Pending);
    assert_eq!(stored.failure_kind, None);
}

async fn notification_policy_rejects_excessive_targets(store: &impl OutboundStateStore) {
    let targets = (0..33)
        .map(|i| ThreadNotificationTarget {
            target: reply_ref(&format!("reply-too-many-{i}")),
            final_replies: true,
            progress: false,
        })
        .collect();
    let result = store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: turn_scope(),
            targets,
        })
        .await;
    assert!(matches!(result, Err(OutboundError::InvalidRequest { .. })));
}

async fn full_turn_scope_isolation(store: &impl OutboundStateStore, original_scope: TurnScope) {
    let sibling_scope = sibling_turn_scope();
    let sibling_target = reply_ref("reply-sibling");
    store
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: sibling_scope.clone(),
            targets: vec![ThreadNotificationTarget {
                target: sibling_target.clone(),
                final_replies: true,
                progress: true,
            }],
        })
        .await
        .unwrap();

    let original_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: original_scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: reply_ref("reply-default"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:isolated-original").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&original_plan),
        vec![reply_ref("reply-default"), reply_ref("reply-extra-final")]
    );

    let sibling_plan = store
        .plan_push_targets(OutboundPushTargetRequest {
            scope: sibling_scope.clone(),
            turn_run_id: Some(TurnRunId::new()),
            reply_target: reply_ref("reply-sibling-default"),
            kind: OutboundPushKind::FinalReply,
            projection_ref: ProjectionUpdateRef::new("projection:isolated-sibling").unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        targets(&sibling_plan),
        vec![reply_ref("reply-sibling-default"), sibling_target]
    );

    let sibling_delivery_id = OutboundDeliveryId::new();
    store
        .record_delivery_attempt(OutboundDeliveryAttempt {
            delivery_id: sibling_delivery_id,
            scope: sibling_scope.clone(),
            candidate: sibling_plan.candidates[0].clone(),
            status: OutboundDeliveryStatus::Pending,
            attempted_at: now(),
            failure_kind: None,
        })
        .await
        .unwrap();

    let original_deliveries = store.list_delivery_attempts(original_scope).await.unwrap();
    assert_eq!(original_deliveries.len(), 1);
    let sibling_deliveries = store.list_delivery_attempts(sibling_scope).await.unwrap();
    assert_eq!(sibling_deliveries.len(), 1);
    assert_eq!(sibling_deliveries[0].delivery_id, sibling_delivery_id);
}

fn targets(plan: &OutboundPushPlan) -> Vec<ReplyTargetBindingRef> {
    plan.candidates
        .iter()
        .map(|candidate| candidate.target.clone())
        .collect()
}

fn subscription_id() -> ProjectionSubscriptionId {
    ProjectionSubscriptionId::new("webui-subscription-1").unwrap()
}

fn turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-outbound").unwrap(),
        Some(AgentId::new("agent-outbound").unwrap()),
        Some(ProjectId::new("project-outbound").unwrap()),
        thread_id(),
    )
}

fn sibling_turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-outbound").unwrap(),
        Some(AgentId::new("agent-outbound-other").unwrap()),
        Some(ProjectId::new("project-outbound-other").unwrap()),
        thread_id(),
    )
}

fn projection_scope() -> ProjectionScope {
    projection_scope_for_user("user-outbound")
}

fn projection_scope_for_user(user_id: &str) -> ProjectionScope {
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new("tenant-outbound").unwrap(),
            UserId::new(user_id).unwrap(),
            Some(AgentId::new("agent-outbound").unwrap()),
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-outbound").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id()),
            process_id: None,
        },
    }
}

fn actor() -> TurnActor {
    TurnActor::new(UserId::new("user-outbound").unwrap())
}

fn thread_id() -> ThreadId {
    ThreadId::new("thread-outbound").unwrap()
}

fn reply_ref(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).unwrap()
}

fn now() -> ironclaw_host_api::Timestamp {
    chrono::Utc::now()
}

#[cfg(feature = "libsql")]
fn libsql_db_path() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbound.db").to_string_lossy().to_string();
    (path, dir)
}

#[cfg(feature = "postgres")]
async fn postgres_pool() -> Option<deadpool_postgres::Pool> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_OUTBOUND_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = url.parse::<tokio_postgres::Config>().unwrap();
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    Some(
        deadpool_postgres::Pool::builder(manager)
            .max_size(4)
            .build()
            .unwrap(),
    )
}
