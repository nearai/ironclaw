#![cfg(any(feature = "libsql", feature = "postgres"))]

#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::num::NonZeroUsize;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, ScopedPath, TenantId, UserId, VirtualPath,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, ProductInboundAction,
    ProductWorkflowError, SourceBindingKey,
};
use ironclaw_product_workflow_storage::RebornFilesystemIdempotencyLedger;
#[cfg(feature = "libsql")]
use ironclaw_product_workflow_storage::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
use ironclaw_product_workflow_storage::RebornPostgresIdempotencyLedger;

fn fingerprint(suffix: &str) -> ActionFingerprintKey {
    fingerprint_for_actor(suffix, "user1")
}

fn fingerprint_for_actor(suffix: &str, actor_id: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid adapter"),
        AdapterInstallationId::new("install_alpha").expect("valid installation"),
        ExternalActorRef::new("test", actor_id, Option::<String>::None).expect("valid actor"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new(format!("evt:{suffix}")).expect("valid event"),
    )
}

fn custom_root(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/product_workflow/idempotency/test_roots/{suffix}"
    ))
    .expect("valid custom ledger root")
}

fn scoped_custom_root(suffix: &str) -> ScopedPath {
    ScopedPath::new(format!(
        "/engine/product_workflow/idempotency/test_roots/{suffix}"
    ))
    .expect("valid scoped custom ledger root")
}

fn scoped_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/engine").expect("engine alias"),
        VirtualPath::new("/engine/scoped-workflow-storage").expect("engine target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn resource_scope(user_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant:workflow-storage").expect("tenant"),
        user_id: UserId::new(user_id).expect("user"),
        agent_id: Some(AgentId::new("agent:workflow-storage").expect("agent")),
        project_id: Some(ProjectId::new("project:workflow-storage").expect("project")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[cfg(feature = "postgres")]
fn unique_suffix(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{name}-{nanos}")
}

#[cfg(feature = "libsql")]
async fn libsql_filesystem(path: &str) -> Arc<LibSqlRootFilesystem> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem
        .run_migrations()
        .await
        .expect("run libsql filesystem migrations");
    filesystem
}

async fn assert_settled_action_survives_reopen_and_replays(
    ledger: &dyn IdempotencyLedger,
    reopened: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(mut action) = decision else {
        panic!("expected new action");
    };
    action.settle(ProductInboundAck::NoOp);
    ledger.settle(action).await.expect("settle");

    let replay = reopened
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("replay");

    let IdempotencyDecision::Replay(action) = replay else {
        panic!("expected replay");
    };
    assert_eq!(action.outcome, Some(ProductInboundAck::NoOp));
}

async fn assert_in_flight_action_blocks_until_lease_expires(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("begin"),
        IdempotencyDecision::New(_)
    ));
    let blocked = ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(5))
        .await
        .expect_err("fresh reservation should block");
    assert!(matches!(blocked, ProductWorkflowError::Transient { .. }));

    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

async fn assert_release_allows_retry_without_waiting_for_lease(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let decision = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin");
    let IdempotencyDecision::New(action) = decision else {
        panic!("expected new action");
    };
    ledger.release(action).await.expect("release");

    let retry = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("retry after release");
    assert!(matches!(retry, IdempotencyDecision::New(_)));
}

async fn assert_duplicate_reservation_contention_serializes(
    first: &dyn IdempotencyLedger,
    second: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let (left, right) = tokio::join!(
        first.begin_or_replay(fingerprint.clone(), received_at),
        second.begin_or_replay(fingerprint, received_at),
    );
    let results = [left, right];
    let new_count = results
        .iter()
        .filter(|result| matches!(result, Ok(IdempotencyDecision::New(_))))
        .count();
    let blocked_count = results
        .iter()
        .filter(|result| matches!(result, Err(ProductWorkflowError::Transient { .. })))
        .count();

    assert_eq!(new_count, 1);
    assert_eq!(blocked_count, 1);
}

async fn assert_superseded_reservation_cannot_settle(ledger: &dyn IdempotencyLedger, suffix: &str) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);

    let IdempotencyDecision::New(mut stale_action) = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin")
    else {
        panic!("expected new action");
    };

    let IdempotencyDecision::New(mut replacement) = ledger
        .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
        .await
        .expect("expired reservation should be reclaimed")
    else {
        panic!("expected reclaimed action");
    };

    stale_action.settle(ProductInboundAck::NoOp);
    let stale_error = ledger
        .settle(stale_action)
        .await
        .expect_err("superseded action must not settle");
    assert!(matches!(
        stale_error,
        ProductWorkflowError::Transient { .. }
    ));

    replacement.settle(ProductInboundAck::NoOp);
    ledger
        .settle(replacement)
        .await
        .expect("replacement settle");
}

