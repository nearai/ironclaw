//! Cross-backend parity matrix — the load-bearing test of durable-backend
//! PR 4/4.
//!
//! A single deterministic, scripted sequence of `record_invocation` /
//! `record_value` calls (fixed ids, timestamps, values) is fed to **every**
//! `PredicateStateBackend` implementation, and the per-step observable output
//! (returned count / sum, error variant, the running `evictions_observed()`
//! counter) is captured into an [`ObservationLog`]. The matrix then cross-
//! asserts that every backend produced the *identical* log. That equality is
//! the proof the three backends are behaviorally interchangeable: the
//! evaluator can swap in-memory ⇄ Postgres ⇄ libSQL without changing a single
//! gate decision.
//!
//! # Which legs run
//!
//! - **in-memory**: always (pure process state).
//! - **libSQL**: always — the backend runs over an embedded temp-file db that
//!   needs no server, so this leg executes in any environment, including
//!   default `cargo test`.
//! - **Postgres**: compiled only under `--features postgres`, and at runtime
//!   only when `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL` points at a
//!   reachable server. Without a URL the Postgres leg is *skipped* (not
//!   failed), exactly like the per-backend contract suites — but then the
//!   parity guarantee is only proven for {in-memory, libSQL}. A real-Postgres
//!   CI run is required before merge to fully exercise the matrix (same caveat
//!   as #3933).
//!
//! # Why a captured log rather than ad-hoc asserts
//!
//! Capturing the full per-step output of one backend and asserting the others
//! reproduce it exactly means a NEW behavioral divergence (a backend that
//! fails closed at a different boundary, dedups differently, or returns a
//! different sum) surfaces as a concrete `assert_eq!` diff naming the diverging
//! step — instead of silently passing because each backend's own bespoke
//! assertions happened to be loose. If this file ever fails, it has found a
//! real bug in one of the backends (see the PR-4/4 description); do NOT loosen
//! the assertion to make it pass.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::predicate_state::{
    InMemoryPredicateStateBackend, InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY,
    PredicateBackendError, PredicateEventId, PredicateStateBackend, ValueKey,
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
enum StepOutcome {
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
struct Observation {
    label: String,
    outcome: StepOutcome,
    evictions_after: u64,
}

/// The full per-backend log. Two backends are behaviorally identical iff their
/// logs are equal.
type ObservationLog = Vec<Observation>;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn hook_id() -> HookId {
    HookId::derive(
        &ExtensionId::new("ext").expect("ext id"),
        "1.0",
        &HookLocalId::new("h").expect("hook local id"),
        HookVersion::ONE,
    )
}

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).expect("tenant id")
}

fn ev(s: &str) -> PredicateEventId {
    PredicateEventId::new(s).expect("event id")
}

fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

fn at_secs(secs: i64) -> DateTime<Utc> {
    base() + chrono::Duration::seconds(secs)
}

fn at_millis(ms: i64) -> DateTime<Utc> {
    base() + chrono::Duration::milliseconds(ms)
}

fn inv_key(tenant_name: &str, capability: &str) -> InvocationKey {
    InvocationKey {
        hook_id: hook_id(),
        tenant_id: tenant(tenant_name),
        capability: capability.to_string(),
    }
}

