use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Duration;
use ironclaw_assistant::RebornFilesystemIdempotencyLedger;
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::PostgresRootFilesystem;

/// WS5 collapsed the per-backend ledger newtypes onto the generic fabric form.
/// These aliases keep the suite's two backend lanes named while proving both
/// resolve to the same generic type.
type RebornLibSqlIdempotencyLedger = RebornFilesystemIdempotencyLedger<LibSqlRootFilesystem>;
type RebornPostgresIdempotencyLedger = RebornFilesystemIdempotencyLedger<PostgresRootFilesystem>;

// Shared ledger test support was renamed on fold-in to avoid colliding with the
// product_surface crate's own `tests/support/` module.
#[path = "durable_ledger_support/mod.rs"]
mod support;

use support::*;
fn unique_suffix(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{name}-{nanos}")
}
async fn libsql_filesystem(path: &str) -> Arc<LibSqlRootFilesystem> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db).expect("filesystem runtime"));
    filesystem
        .run_migrations()
        .await
        .expect("run libsql filesystem migrations");
    filesystem
}
#[tokio::test]
async fn libsql_settled_action_survives_reopen_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new_root(libsql_filesystem(&db_path).await);
    let reopened = RebornLibSqlIdempotencyLedger::new_root(libsql_filesystem(&db_path).await);

    assert_settled_action_survives_reopen_and_replays(&ledger, &reopened, "libsql-settled-replay")
        .await;
}
#[tokio::test]
async fn libsql_in_flight_action_blocks_until_lease_expires() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );
    assert_in_flight_action_blocks_until_lease_expires(&ledger, "libsql-lease").await;
}
#[tokio::test]
async fn libsql_release_allows_retry_without_waiting_for_lease() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(60),
    );
    assert_release_allows_retry_without_waiting_for_lease(&ledger, "libsql-release").await;
}
#[tokio::test]
async fn libsql_duplicate_reservation_contention_serializes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let first = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );
    let second = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path).await,
        Duration::seconds(10),
    );

    assert_duplicate_reservation_contention_serializes(&first, &second, "libsql-contention").await;
}
#[tokio::test]
async fn libsql_settled_entry_limit_prunes_oldest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, "libsql-retention").await;
}
#[tokio::test]
async fn libsql_settled_prune_interval_defers_until_interval() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(&ledger, "libsql-prune-interval").await;
}
#[tokio::test]
async fn libsql_superseded_reservation_cannot_settle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::with_root_lease(
        libsql_filesystem(&db_path.display().to_string()).await,
        Duration::seconds(10),
    );

    assert_superseded_reservation_cannot_settle(&ledger, "libsql-superseded").await;
}
#[tokio::test]
async fn libsql_settle_missing_reservation_returns_transient() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let ledger = RebornLibSqlIdempotencyLedger::new_root(
        libsql_filesystem(&db_path.display().to_string()).await,
    );

    assert_settle_missing_reservation_returns_transient(&ledger, "libsql-missing-settle").await;
}
#[tokio::test]
async fn libsql_custom_root_isolated_from_default_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let filesystem = libsql_filesystem(&db_path.display().to_string()).await;
    let custom = RebornLibSqlIdempotencyLedger::with_virtual_root(
        Arc::clone(&filesystem),
        custom_root("libsql"),
        Duration::seconds(60),
    );
    let default = RebornLibSqlIdempotencyLedger::new_root(filesystem);

    assert_custom_root_isolated_from_default_root(&custom, &default, "libsql-custom-root").await;
}
#[tokio::test]
async fn libsql_actor_identity_is_part_of_fingerprint_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("workflow-ledger.db");
    let db_path = db_path.display().to_string();
    let ledger = RebornLibSqlIdempotencyLedger::new_root(libsql_filesystem(&db_path).await);

    assert_actor_identity_is_part_of_fingerprint_path(&ledger, "libsql-actor-isolation").await;
}
#[tokio::test]
async fn postgres_settled_action_survives_reopen_and_replays_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new_root(Arc::clone(&filesystem));
    let reopened = RebornPostgresIdempotencyLedger::new_root(filesystem);

    assert_settled_action_survives_reopen_and_replays(
        &ledger,
        &reopened,
        &unique_suffix("postgres-settled-replay"),
    )
    .await;
}
#[tokio::test]
async fn postgres_in_flight_action_blocks_until_lease_expires_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_root_lease(filesystem, Duration::seconds(10));

    assert_in_flight_action_blocks_until_lease_expires(&ledger, &unique_suffix("postgres-lease"))
        .await;
}
#[tokio::test]
async fn postgres_release_allows_retry_without_waiting_for_lease_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_root_lease(filesystem, Duration::seconds(60));

    assert_release_allows_retry_without_waiting_for_lease(
        &ledger,
        &unique_suffix("postgres-release"),
    )
    .await;
}
#[tokio::test]
async fn postgres_duplicate_reservation_contention_serializes_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let first = RebornPostgresIdempotencyLedger::with_root_lease(
        Arc::clone(&filesystem),
        Duration::seconds(10),
    );
    let second =
        RebornPostgresIdempotencyLedger::with_root_lease(filesystem, Duration::seconds(10));

    assert_duplicate_reservation_contention_serializes(
        &first,
        &second,
        &unique_suffix("postgres-contention"),
    )
    .await;
}
#[tokio::test]
async fn postgres_settled_entry_limit_prunes_oldest_when_configured() {
    let Some(db) = isolated_postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::with_root_lease(
        Arc::clone(&db.filesystem),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, &unique_suffix("postgres-retention")).await;
    drop(ledger);
    db.cleanup().await;
}
#[tokio::test]
async fn postgres_settled_prune_interval_defers_until_interval_when_configured() {
    let Some(db) = isolated_postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::with_root_lease(
        Arc::clone(&db.filesystem),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(
        &ledger,
        &unique_suffix("postgres-prune-interval"),
    )
    .await;
    drop(ledger);
    db.cleanup().await;
}
#[tokio::test]
async fn postgres_superseded_reservation_cannot_settle_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger =
        RebornPostgresIdempotencyLedger::with_root_lease(filesystem, Duration::seconds(10));

    assert_superseded_reservation_cannot_settle(&ledger, &unique_suffix("postgres-superseded"))
        .await;
}
#[tokio::test]
async fn postgres_settle_missing_reservation_returns_transient_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new_root(filesystem);

    assert_settle_missing_reservation_returns_transient(
        &ledger,
        &unique_suffix("postgres-missing-settle"),
    )
    .await;
}
#[tokio::test]
async fn postgres_custom_root_isolated_from_default_root_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let custom = RebornPostgresIdempotencyLedger::with_virtual_root(
        Arc::clone(&filesystem),
        custom_root("postgres"),
        Duration::seconds(60),
    );
    let default = RebornPostgresIdempotencyLedger::new_root(filesystem);

    assert_custom_root_isolated_from_default_root(
        &custom,
        &default,
        &unique_suffix("postgres-custom-root"),
    )
    .await;
}
#[tokio::test]
async fn postgres_actor_identity_is_part_of_fingerprint_path_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let ledger = RebornPostgresIdempotencyLedger::new_root(filesystem);

    assert_actor_identity_is_part_of_fingerprint_path(
        &ledger,
        &unique_suffix("postgres-actor-isolation"),
    )
    .await;
}
/// A private database for the settled-entry retention tests — the fabric
/// contract's `IsolatedDatabase` pattern
/// (`crates/substrates/ironclaw_filesystem/tests/db_root_filesystem_contract.rs`).
///
/// Unique fingerprint suffixes isolate every other test's rows, but the
/// settled-entry prune bookkeeping is global to the ledger root: it counts
/// and orders *all* settled entries under it, so sibling tests' entries
/// change which entry a limit of 1 prunes and when an interval of 3 fires —
/// and a limit-1 pruner running beside the other tests deletes *their*
/// settled rows in turn ("conflict row disappeared"). A name suffix cannot
/// isolate that; a private database can (the libsql twins get exactly that
/// from per-test temp files). The WS12 gauntlet report, §P8, measured the
/// defect; the two retention tests above are its regression pin.
struct IsolatedPostgresFilesystem {
    filesystem: Arc<PostgresRootFilesystem>,
    admin: tokio_postgres::Client,
    name: String,
}

