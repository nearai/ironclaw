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
//!   parity guarantee is only proven for {in-memory, libSQL}. Set
//!   `IRONCLAW_REQUIRE_POSTGRES=1` (CI does) to turn a missing/unreachable
//!   Postgres into a HARD failure so a skip cannot masquerade as a green
//!   full-matrix run. A real-Postgres CI run is required before merge to fully
//!   exercise the matrix (same caveat as #3933).
//!
//! # Why a captured log cross-checked against an independent oracle
//!
//! Capturing the full per-step output of one backend and asserting the others
//! reproduce it exactly means a NEW behavioral divergence (a backend that
//! fails closed at a different boundary, dedups differently, or returns a
//! different sum) surfaces as a concrete `assert_eq!` diff naming the diverging
//! step — instead of silently passing because each backend's own bespoke
//! assertions happened to be loose.
//!
//! Cross-backend equality alone is necessary but not sufficient: if two
//! backends shared the SAME semantic bug they would agree with each other and
//! still pass. So each script ALSO carries an independent, hand-computed
//! `expected_*` oracle log (the count/sum/error sequence worked out from the
//! semantics, not captured from any backend), and every backend — including the
//! in-memory reference — is asserted against that oracle. A shared bug now
//! fails because both backends diverge from the oracle. If this file ever
//! fails, it has found a real bug in a backend (or a stale oracle — fix the
//! bug, do NOT silently update the oracle to match a regressed backend); do NOT
//! loosen the assertion to make it pass.

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
/// one LRU dimension all three backends implement. The in-memory backend ALSO
/// has a global `MAX_HISTORY_KEYS` cap that the durable backends do not;
/// `run_global_cap_parity_script` asserts the three backends AGREE in the
/// regime below that cap (no global eviction fires), and the divergence ABOVE
/// 8192 total scopes is an intentional memory-bound difference documented in
/// 03-persistent-counter.md. This per-tenant LRU script stays under both caps
/// so a divergence here localizes to the per-tenant dimension.
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

/// Global-cap parity script: many tenants each record a handful of distinct
/// scopes, every tenant staying well under `MAX_KEYS_PER_TENANT` and the total
/// staying well under the in-memory `MAX_HISTORY_KEYS` (8192) global cap. Every
/// insert goes through the under-per-tenant-quota branch that — on the in-memory
/// backend — *consults* the global cap, but the threshold is never crossed, so
/// no global eviction fires on ANY backend.
///
/// This makes the cross-backend equality assertion meaningful for the
/// global-cap dimension instead of silently excluding it: all three backends
/// must retain every scope with ZERO evictions. The in-memory backend's global
/// cap and the durable backends' lack of one are reconciled in this regime —
/// they only diverge ABOVE 8192 total scopes, which is an intentional
/// memory-bound divergence documented in 03-persistent-counter.md and not
/// exercised here (inserting 8192+ scopes through a per-op-connection durable
/// backend is prohibitively slow for a unit test; the per-backend contract
/// suites' `no_global_key_cap_only_per_tenant` cover the durable side directly).
async fn run_global_cap_parity_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
    let mut log = ObservationLog::new();
    let window = Duration::from_secs(3600);

    const TENANTS: usize = 40;
    const SCOPES_PER_TENANT: usize = 5;

    // Phase 1: insert TENANTS × SCOPES_PER_TENANT distinct scopes.
    for t in 0..TENANTS {
        for s in 0..SCOPES_PER_TENANT {
            let key = inv_key(&format!("gtenant{t}"), &format!("gcap.{s}"));
            step_invocation(
                backend,
                &mut log,
                &format!("global/insert-{t}-{s}"),
                &key,
                &ev(&format!("g-{t}-{s}")),
                at_millis((t * SCOPES_PER_TENANT + s) as i64),
                window,
            )
            .await;
        }
    }

    // Phase 2: replay every scope's original id (dedup no-op). If a global cap
    // had evicted the earliest tenants' scopes, those would restart at 1 here;
    // with no global cap (durable) or the cap unreached (in-memory) every scope
    // survives and the replay returns its stable count of 1. Equality across
    // backends in this phase is the load-bearing global-cap parity assertion.
    for t in 0..TENANTS {
        for s in 0..SCOPES_PER_TENANT {
            let key = inv_key(&format!("gtenant{t}"), &format!("gcap.{s}"));
            step_invocation(
                backend,
                &mut log,
                &format!("global/replay-{t}-{s}"),
                &key,
                &ev(&format!("g-{t}-{s}")),
                at_millis((TENANTS * SCOPES_PER_TENANT + t * SCOPES_PER_TENANT + s) as i64),
                window,
            )
            .await;
        }
    }

    log
}

