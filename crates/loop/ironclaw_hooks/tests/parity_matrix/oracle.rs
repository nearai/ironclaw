//! Independent, hand-computed oracle logs — the count/sum/error sequence worked
//! out from the predicate-state semantics, NOT captured from any backend, so a
//! shared cross-backend bug still fails (every backend is asserted against
//! these). One builder per scenario in `super::scripts`.

use ironclaw_hooks::predicate_state::MAX_SAMPLES_PER_KEY;

use super::support::*;

/// Independent oracle for [`run_core_script`]: each step's count/sum, computed
/// by hand from the sliding-window + dedup + tenant-isolation semantics, NOT
/// captured from any backend. No LRU/global cap is touched, so every
/// `evictions_after` is 0.
pub(crate) fn expected_core_log() -> ObservationLog {
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
pub(crate) fn expected_cap_log() -> ObservationLog {
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
pub(crate) fn expected_lru_log() -> ObservationLog {
    const OVERFLOW: u64 = 8;
    const AFTER_VICTIM_REINSERT: u64 = OVERFLOW + 1; // 9
    vec![
        obs_count("lru/beta-initial", 1, 0),
        obs_count("lru/alpha-flood-evictions", OVERFLOW as u32, OVERFLOW),
        obs_count("lru/oldest-victim-restarts", 1, AFTER_VICTIM_REINSERT),
        obs_count("lru/beta-survives", 1, AFTER_VICTIM_REINSERT),
    ]
}

/// Independent oracle for [`run_lru_value_script`] — the per-tenant LRU
/// quota driven through `record_value` instead of `record_invocation`. The
/// quota/victim/co-tenant semantics are identical to [`expected_lru_log`]
/// (`enforce_caps` is shared, table-parameterized), so the eviction-count
/// trajectory matches; only the per-step outcome is a Sum (each fresh value
/// scope holds the single recorded `amount` of 5) rather than a Count.
///
/// beta records one value scope (sum 5, 0 evictions); alpha floods
/// `MAX_KEYS_PER_TENANT + OVERFLOW` distinct value scopes so exactly `OVERFLOW`
/// per-tenant evictions fire; the evicted oldest scope restarts (sum 5) and
/// triggers ONE MORE eviction (8 -> 9); beta's scope survives (replay dedups,
/// sum unchanged at 5, no new eviction).
pub(crate) fn expected_lru_value_log() -> ObservationLog {
    const OVERFLOW: u64 = 8;
    const AFTER_VICTIM_REINSERT: u64 = OVERFLOW + 1; // 9
    vec![
        obs_sum("lru-value/beta-initial", "5", 0),
        obs_sum("lru-value/alpha-flood-evictions", "5", OVERFLOW),
        obs_sum(
            "lru-value/oldest-victim-restarts",
            "5",
            AFTER_VICTIM_REINSERT,
        ),
        obs_sum("lru-value/beta-survives", "5", AFTER_VICTIM_REINSERT),
    ]
}

/// Independent oracle for [`run_global_cap_parity_script`]: every insert and
/// every replay returns count 1 with zero evictions — the per-tenant quota
/// never trips (5 < 2048) and the in-memory global cap (8192) is never reached
/// (200 < 8192), so all three backends retain every scope.
pub(crate) fn expected_global_cap_log() -> ObservationLog {
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
pub(crate) fn expected_multisample_lru_log() -> ObservationLog {
    vec![
        obs_count("multi/key0-second-sample", 2, 0),
        obs_count("multi/new-key-forces-eviction", 1, 1),
        obs_count("multi/key0-probe-after-eviction", 1, 2),
    ]
}