impl IsolatedPostgresFilesystem {
    /// Drop the database on the way out of a passing test.
    ///
    /// A courtesy, not a guarantee: a failing assertion unwinds straight
    /// past it, so a red run can leave its database behind — that is what
    /// the sweep in `isolated_postgres_filesystem` collects on the next
    /// run. `FORCE` closes pool connections that have not gone away by the
    /// time the handles drop.
    async fn cleanup(self) {
        let Self {
            filesystem,
            admin,
            name,
        } = self;
        drop(filesystem);
        let _ = admin
            .execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"), &[])
            .await;
    }
}

/// One stale-database sweep per test binary, ahead of every creation.
/// Sweeping per provisioning call (as the fabric contract does) can race a
/// sibling test of the same binary between its `CREATE DATABASE` and first
/// connection; a single up-front sweep removes that window while still
/// collecting what failed runs left behind.
static STALE_SWEEP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// Distinguishes the databases of concurrently provisioning tests.
static NEXT_DATABASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Only sweep databases at least this old. A prefix-only sweep could delete
/// a concurrent *process*'s freshly created database in the window between
/// its `CREATE DATABASE` and its first connection (zero backends — nothing
/// for a no-`FORCE` drop to refuse); the epoch embedded in every name keeps
/// anything younger than this out of the sweep's SELECT entirely, so that
/// window is unreachable. Leftovers from crashed runs collect on the first
/// run after the cutoff.
const STALE_SWEEP_MIN_AGE_SECS: u64 = 3600;

