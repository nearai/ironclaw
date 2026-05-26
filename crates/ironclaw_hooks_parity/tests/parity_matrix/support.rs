//! Parity-matrix shared support: observation types, deterministic fixtures,
//! per-step drivers, the three backend factories, and the `assert_parity`
//! oracle runner. Split out of the monolithic `parity_matrix.rs` so the
//! scenarios (`super::scripts`) and the hand-computed oracles (`super::oracle`)
//! read as focused modules (#3937 follow-up).

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::predicate_state::{
    InMemoryPredicateStateBackend, InvocationKey, PredicateBackendError, PredicateEventId,
    PredicateStateBackend, ValueKey,
};
use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Observation log
// ---------------------------------------------------------------------------

/// The observable result of one scripted step, normalized so it can be
/// compared across backends regardless of internal representation. Error
/// values collapse to their *variant* (we compare the kind of failure, not the
/// exact message string, which legitimately differs between backends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StepOutcome {
    /// `record_invocation` returned this in-window count.
    Count(u32),
    /// `record_value` returned this in-window sum (string form for stable Eq).
    Sum(String),
    /// The per-key sliding window hit its sample cap (fail-closed).
    WindowOverflow,
    /// Any other backend error variant (should not occur in these scripts).
    OtherError(String),
}

/// One observation: the step label, its outcome, and the cumulative eviction
/// counter *after* the step. The label makes a divergence diff name the exact
/// scripted action that differed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub(crate) label: String,
    pub(crate) outcome: StepOutcome,
    pub(crate) evictions_after: u64,
}

/// The full per-backend log. Two backends are behaviorally identical iff their
/// logs are equal.
pub(crate) type ObservationLog = Vec<Observation>;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

pub(crate) fn hook_id() -> HookId {
    HookId::derive(
        &ExtensionId::new("ext").expect("ext id"),
        "1.0",
        &HookLocalId::new("h").expect("hook local id"),
        HookVersion::ONE,
    )
}

pub(crate) fn tenant(name: &str) -> TenantId {
    TenantId::new(name).expect("tenant id")
}

pub(crate) fn ev(s: &str) -> PredicateEventId {
    PredicateEventId::new(s).expect("event id")
}

pub(crate) fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

pub(crate) fn at_secs(secs: i64) -> DateTime<Utc> {
    base() + chrono::Duration::seconds(secs)
}

pub(crate) fn at_millis(ms: i64) -> DateTime<Utc> {
    base() + chrono::Duration::milliseconds(ms)
}

pub(crate) fn inv_key(tenant_name: &str, capability: &str) -> InvocationKey {
    InvocationKey {
        hook_id: hook_id(),
        tenant_id: tenant(tenant_name),
        capability: capability.to_string(),
    }
}