async fn assert_settle_missing_reservation_returns_transient(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let mut action = ProductInboundAction::begin(fingerprint(suffix), received_at);
    action.settle(ProductInboundAck::NoOp);

    let error = ledger
        .settle(action)
        .await
        .expect_err("missing reservation must not settle");
    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

async fn assert_custom_root_isolated_from_default_root(
    custom: &dyn IdempotencyLedger,
    default: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let fingerprint = fingerprint(suffix);
    let IdempotencyDecision::New(mut action) = custom
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("begin in custom root")
    else {
        panic!("expected new custom-root action");
    };
    action.settle(ProductInboundAck::NoOp);
    custom.settle(action).await.expect("settle custom root");

    let default_decision = default
        .begin_or_replay(fingerprint, received_at + Duration::seconds(1))
        .await
        .expect("begin in default root");
    assert!(matches!(default_decision, IdempotencyDecision::New(_)));
}

async fn assert_actor_identity_is_part_of_fingerprint_path(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let first_actor = fingerprint_for_actor(suffix, "user1");
    let second_actor = fingerprint_for_actor(suffix, "user2");

    assert!(matches!(
        ledger
            .begin_or_replay(first_actor, received_at)
            .await
            .expect("begin first actor"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(second_actor, received_at)
            .await
            .expect("begin second actor"),
        IdempotencyDecision::New(_)
    ));
}

async fn assert_settled_entry_limit_prunes_oldest(ledger: &dyn IdempotencyLedger, suffix: &str) {
    let received_at = Utc::now();
    let oldest = fingerprint(&format!("{suffix}-oldest"));
    let newest = fingerprint(&format!("{suffix}-newest"));

    let IdempotencyDecision::New(mut oldest_action) = ledger
        .begin_or_replay(oldest.clone(), received_at)
        .await
        .expect("begin oldest")
    else {
        panic!("expected new oldest action");
    };
    oldest_action.settle(ProductInboundAck::NoOp);
    ledger.settle(oldest_action).await.expect("settle oldest");

    let IdempotencyDecision::New(mut newest_action) = ledger
        .begin_or_replay(newest.clone(), received_at + Duration::seconds(1))
        .await
        .expect("begin newest")
    else {
        panic!("expected new newest action");
    };
    newest_action.settle(ProductInboundAck::NoOp);
    ledger.settle(newest_action).await.expect("settle newest");

    assert!(matches!(
        ledger
            .begin_or_replay(oldest, received_at + Duration::seconds(2))
            .await
            .expect("oldest was pruned and can reserve again"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(newest, received_at + Duration::seconds(2))
            .await
            .expect("newest remains available for replay"),
        IdempotencyDecision::Replay(_)
    ));
}

async fn assert_settled_prune_interval_defers_until_interval(
    ledger: &dyn IdempotencyLedger,
    suffix: &str,
) {
    let received_at = Utc::now();
    let first = fingerprint(&format!("{suffix}-first"));
    let second = fingerprint(&format!("{suffix}-second"));
    let third = fingerprint(&format!("{suffix}-third"));

    for (fingerprint, received_at) in [
        (first.clone(), received_at),
        (second.clone(), received_at + Duration::seconds(1)),
    ] {
        let IdempotencyDecision::New(mut action) = ledger
            .begin_or_replay(fingerprint, received_at)
            .await
            .expect("begin settled action")
        else {
            panic!("expected new action before prune interval");
        };
        action.settle(ProductInboundAck::NoOp);
        ledger.settle(action).await.expect("settle action");
    }

    assert!(matches!(
        ledger
            .begin_or_replay(first.clone(), received_at + Duration::seconds(2))
            .await
            .expect("first action is retained before prune interval"),
        IdempotencyDecision::Replay(_)
    ));

    let IdempotencyDecision::New(mut third_action) = ledger
        .begin_or_replay(third.clone(), received_at + Duration::seconds(3))
        .await
        .expect("begin third action")
    else {
        panic!("expected third action");
    };
    third_action.settle(ProductInboundAck::NoOp);
    ledger
        .settle(third_action)
        .await
        .expect("settle third action");

    assert!(matches!(
        ledger
            .begin_or_replay(first, received_at + Duration::seconds(4))
            .await
            .expect("first action is pruned on interval"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(third, received_at + Duration::seconds(4))
            .await
            .expect("newest action remains after interval prune"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn scoped_filesystem_settled_action_replays() {
    let filesystem = scoped_filesystem();
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        resource_scope("user:scoped-replay"),
        Duration::seconds(10),
    );
    let reopened = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        resource_scope("user:scoped-replay"),
        Duration::seconds(10),
    );

    assert_settled_action_survives_reopen_and_replays(&ledger, &reopened, "scoped-replay").await;
}

#[tokio::test]
async fn scoped_filesystem_in_flight_action_blocks_until_lease_expires() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-lease"),
        Duration::seconds(10),
    );

    assert_in_flight_action_blocks_until_lease_expires(&ledger, "scoped-lease").await;
}

#[tokio::test]
async fn scoped_filesystem_duplicate_reservation_contention_serializes() {
    let filesystem = scoped_filesystem();
    let first = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        resource_scope("user:scoped-contention"),
        Duration::seconds(10),
    );
    let second = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        resource_scope("user:scoped-contention"),
        Duration::seconds(10),
    );

    assert_duplicate_reservation_contention_serializes(&first, &second, "scoped-contention").await;
}

#[tokio::test]
async fn scoped_filesystem_settled_entry_limit_prunes_oldest() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-retention"),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, "scoped-retention").await;
}

#[tokio::test]
async fn scoped_filesystem_settled_prune_interval_defers_until_interval() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-prune-interval"),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(&ledger, "scoped-prune-interval").await;
}