/// Multi-sample-per-key per-tenant LRU script — the MIN-vs-MAX victim-rule
/// discriminator (regression guard for the Postgres `MAX(ts)` bug fixed in
/// 0c102a631, which all three backends now resolve as oldest-front
/// `MIN(occurred_at)`).
///
/// The existing `run_lru_script` puts exactly ONE sample per key, so each
/// key's `MIN(ts) == MAX(ts)` and a backend that ranked eviction victims by
/// newest-activity (`MAX`) instead of oldest-front (`MIN`) would pick the SAME
/// victim and the divergence would be invisible. This script gives the
/// oldest-front key a SECOND, very RECENT sample so its `MIN(ts)` (oldest) and
/// `MAX(ts)` (newest) point at DIFFERENT victims, making the rule observable:
///
/// 1. Tenant gamma fills exactly `MAX_KEYS_PER_TENANT` distinct keys, one
///    sample each, at strictly increasing timestamps (key 0 oldest-front, key
///    N-1 newest). No eviction yet — each insert was below the quota.
/// 2. Add a SECOND sample to key 0 with a far-RECENT timestamp. Key 0 now has
///    `MIN(ts)` = the original (oldest of ALL keys) but `MAX(ts)` = the newest
///    of all keys. Existing key, so no eviction; count returns 2.
/// 3. Insert a NEW key, pushing gamma over quota and forcing one eviction.
///    - oldest-front (`MIN`, correct): key 0 is still the global oldest-front →
///      key 0 is the victim.
///    - newest-activity (`MAX`, the old Postgres bug): key 0 looks newest → it
///      is SPARED and key 1 is evicted instead.
/// 4. Probe key 0 with a fresh distinct id. This is the load-bearing
///    discriminator:
///    - `MIN` (correct): key 0 was evicted, so it is a fresh bucket → count 1.
///      (Re-inserting key 0 finds gamma at quota again and evicts the new
///      oldest-front, key 1 — a second eviction.)
///    - `MAX` (buggy): key 0 was spared and still holds its 2 in-window
///      samples → the fresh id makes count 3.
///
/// A backend that regressed to `MAX`-victim selection produces count 3 at the
/// probe and fails against the oracle (which pins count 1).
async fn run_multisample_lru_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
    let mut log = ObservationLog::new();
    // Wide window so nothing trims across the whole script.
    let window = Duration::from_secs(1_000_000);

    // (1) Fill exactly MAX_KEYS_PER_TENANT keys, one sample each, increasing ts.
    // Recorded directly (not logged) — this is setup, not an observation.
    for i in 0..MAX_KEYS_PER_TENANT {
        let key = inv_key("gamma", &format!("gamma.cap.{i}"));
        backend
            .record_invocation(
                &key,
                &ev(&format!("g-{i}")),
                at_millis(i as i64 + 1),
                window,
            )
            .await
            .expect("fill ok");
    }

    // (2) Second, far-recent sample on key 0. Existing key → no eviction.
    // Count is 2 (two in-window samples); MIN(ts) stays oldest, MAX(ts) newest.
    let key0 = inv_key("gamma", "gamma.cap.0");
    step_invocation(
        backend,
        &mut log,
        "multi/key0-second-sample",
        &key0,
        &ev("g-0-recent"),
        at_millis(1_000_000),
        window,
    )
    .await;

    // (3) New key pushes gamma over quota → exactly one eviction fires.
    let key_new = inv_key("gamma", "gamma.cap.NEW");
    step_invocation(
        backend,
        &mut log,
        "multi/new-key-forces-eviction",
        &key_new,
        &ev("g-new"),
        at_millis(1_000_001),
        window,
    )
    .await;

    // (4) Probe key 0 — the MIN-vs-MAX discriminator. Under oldest-front (MIN)
    // key 0 was the victim, so a fresh id restarts it at count 1 (and triggers
    // a SECOND eviction of the new oldest-front key). Under newest-activity
    // (MAX) key 0 was spared and the fresh id makes count 3.
    step_invocation(
        backend,
        &mut log,
        "multi/key0-probe-after-eviction",
        &key0,
        &ev("g-0-probe"),
        at_millis(1_000_002),
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
        .batch_execute("TRUNCATE TABLE hooks_predicate_invocations, hooks_predicate_values")
        .await
        .ok()?;
    Some(Arc::new(backend))
}

// ---------------------------------------------------------------------------
// The matrix
// ---------------------------------------------------------------------------