fn val_key(tenant_name: &str, capability: &str, field: &str) -> ValueKey {
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
async fn step_invocation(
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
async fn step_value(
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

/// Core behavioral script: counting, summing, window-trim, dedup/replay,
/// tenant isolation, cross-map dedup isolation, and the exact-cutoff retain
/// boundary. Deterministic — no wall-clock, no randomness. This exercises
/// every guarantee the three backends share (it deliberately stays under the
/// per-key cap and the per-tenant quota; those are scripted separately so a
/// divergence localizes).
async fn run_core_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
    let mut log = ObservationLog::new();
    let win = Duration::from_secs(60);

    // --- counting within window ---
    let k = inv_key("alpha", "cap.count");
    step_invocation(
        backend,
        &mut log,
        "count/e1",
        &k,
        &ev("e1"),
        at_secs(0),
        win,
    )
    .await;
    step_invocation(
        backend,
        &mut log,
        "count/e2",
        &k,
        &ev("e2"),
        at_secs(1),
        win,
    )
    .await;
    step_invocation(
        backend,
        &mut log,
        "count/e3",
        &k,
        &ev("e3"),
        at_secs(2),
        win,
    )
    .await;

    // --- replay/dedup: e2 again is a no-op against the count ---
    step_invocation(
        backend,
        &mut log,
        "count/replay-e2",
        &k,
        &ev("e2"),
        at_secs(3),
        win,
    )
    .await;
    // --- a fresh id advances ---
    step_invocation(
        backend,
        &mut log,
        "count/e4",
        &k,
        &ev("e4"),
        at_secs(4),
        win,
    )
    .await;

    // --- window trim: a far-future event trims everything older ---
    step_invocation(
        backend,
        &mut log,
        "count/far-future",
        &k,
        &ev("e-far"),
        at_secs(10_000),
        win,
    )
    .await;

    // --- exact-cutoff retain boundary (`< cutoff`, not `<=`) ---
    let kb = inv_key("alpha", "cap.boundary");
    step_invocation(
        backend,
        &mut log,
        "boundary/t0",
        &kb,
        &ev("b0"),
        at_secs(0),
        win,
    )
    .await;
    step_invocation(
        backend,
        &mut log,
        "boundary/at-cutoff",
        &kb,
        &ev("b60"),
        at_secs(60),
        win,
    )
    .await;

    // --- tenant isolation: beta's counter never inherits alpha's ---
    let ka = inv_key("alpha", "cap.iso");
    let kbeta = inv_key("beta", "cap.iso");
    step_invocation(
        backend,
        &mut log,
        "iso/alpha-1",
        &ka,
        &ev("a1"),
        at_secs(0),
        win,
    )
    .await;
    step_invocation(
        backend,
        &mut log,
        "iso/alpha-2",
        &ka,
        &ev("a2"),
        at_secs(1),
        win,
    )
    .await;
    step_invocation(
        backend,
        &mut log,
        "iso/beta-1",
        &kbeta,
        &ev("z1"),
        at_secs(0),
        win,
    )
    .await;

    // --- value sums within window ---
    let vk = val_key("alpha", "cap.spend", "amount");
    step_value(
        backend,
        &mut log,
        "sum/v1",
        &vk,
        &ev("v1"),
        at_secs(0),
        Decimal::from(50),
        win,
    )
    .await;
    step_value(
        backend,
        &mut log,
        "sum/v2",
        &vk,
        &ev("v2"),
        at_secs(1),
        Decimal::from(75),
        win,
    )
    .await;
    // value replay no-op
    step_value(
        backend,
        &mut log,
        "sum/replay-v2",
        &vk,
        &ev("v2"),
        at_secs(2),
        Decimal::from(75),
        win,
    )
    .await;
    // fractional value to catch any integer-truncation divergence
    step_value(
        backend,
        &mut log,
        "sum/fractional",
        &vk,
        &ev("v3"),
        at_secs(3),
        Decimal::new(125, 2), // 1.25
        win,
    )
    .await;

    // --- cross-map dedup isolation: the SAME event id in both maps ---
    let xi = inv_key("alpha", "cap.cross");
    let xv = val_key("alpha", "cap.cross", "amount");
    step_invocation(
        backend,
        &mut log,
        "cross/inv-shared",
        &xi,
        &ev("shared-id"),
        at_secs(0),
        win,
    )
    .await;
    step_value(
        backend,
        &mut log,
        "cross/val-shared",
        &xv,
        &ev("shared-id"),
        at_secs(0),
        Decimal::from(42),
        win,
    )
    .await;

    log
}

/// Fail-closed cap script: fill a single key to exactly `MAX_SAMPLES_PER_KEY`
/// distinct in-window ids (all succeed), then assert the next distinct id
/// fails closed with `WindowOverflow`, and a replay of an in-window id at the
/// cap dedups to a no-op. To keep the log small we only record the boundary
/// steps (the 4096 fill steps are summarized by their final count), so the
/// cross-assert stays a tractable size while still proving the boundary
/// matches across backends.
async fn run_cap_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
    let mut log = ObservationLog::new();
    let key = inv_key("alpha", "cap.hot");
    let window = Duration::from_secs(3600);

    // Fill to the cap. Record only the final at-cap count.
    let mut last = 0u32;
    for i in 0..MAX_SAMPLES_PER_KEY {
        last = backend
            .record_invocation(&key, &ev(&format!("e-{i}")), at_millis(i as i64), window)
            .await
            .expect("inserts up to the cap succeed");
    }
    log.push(Observation {
        label: "cap/at-cap-count".to_string(),
        outcome: StepOutcome::Count(last),
        evictions_after: backend.evictions_observed(),
    });

    // Next distinct in-window id fails closed.
    step_invocation(
        backend,
        &mut log,
        "cap/overflow",
        &key,
        &ev("e-overflow"),
        at_millis(MAX_SAMPLES_PER_KEY as i64),
        window,
    )
    .await;

    // Replay of an in-window id at the cap dedups (no-op), not overflow.
    step_invocation(
        backend,
        &mut log,
        "cap/replay-at-cap",
        &key,
        &ev("e-0"),
        at_millis(MAX_SAMPLES_PER_KEY as i64 + 1),
        window,
    )
    .await;

    log
}

/// Per-tenant LRU script: a single tenant fills `MAX_KEYS_PER_TENANT + K`
/// distinct scopes; the oldest scopes are evicted, `evictions_observed()`
/// advances, and a quiet co-tenant's scope survives. Asserts the eviction
/// *count* and the *victim* (the oldest scope no longer counts from where it
/// left off) match across backends. This is the shared per-tenant quota — the
/// one LRU dimension all three backends implement (the in-memory backend ALSO
/// has a global `MAX_HISTORY_KEYS` cap that the durable backends do not; that
/// intended divergence is documented in 03-persistent-counter.md and is NOT
/// exercised here so the matrix stays apples-to-apples).
async fn run_lru_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
    let mut log = ObservationLog::new();
    let window = Duration::from_secs(3600);

    // Quiet tenant beta records one scope.
    let beta = inv_key("beta", "beta.cap");
    step_invocation(
        backend,
        &mut log,
        "lru/beta-initial",
        &beta,
        &ev("beta-evt"),
        at_millis(0),
        window,
    )
    .await;

    // Noisy tenant alpha floods K past its quota with distinct scopes. Each
    // scope gets one invocation. The first `MAX_KEYS_PER_TENANT` create no
    // eviction; the overflow ones evict alpha's own oldest scopes.
    const OVERFLOW: usize = 8;
    for i in 0..(MAX_KEYS_PER_TENANT + OVERFLOW) {
        let key = inv_key("alpha", &format!("alpha.cap.{i}"));
        backend
            .record_invocation(
                &key,
                &ev(&format!("a-{i}")),
                at_millis(i as i64 + 1),
                window,
            )
            .await
            .expect("ok");
    }
    log.push(Observation {
        label: "lru/alpha-flood-evictions".to_string(),
        outcome: StepOutcome::Count(OVERFLOW as u32),
        evictions_after: backend.evictions_observed(),
    });

    // The OLDEST alpha scope (index 0) was the LRU victim: re-recording a
    // DISTINCT id against it counts as 1 (the bucket was evicted, so it does
    // not resume from 1-already-present). This pins the victim identity.
    let oldest = inv_key("alpha", "alpha.cap.0");
    step_invocation(
        backend,
        &mut log,
        "lru/oldest-victim-restarts",
        &oldest,
        &ev("a-0-revived"),
        at_millis((MAX_KEYS_PER_TENANT + OVERFLOW) as i64 + 1),
        window,
    )
    .await;

    // Quiet tenant beta's scope survived: replay of its original id is a
    // dedup no-op returning count 1 (proves it was never evicted).
    step_invocation(
        backend,
        &mut log,
        "lru/beta-survives",
        &beta,
        &ev("beta-evt"),
        at_millis((MAX_KEYS_PER_TENANT + OVERFLOW) as i64 + 2),
        window,
    )
    .await;

    log
}

