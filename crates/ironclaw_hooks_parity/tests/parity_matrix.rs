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

// `#[path]` keeps these submodules in the `parity_matrix/` subdirectory: files
// directly under `tests/` are each compiled as their own integration-test
// binary, but files in a subdirectory are not — so the shared support/scenario/
// oracle code lives in `tests/parity_matrix/` and is pulled in here explicitly.
#[path = "parity_matrix/oracle.rs"]
mod oracle;
#[path = "parity_matrix/scripts.rs"]
mod scripts;
#[path = "parity_matrix/support.rs"]
mod support;

use oracle::*;
use scripts::*;
use support::*;

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
