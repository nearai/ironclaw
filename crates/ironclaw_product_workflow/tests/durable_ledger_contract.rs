#![cfg(any(feature = "libsql", feature = "postgres"))]

use std::num::NonZeroUsize;
use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Duration;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_product_workflow::RebornLibSqlIdempotencyLedger;
#[cfg(feature = "postgres")]
use ironclaw_product_workflow::RebornPostgresIdempotencyLedger;

// Shared ledger test support was renamed on fold-in to avoid colliding with the
// product_workflow crate's own `tests/support/` module.
#[path = "durable_ledger_support/mod.rs"]
mod support;

use support::*;

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
async fn libsql_settled_prune_interval_defers_until_interval() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_in_flight_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(&ledger, "libsql-prune-interval").await;
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
async fn postgres_settled_prune_interval_defers_until_interval_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_in_flight_lease(filesystem, Duration::seconds(10))
            .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
            .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(
        &ledger,
        &unique_suffix("postgres-prune-interval"),
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