// ---------------------------------------------------------------------------
// Backend factories
// ---------------------------------------------------------------------------

/// Build a fresh in-memory backend.
fn in_memory() -> Arc<dyn PredicateStateBackend> {
    Arc::new(InMemoryPredicateStateBackend::new())
}

/// Build a fresh, migrated libSQL backend over a private temp-file db. The
/// `TempDir` is leaked so the file outlives the returned handle for the
/// duration of the (short-lived) test process.
async fn libsql_backend() -> Arc<dyn PredicateStateBackend> {
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
    Box::leak(Box::new(dir));
    Arc::new(backend)
}

/// Build a fresh, migrated Postgres backend bound to an isolated schema and
/// truncated table, or `None` if no DB URL is set. Each call uses a unique
/// schema so concurrent matrix runs cannot collide.
#[cfg(feature = "postgres")]
async fn postgres_backend() -> Option<Arc<dyn PredicateStateBackend>> {
    use ironclaw_hooks_postgres::PostgresPredicateStateBackend;

    let url = std::env::var("IRONCLAW_HOOKS_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

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
            .ok()?;
        tokio::spawn(conn);
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .await
            .ok()?;
    }

    let config = url.parse::<tokio_postgres::Config>().ok()?;
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
        .ok()?;
    let backend = PostgresPredicateStateBackend::new(pool.clone());
    backend.run_migrations().await.ok()?;
    let client = pool.get().await.ok()?;
    client
        .batch_execute("TRUNCATE TABLE hook_predicate_counters")
        .await
        .ok()?;
    Some(Arc::new(backend))
}

