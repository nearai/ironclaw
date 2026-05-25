//! The deterministic scripted scenarios fed to every backend. Each `run_*`
//! function drives a fixed `record_invocation`/`record_value` sequence and
//! captures the normalized `ObservationLog`; the matching oracle lives in
//! `super::oracle`.

use std::time::Duration;

use ironclaw_hooks::predicate_state::{
    MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateStateBackend,
};
use rust_decimal::Decimal;

use super::support::*;

/// Core behavioral script: counting, summing, window-trim, dedup/replay,
/// tenant isolation, cross-map dedup isolation, and the exact-cutoff retain
/// boundary. Deterministic — no wall-clock, no randomness. This exercises
/// every guarantee the three backends share (it deliberately stays under the
/// per-key cap and the per-tenant quota; those are scripted separately so a
/// divergence localizes).
pub(crate) async fn run_core_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
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
pub(crate) async fn run_cap_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
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
pub(crate) async fn run_lru_script(backend: &dyn PredicateStateBackend) -> ObservationLog {
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
pub(crate) async fn run_global_cap_parity_script(
    backend: &dyn PredicateStateBackend,
) -> ObservationLog {
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
pub(crate) async fn run_multisample_lru_script(
    backend: &dyn PredicateStateBackend,
) -> ObservationLog {
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