fn unix_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the UNIX epoch")
        .as_secs()
}

/// Parses the creation epoch out of `<prefix><epoch>_<pid>_<counter>`.
/// `None` marks a pre-timestamp legacy name: definitionally past the cutoff,
/// collected immediately.
fn isolated_database_epoch(name: &str, prefix: &str) -> Option<u64> {
    name.strip_prefix(prefix)?
        .split('_')
        .next()?
        .parse::<u64>()
        .ok()
}

async fn isolated_postgres_filesystem() -> Option<IsolatedPostgresFilesystem> {
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
    // Reachability keeps `postgres_filesystem`'s skip semantics. Past a
    // reachable server, provisioning failures panic instead: this leg has no
    // CI executor, and a silent skip would let the retention tests pass
    // while testing nothing.
    let (admin, connection) = match config.connect(tokio_postgres::NoTls).await {
        Ok(connected) => connected,
        Err(error) => {
            eprintln!(
                "skipping postgres product workflow ledger contract: database unavailable ({error})"
            );
            return None;
        }
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    STALE_SWEEP
        .get_or_init(|| async {
            // Collect databases previously failed runs unwound past. Age-
            // gated by the epoch in each name so a concurrent run's fresh
            // database is never selected — even inside its zero-backend
            // window between `CREATE DATABASE` and first connection, which
            // a backend-count check (or the no-`FORCE` drop below) cannot
            // protect. No `FORCE`: anything old but still held open by a
            // live run additionally refuses to drop.
            if let Ok(stale) = admin
                .query(
                    "SELECT datname FROM pg_database WHERE datname LIKE 'pwledger_isolated_%'",
                    &[],
                )
                .await
            {
                let now = unix_epoch_secs();
                for row in stale {
                    let name = row.get::<_, String>(0);
                    let stale_enough = isolated_database_epoch(&name, "pwledger_isolated_")
                        .is_none_or(|epoch| now.saturating_sub(epoch) > STALE_SWEEP_MIN_AGE_SECS);
                    if !stale_enough {
                        continue;
                    }
                    let _ = admin
                        .execute(&format!("DROP DATABASE IF EXISTS {name}"), &[])
                        .await;
                }
            }
        })
        .await;
    // Identifiers cannot be bind parameters in DDL. The interpolations are
    // process-generated (epoch + pid + counter) or come from `pg_database`,
    // never caller input.
    let name = format!(
        "pwledger_isolated_{}_{}_{}",
        unix_epoch_secs(),
        std::process::id(),
        NEXT_DATABASE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    admin
        .execute(&format!("CREATE DATABASE {name}"), &[])
        .await
        .expect("create the isolated database (the role needs CREATEDB)");
    let mut isolated = config.clone();
    isolated.dbname(&name);
    let manager = deadpool_postgres::Manager::new(isolated, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool builds against the isolated database");
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    filesystem
        .run_migrations()
        .await
        .expect("migrate the isolated database");
    Some(IsolatedPostgresFilesystem {
        filesystem,
        admin,
        name,
    })
}

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