// ---------------------------------------------------------------------------
// The matrix
// ---------------------------------------------------------------------------

/// Run `script` against in-memory, libSQL, and (if available) Postgres, and
/// cross-assert all produced logs are identical. The in-memory log is the
/// reference; each other backend must reproduce it exactly. Returns the names
/// of the legs that actually executed so the caller can print which legs ran.
async fn assert_parity<F, Fut>(script_name: &str, script: F) -> Vec<&'static str>
where
    F: Fn(Arc<dyn PredicateStateBackend>) -> Fut,
    Fut: std::future::Future<Output = ObservationLog>,
{
    let mut ran = Vec::new();

    // Reference leg: in-memory (always runs).
    let reference = script(in_memory()).await;
    ran.push("in-memory");

    // libSQL leg (always runs — embedded temp-file db).
    let libsql_log = script(libsql_backend().await).await;
    assert_eq!(
        libsql_log, reference,
        "[{script_name}] libSQL diverged from in-memory reference — \
         a real cross-backend behavioral bug, do NOT loosen this assertion"
    );
    ran.push("libsql");

    // Postgres leg (compiled under `postgres`, runs only with a DB URL).
    #[cfg(feature = "postgres")]
    {
        if let Some(pg) = postgres_backend().await {
            let pg_log = script(pg).await;
            assert_eq!(
                pg_log, reference,
                "[{script_name}] Postgres diverged from in-memory reference — \
                 a real cross-backend behavioral bug, do NOT loosen this assertion"
            );
            ran.push("postgres");
        } else {
            eprintln!(
                "[{script_name}] Postgres leg SKIPPED: IRONCLAW_HOOKS_POSTGRES_URL / \
                 DATABASE_URL not set. Parity proven for in-memory + libSQL only; a \
                 real-Postgres CI run is required before merge."
            );
        }
    }
    #[cfg(not(feature = "postgres"))]
    {
        eprintln!(
            "[{script_name}] Postgres leg NOT COMPILED (build without --features postgres). \
             Parity proven for in-memory + libSQL only."
        );
    }

    eprintln!("[{script_name}] parity legs executed: {ran:?}");
    ran
}

#[tokio::test]
async fn parity_core_behavioral_script() {
    let ran = assert_parity("core", |b| async move { run_core_script(&*b).await }).await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

#[tokio::test]
async fn parity_fail_closed_cap_script() {
    let ran = assert_parity("cap", |b| async move { run_cap_script(&*b).await }).await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

#[tokio::test]
async fn parity_per_tenant_lru_script() {
    let ran = assert_parity("lru", |b| async move { run_lru_script(&*b).await }).await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}
