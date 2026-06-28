//! Reborn integration-test framework — slice 3: storage-backend matrix.
//!
//! Two things this tier did not cover before:
//!   1. **Backend parity** — one golden scenario through BOTH
//!      `StorageMode::InMemory` and `StorageMode::LibSql` (real SQLite on a
//!      tmp `.db`, real SQL + migrations + CAS), asserting an identical
//!      outcome. The canonical `rstest` matrix exemplar for this tier.
//!   2. **Persistence correctness (LibSql-only)** — a write-then-read-back
//!      test that reopens the SQLite file through a *fresh* database handle and
//!      asserts the assistant reply survived to disk, proving real
//!      serialization + durability (design §3.8 guardrail).
//!
//! Runs under default features, no services, no keys, no Docker, no
//! `integration` feature — libSQL is an embedded SQLite file in a `TempDir`
//! dropped at test end.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_integration_greeting.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::reply::RebornScriptedReply;
use rstest::rstest;

/// Backend-parity self-test (design §7): the same golden turn must produce the
/// same finalized reply on every storage backend. The canonical matrix
/// exemplar — add a backend by adding one `#[case]`.
#[rstest]
#[case(StorageMode::InMemory)]
#[case(StorageMode::LibSql)]
#[tokio::test]
async fn backend_parity_replies_to_greeting(#[case] storage: StorageMode) {
    let harness = RebornIntegrationHarness::test_default()
        .storage(storage)
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("hi there")
        .await
        .expect("turn completes");
    harness
        .assert_reply_contains("Hello! How can I help?")
        .await
        .expect("reply finalized in thread history");
}

/// Persistence correctness, LibSql-only (design §3.8): the reply must survive
/// to the SQLite file and read back through a *fresh* database handle — real
/// serialization + durability, not an in-process cache. InMemory cannot make
/// this assertion (nothing reaches disk), so this test legitimately requires
/// `StorageMode::LibSql`.
#[tokio::test]
async fn libsql_persists_reply_across_reopen() {
    let harness = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([RebornScriptedReply::text("durable answer")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("remember this")
        .await
        .expect("turn completes");
    harness
        .assert_reply_persists_after_reopen("durable answer")
        .await
        .expect("reply durable in reopened SQLite");
}

/// Guard: `assert_reply_persists_after_reopen` must return `Err` when the
/// expected text is absent — proving the LibSql reopen read-back assertion is
/// not vacuously green (it really inspects the reopened on-disk history, and a
/// wrong expectation fails). Mirrors the negative-guard tests the other slices
/// carry (e.g. `assertions_fail_when_tool_did_not_run`).
#[tokio::test]
async fn persistence_assertion_fails_on_mismatch_after_reopen() {
    let harness = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([RebornScriptedReply::text("durable answer")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("remember this")
        .await
        .expect("turn completes");
    assert!(
        harness
            .assert_reply_persists_after_reopen("a reply that was never produced")
            .await
            .is_err(),
        "reopen assertion must fail when the expected text is absent from persisted history"
    );
}