pub(crate) fn val_key(tenant_name: &str, capability: &str, field: &str) -> ValueKey {
    ValueKey {
        hook_id: hook_id(),
        tenant_id: tenant(tenant_name),
        capability: capability.to_string(),
        field: field.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The scripted sequences
// ---------------------------------------------------------------------------

/// Helper: drive one invocation step and record the normalized outcome.
pub(crate) async fn step_invocation(
    backend: &dyn PredicateStateBackend,
    log: &mut ObservationLog,
    label: &str,
    key: &InvocationKey,
    event_id: &PredicateEventId,
    now: DateTime<Utc>,
    window: Duration,
) {
    let outcome = match backend.record_invocation(key, event_id, now, window).await {
        Ok(c) => StepOutcome::Count(c),
        Err(PredicateBackendError::WindowOverflow { .. }) => StepOutcome::WindowOverflow,
        Err(other) => StepOutcome::OtherError(format!("{other:?}")),
    };
    log.push(Observation {
        label: label.to_string(),
        outcome,
        evictions_after: backend.evictions_observed(),
    });
}

/// Helper: drive one value step and record the normalized outcome.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn step_value(
    backend: &dyn PredicateStateBackend,
    log: &mut ObservationLog,
    label: &str,
    key: &ValueKey,
    event_id: &PredicateEventId,
    now: DateTime<Utc>,
    value: Decimal,
    window: Duration,
) {
    let outcome = match backend
        .record_value(key, event_id, now, value, window)
        .await
    {
        Ok(s) => StepOutcome::Sum(s.normalize().to_string()),
        Err(PredicateBackendError::WindowOverflow { .. }) => StepOutcome::WindowOverflow,
        Err(other) => StepOutcome::OtherError(format!("{other:?}")),
    };
    log.push(Observation {
        label: label.to_string(),
        outcome,
        evictions_after: backend.evictions_observed(),
    });
}
// ---------------------------------------------------------------------------
// Backend factories
// ---------------------------------------------------------------------------

/// Build a fresh in-memory backend.
pub(crate) fn in_memory() -> Arc<dyn PredicateStateBackend> {
    Arc::new(InMemoryPredicateStateBackend::new())
}

/// A libSQL backend bound to a private temp-file db that **owns** its
/// `TempDir`, so the db file is reclaimed when the fixture drops — no
/// `Box::leak`. The caller holds the fixture across the script run; on drop the
/// `backend` field (and its db handle) drops *before* `_dir` (struct fields
/// drop in declaration order), so the file is closed before the directory is
/// removed.
pub(crate) struct LibSqlFixture {
    backend: Arc<dyn PredicateStateBackend>,
    _dir: tempfile::TempDir,
}

/// Build a fresh, migrated libSQL backend over a private temp-file db, wrapped
/// in a [`LibSqlFixture`] that owns the `TempDir` for RAII cleanup.
pub(crate) async fn libsql_backend() -> LibSqlFixture {
    use ironclaw_hooks_libsql::LibSqlPredicateStateBackend;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("parity.db");
    let db = Arc::new(
        libsql::Builder::new_local(path.to_string_lossy().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let backend = LibSqlPredicateStateBackend::new(db);
    backend.run_migrations().await.expect("migrate");
    LibSqlFixture {
        backend: Arc::new(backend),
        _dir: dir,
    }
}

/// Build the Postgres parity leg, distinguishing *missing env* from *setup
/// failure after env discovery*:
///
/// - `Ok(None)` — no DB URL configured. The leg is legitimately absent; the
///   caller may skip it (subject to [`require_postgres_or_skip`]).
/// - `Err(_)` — a DB URL *was* found but connect / schema-create / pool-build /
///   migration / truncate failed. This is a real setup failure and must never
///   collapse into a green skip-pass; the caller turns it into a hard panic.
///
/// This is the shape the review asked for: missing env skips, but any error
/// after env discovery fails loudly so a misconfigured-but-reachable CI DB
/// cannot silently drop the Postgres parity leg. Each call uses a unique schema
/// so concurrent matrix runs cannot collide.
#[cfg(feature = "postgres")]
pub(crate) async fn postgres_backend() -> Result<Option<Arc<dyn PredicateStateBackend>>, String> {
    use ironclaw_hooks_postgres::PostgresPredicateStateBackend;

    // Missing env => skip-eligible. Everything below this point is a real
    // setup step whose failure is fatal (mapped to `Err`, never silently
    // swallowed into a skip).
    let Ok(url) =
        std::env::var("IRONCLAW_HOOKS_POSTGRES_URL").or_else(|_| std::env::var("DATABASE_URL"))
    else {
        return Ok(None);
    };

    // Unique schema per process so the parity binary cannot collide with the
    // per-backend test binaries `cargo test` runs in parallel.
    let schema = format!(
        "hooks_parity_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        tokio::spawn(conn);
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .await
            .map_err(|e| format!("create schema: {e}"))?;
    }

    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|e| format!("parse config: {e}"))?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let schema_for_hook = schema.clone();
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(8)
        .post_create(deadpool_postgres::Hook::async_fn(move |client, _| {
            let schema = schema_for_hook.clone();
            Box::pin(async move {
                client
                    .batch_execute(&format!("SET search_path TO {schema}"))
                    .await
                    .map_err(|e| deadpool_postgres::HookError::message(e.to_string()))?;
                Ok(())
            })
        }))
        .build()
        .map_err(|e| format!("build pool: {e}"))?;
    let backend = PostgresPredicateStateBackend::new(pool.clone());
    backend
        .run_migrations()
        .await
        .map_err(|e| format!("run migrations: {e}"))?;
    let client = pool.get().await.map_err(|e| format!("pool get: {e}"))?;
    client
        .batch_execute("TRUNCATE TABLE hooks_predicate_invocations, hooks_predicate_values")
        .await
        .map_err(|e| format!("truncate: {e}"))?;
    Ok(Some(Arc::new(backend)))
}

// ---------------------------------------------------------------------------
// The matrix
// ---------------------------------------------------------------------------

/// Process-global async mutex serializing the libSQL legs across the parallel
/// `#[tokio::test]` parity functions. See the call site for why concurrent
/// independent libSQL `Database` handles must not run heavy fills at once.
pub(crate) fn libsql_serial_guard() -> &'static tokio::sync::Mutex<()> {
    static GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// When `IRONCLAW_REQUIRE_POSTGRES=1`, a missing/unreachable Postgres backend is
/// a HARD failure rather than a silent skip. CI sets this so a misconfigured DB
/// (or a forgotten `--features postgres`) cannot turn the Postgres parity leg
/// into a green skip-pass; local runs without the env var still skip cleanly.
pub(crate) fn require_postgres_or_skip(script_name: &str) {
    let required = std::env::var("IRONCLAW_REQUIRE_POSTGRES")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if required {
        panic!(
            "[{script_name}] IRONCLAW_REQUIRE_POSTGRES=1 but the Postgres parity leg \
             did not run (backend not compiled with --features postgres, or \
             IRONCLAW_HOOKS_POSTGRES_URL / DATABASE_URL unset/unreachable). \
             Refusing to skip-pass under the CI hard-gate."
        );
    }
    eprintln!(
        "[{script_name}] Postgres leg SKIPPED (no --features postgres or no reachable \
         DB URL). Parity proven for in-memory + libSQL only; set \
         IRONCLAW_REQUIRE_POSTGRES=1 to make this a hard failure in CI."
    );
}

/// Helpers to build expected observations concisely.
pub(crate) fn obs_count(label: &str, count: u32, evictions_after: u64) -> Observation {
    Observation {
        label: label.to_string(),
        outcome: StepOutcome::Count(count),
        evictions_after,
    }
}

pub(crate) fn obs_sum(label: &str, sum: &str, evictions_after: u64) -> Observation {
    Observation {
        label: label.to_string(),
        outcome: StepOutcome::Sum(sum.to_string()),
        evictions_after,
    }
}

pub(crate) fn obs_overflow(label: &str, evictions_after: u64) -> Observation {
    Observation {
        label: label.to_string(),
        outcome: StepOutcome::WindowOverflow,
        evictions_after,
    }
}

/// Run `script` against in-memory, libSQL, and (if available) Postgres, and
/// cross-assert all produced logs are identical AND equal to an independently
/// hand-computed `expected` log.
///
/// The `expected` log is the *oracle*: it is the per-step count/sum/error
/// sequence worked out by hand from the predicate-state semantics (see the
/// per-script `expected_*` builders), NOT captured from any backend. Asserting
/// every backend matches `expected` — rather than just matching each other —
/// means two backends that happen to share the same semantic bug can no longer
/// both pass: they would both diverge from the independent oracle. The
/// in-memory backend is the reference for the *cross-backend* equality check,
/// but it is no longer the sole source of truth for *correctness*.
///
/// Returns the names of the legs that actually executed so the caller can print
/// which legs ran.
pub(crate) async fn assert_parity<F, Fut>(
    script_name: &str,
    expected: ObservationLog,
    script: F,
) -> Vec<&'static str>
where
    F: Fn(Arc<dyn PredicateStateBackend>) -> Fut,
    Fut: std::future::Future<Output = ObservationLog>,
{
    let mut ran = Vec::new();

    // Reference leg: in-memory (always runs).
    let reference = script(in_memory()).await;
    assert_eq!(
        reference, expected,
        "[{script_name}] in-memory backend diverged from the independent \
         hand-computed oracle — either the backend regressed or the oracle is \
         stale; do NOT silently update the oracle to match the backend"
    );
    ran.push("in-memory");

    // libSQL leg (always runs — embedded temp-file db).
    //
    // Serialize across the parallel `#[tokio::test]` functions: each leg builds
    // its OWN independent libSQL `Database` handle and several scripts do heavy
    // fills (per-key cap = 4096 connect/BEGIN IMMEDIATE/COMMIT cycles). Running
    // multiple independent `Database` handles' heavy fills concurrently in one
    // process intermittently trips `SQLITE_MISUSE` ("bad parameter or other API
    // misuse") in the replication-enabled libSQL build — a driver-level limit of
    // concurrent independent handles, NOT a backend bug (the per-backend libSQL
    // contract suite documents the same and runs serially via `harness=false`).
    // Holding this guard across the whole libSQL leg gives the same one-heavy-
    // -fill-at-a-time discipline without rewriting the parity binary's harness.
    let libsql_log = {
        let _guard = libsql_serial_guard().lock().await;
        // Hold the fixture (which owns the TempDir) across the whole script run,
        // then let it drop — backend first, then the temp dir — so the db file
        // is cleaned up rather than leaked.
        let fixture = libsql_backend().await;
        script(Arc::clone(&fixture.backend)).await
    };
    assert_eq!(
        libsql_log, expected,
        "[{script_name}] libSQL diverged from the oracle — \
         a real cross-backend behavioral bug, do NOT loosen this assertion"
    );
    ran.push("libsql");

    // Postgres leg (compiled under `postgres`, runs only with a DB URL).
    #[cfg(feature = "postgres")]
    {
        // `Err` = DB URL was set but setup (connect/schema/pool/migrate/
        // truncate) failed: ALWAYS fatal, never a skip — a misconfigured but
        // reachable CI DB must not silently drop the Postgres parity leg.
        // `Ok(None)` = no DB URL: skip-eligible (subject to the hard-gate).
        match postgres_backend().await {
            Ok(Some(pg)) => {
                let pg_log = script(pg).await;
                assert_eq!(
                    pg_log, expected,
                    "[{script_name}] Postgres diverged from the oracle — \
                     a real cross-backend behavioral bug, do NOT loosen this assertion"
                );
                ran.push("postgres");
            }
            Ok(None) => require_postgres_or_skip(script_name),
            Err(e) => panic!(
                "[{script_name}] Postgres parity leg setup FAILED after the DB URL was \
                 found: {e}. A configured-but-unreachable/misconfigured DB must fail \
                 loudly, not skip-pass."
            ),
        }
    }
    #[cfg(not(feature = "postgres"))]
    {
        require_postgres_or_skip(script_name);
    }

    eprintln!("[{script_name}] parity legs executed: {ran:?}");
    ran
}