#[tokio::test]
async fn scoped_filesystem_custom_root_isolated_from_default_root() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-custom-root");
    let custom = RebornFilesystemIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        scope.clone(),
        scoped_custom_root("scoped"),
        Duration::seconds(60),
    );
    let default = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        scope,
        Duration::seconds(60),
    );

    assert_custom_root_isolated_from_default_root(&custom, &default, "scoped-custom-root").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_settled_action_survives_reopen_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);
    let reopened = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);

    assert_settled_action_survives_reopen_and_replays(&ledger, &reopened, "libsql-settled-replay")
        .await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_in_flight_action_blocks_until_lease_expires() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );
    assert_in_flight_action_blocks_until_lease_expires(&ledger, "libsql-lease").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_release_allows_retry_without_waiting_for_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(60),
    );
    assert_release_allows_retry_without_waiting_for_lease(&ledger, "libsql-release").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_duplicate_reservation_contention_serializes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let first = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );
    let second = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );

    assert_duplicate_reservation_contention_serializes(&first, &second, "libsql-contention").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_settled_entry_limit_prunes_oldest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, "libsql-retention").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_superseded_reservation_cannot_settle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );

    assert_superseded_reservation_cannot_settle(&ledger, "libsql-superseded").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_settle_missing_reservation_returns_transient() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger =
        RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path.display().to_string()).await);

    assert_settle_missing_reservation_returns_transient(&ledger, "libsql-missing-settle").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_custom_root_isolated_from_default_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let filesystem = libsql_filesystem(&db_path.display().to_string()).await;
    let custom = RebornLibSqlIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        custom_root("libsql"),
        Duration::seconds(60),
    );
    let default = RebornLibSqlIdempotencyLedger::new(filesystem);

    assert_custom_root_isolated_from_default_root(&custom, &default, "libsql-custom-root").await;
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_actor_identity_is_part_of_fingerprint_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new(libsql_filesystem(&db_path).await);

    assert_actor_identity_is_part_of_fingerprint_path(&ledger, "libsql-actor-isolation").await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_settled_action_survives_reopen_and_replays_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(Arc::clone(&filesystem));
    let reopened = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_settled_action_survives_reopen_and_replays(
        &ledger,
        &reopened,
        &unique_suffix("postgres-settled-replay"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_in_flight_action_blocks_until_lease_expires_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_in_flight_action_blocks_until_lease_expires(&ledger, &unique_suffix("postgres-lease"))
        .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_release_allows_retry_without_waiting_for_lease_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(60));

    assert_release_allows_retry_without_waiting_for_lease(
        &ledger,
        &unique_suffix("postgres-release"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_duplicate_reservation_contention_serializes_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let first = RebornPostgresIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        Duration::seconds(10),
    );
    let second =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_duplicate_reservation_contention_serializes(
        &first,
        &second,
        &unique_suffix("postgres-contention"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_settled_entry_limit_prunes_oldest_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10))
            .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, &unique_suffix("postgres-retention")).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_superseded_reservation_cannot_settle_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10));

    assert_superseded_reservation_cannot_settle(&ledger, &unique_suffix("postgres-superseded"))
        .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_settle_missing_reservation_returns_transient_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_settle_missing_reservation_returns_transient(
        &ledger,
        &unique_suffix("postgres-missing-settle"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_custom_root_isolated_from_default_root_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let custom = RebornPostgresIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        custom_root("postgres"),
        Duration::seconds(60),
    );
    let default = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_custom_root_isolated_from_default_root(
        &custom,
        &default,
        &unique_suffix("postgres-custom-root"),
    )
    .await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_actor_identity_is_part_of_fingerprint_path_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new(filesystem);

    assert_actor_identity_is_part_of_fingerprint_path(
        &ledger,
        &unique_suffix("postgres-actor-isolation"),
    )
    .await;
}

#[cfg(feature = "postgres")]
async fn postgres_filesystem() -> Option<Arc<PostgresRootFilesystem>> {
    let url = match std::env::var("IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping postgres product workflow ledger contract: IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL not set"
            );
            return None;
        }
    };
    let config = match url.parse::<tokio_postgres::Config>() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skipping postgres product workflow ledger contract: invalid url ({error})");
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool builds");
    if let Err(error) = pool.get().await {
        eprintln!(
            "skipping postgres product workflow ledger contract: database unavailable ({error})"
        );
        return None;
    }
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    if let Err(error) = filesystem.run_migrations().await {
        eprintln!(
            "skipping postgres product workflow ledger contract: filesystem migrations failed ({error})"
        );
        return None;
    }
    Some(filesystem)
}
