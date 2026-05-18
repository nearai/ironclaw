#![cfg(any(feature = "libsql", feature = "postgres"))]

#[cfg(feature = "libsql")]
use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalEventId, ProductAdapterId, ProductInboundAck,
};
use ironclaw_product_workflow::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, ProductWorkflowError,
    SourceBindingKey,
};
#[cfg(feature = "libsql")]
use ironclaw_product_workflow_storage::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
use ironclaw_product_workflow_storage::RebornPostgresIdempotencyLedger;

fn fingerprint(suffix: &str) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid adapter"),
        AdapterInstallationId::new("install_alpha").expect("valid installation"),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new(format!("evt:{suffix}")).expect("valid event"),
    )
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
async fn libsql_superseded_reservation_cannot_settle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );

    assert_superseded_reservation_cannot_settle(&ledger, "libsql-superseded").await;
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