/// Process-global async mutex serializing the libSQL legs across the parallel
/// `#[tokio::test]` parity functions. See the call site for why concurrent
/// independent libSQL `Database` handles must not run heavy fills at once.
fn libsql_serial_guard() -> &'static tokio::sync::Mutex<()> {
    static GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// When `IRONCLAW_REQUIRE_POSTGRES=1`, a missing/unreachable Postgres backend is
/// a HARD failure rather than a silent skip. CI sets this so a misconfigured DB
/// (or a forgotten `--features postgres`) cannot turn the Postgres parity leg
/// into a green skip-pass; local runs without the env var still skip cleanly.
fn require_postgres_or_skip(script_name: &str) {
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
fn obs_count(label: &str, count: u32, evictions_after: u64) -> Observation {
    Observation {
        label: label.to_string(),
        outcome: StepOutcome::Count(count),
        evictions_after,
    }
}

fn obs_sum(label: &str, sum: &str, evictions_after: u64) -> Observation {
    Observation {
        label: label.to_string(),
        outcome: StepOutcome::Sum(sum.to_string()),
        evictions_after,
    }
}

fn obs_overflow(label: &str, evictions_after: u64) -> Observation {
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
async fn assert_parity<F, Fut>(
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
        script(libsql_backend().await).await
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
        if let Some(pg) = postgres_backend().await {
            let pg_log = script(pg).await;
            assert_eq!(
                pg_log, expected,
                "[{script_name}] Postgres diverged from the oracle — \
                 a real cross-backend behavioral bug, do NOT loosen this assertion"
            );
            ran.push("postgres");
        } else {
            require_postgres_or_skip(script_name);
        }
    }
    #[cfg(not(feature = "postgres"))]
    {
        require_postgres_or_skip(script_name);
    }

    eprintln!("[{script_name}] parity legs executed: {ran:?}");
    ran
}

/// Independent oracle for [`run_core_script`]: each step's count/sum, computed
/// by hand from the sliding-window + dedup + tenant-isolation semantics, NOT
/// captured from any backend. No LRU/global cap is touched, so every
/// `evictions_after` is 0.
fn expected_core_log() -> ObservationLog {
    vec![
        // counting within window, then dedup replay, then a fresh id
        obs_count("count/e1", 1, 0),
        obs_count("count/e2", 2, 0),
        obs_count("count/e3", 3, 0),
        obs_count("count/replay-e2", 3, 0), // replay dedups, no advance
        obs_count("count/e4", 4, 0),
        // far-future event (t=10_000s) trims everything older than now-60s
        obs_count("count/far-future", 1, 0),
        // exact-cutoff retain boundary: t=0 entry is `< cutoff(=0)` false => kept
        obs_count("boundary/t0", 1, 0),
        obs_count("boundary/at-cutoff", 2, 0),
        // tenant isolation: beta never inherits alpha's count
        obs_count("iso/alpha-1", 1, 0),
        obs_count("iso/alpha-2", 2, 0),
        obs_count("iso/beta-1", 1, 0),
        // value sums within window, with a dedup replay and a fractional add
        obs_sum("sum/v1", "50", 0),
        obs_sum("sum/v2", "125", 0),
        obs_sum("sum/replay-v2", "125", 0),
        obs_sum("sum/fractional", "126.25", 0), // 125 + 1.25
        // cross-map dedup isolation: same id in both maps counts independently
        obs_count("cross/inv-shared", 1, 0),
        obs_sum("cross/val-shared", "42", 0),
    ]
}

/// Independent oracle for [`run_cap_script`]: fill to the cap (count ==
/// `MAX_SAMPLES_PER_KEY`), the next distinct id fails closed, and a replay of an
/// in-window id dedups (count unchanged at the cap). No LRU eviction occurs (a
/// single hot key under the per-tenant quota), so `evictions_after` is 0.
fn expected_cap_log() -> ObservationLog {
    vec![
        obs_count("cap/at-cap-count", MAX_SAMPLES_PER_KEY as u32, 0),
        obs_overflow("cap/overflow", 0),
        obs_count("cap/replay-at-cap", MAX_SAMPLES_PER_KEY as u32, 0),
    ]
}

/// Independent oracle for [`run_lru_script`]: beta records 1 scope; alpha floods
/// `MAX_KEYS_PER_TENANT + OVERFLOW` distinct scopes so exactly `OVERFLOW`
/// per-tenant evictions fire; alpha's oldest scope is the victim and restarts at
/// count 1; beta's scope survives (replay dedups to count 1).
///
/// `evictions_after` after the flood equals `OVERFLOW` (8) — the per-tenant
/// quota is the ONLY LRU dimension all three backends share. Re-inserting the
/// evicted oldest scope (`oldest-victim-restarts`) finds alpha at its quota
/// again, so it triggers ONE MORE per-tenant eviction (8 -> 9). The final
/// `beta-survives` step is a replay of an existing beta scope (no new key), so
/// it adds no eviction and the counter stays at 9.
///
/// The in-memory backend's additional global `MAX_HISTORY_KEYS` cap is never
/// reached here (total scopes stay well under 8192), so it contributes no extra
/// evictions and the durable backends (which have no global cap at all) match
/// exactly.
fn expected_lru_log() -> ObservationLog {
    const OVERFLOW: u64 = 8;
    const AFTER_VICTIM_REINSERT: u64 = OVERFLOW + 1; // 9
    vec![
        obs_count("lru/beta-initial", 1, 0),
        obs_count("lru/alpha-flood-evictions", OVERFLOW as u32, OVERFLOW),
        obs_count("lru/oldest-victim-restarts", 1, AFTER_VICTIM_REINSERT),
        obs_count("lru/beta-survives", 1, AFTER_VICTIM_REINSERT),
    ]
}

/// Independent oracle for [`run_global_cap_parity_script`]: every insert and
/// every replay returns count 1 with zero evictions — the per-tenant quota
/// never trips (5 < 2048) and the in-memory global cap (8192) is never reached
/// (200 < 8192), so all three backends retain every scope.
fn expected_global_cap_log() -> ObservationLog {
    const TENANTS: usize = 40;
    const SCOPES_PER_TENANT: usize = 5;
    let mut log = ObservationLog::new();
    for t in 0..TENANTS {
        for s in 0..SCOPES_PER_TENANT {
            log.push(obs_count(&format!("global/insert-{t}-{s}"), 1, 0));
        }
    }
    for t in 0..TENANTS {
        for s in 0..SCOPES_PER_TENANT {
            log.push(obs_count(&format!("global/replay-{t}-{s}"), 1, 0));
        }
    }
    log
}

#[tokio::test]
async fn parity_core_behavioral_script() {
    let ran = assert_parity("core", expected_core_log(), |b| async move {
        run_core_script(&*b).await
    })
    .await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

#[tokio::test]
async fn parity_fail_closed_cap_script() {
    let ran = assert_parity("cap", expected_cap_log(), |b| async move {
        run_cap_script(&*b).await
    })
    .await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

#[tokio::test]
async fn parity_per_tenant_lru_script() {
    let ran = assert_parity("lru", expected_lru_log(), |b| async move {
        run_lru_script(&*b).await
    })
    .await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

/// Independent oracle for [`run_multisample_lru_script`] — the MIN-vs-MAX
/// victim-rule discriminator. All three backends use oldest-front
/// (`MIN(occurred_at)`) victim selection, so:
///
/// - `key0-second-sample`: existing key gains a second in-window sample →
///   count 2, no eviction (evictions stay 0).
/// - `new-key-forces-eviction`: gamma is at `MAX_KEYS_PER_TENANT`; the new key
///   forces eviction of the oldest-front key (key 0) → the new key is fresh
///   (count 1) and one eviction fires (evictions 0 → 1).
/// - `key0-probe-after-eviction`: under the correct `MIN` rule key 0 WAS the
///   victim, so a fresh id restarts it at count 1; re-inserting it finds gamma
///   at quota again and evicts the new oldest-front (key 1) → a SECOND eviction
///   (evictions 1 → 2). A backend that regressed to `MAX`-victim selection
///   would have SPARED key 0 (it looked newest) and this step would observe
///   count 3 with no second eviction — diverging from this oracle and failing.
fn expected_multisample_lru_log() -> ObservationLog {
    vec![
        obs_count("multi/key0-second-sample", 2, 0),
        obs_count("multi/new-key-forces-eviction", 1, 1),
        obs_count("multi/key0-probe-after-eviction", 1, 2),
    ]
}

#[tokio::test]
async fn parity_global_cap_script() {
    let ran = assert_parity("global-cap", expected_global_cap_log(), |b| async move {
        run_global_cap_parity_script(&*b).await
    })
    .await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}

/// Multi-sample-per-key LRU victim-rule parity (MIN oldest-front vs MAX
/// newest-activity). Regression guard for the Postgres `MAX(ts)` bug fixed in
/// 0c102a631: with more than one sample per key, a `MAX`-victim backend evicts
/// a DIFFERENT key than the oldest-front backends and fails the oracle here.
#[tokio::test]
async fn parity_multisample_lru_victim_rule() {
    let ran = assert_parity(
        "multisample-lru",
        expected_multisample_lru_log(),
        |b| async move { run_multisample_lru_script(&*b).await },
    )
    .await;
    assert!(ran.contains(&"in-memory") && ran.contains(&"libsql"));
}
