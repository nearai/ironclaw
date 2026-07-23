//! IronClaw integration-test framework — storage-backend matrix.
//!
//! Covers: backend parity (one golden scenario through `StorageMode::InMemory`
//! and `StorageMode::LibSql`, asserting an identical outcome — the canonical
//! `rstest` matrix exemplar for this tier) and LibSql persistence correctness
//! (write-then-reopen through a fresh database handle, design §3.8 guardrail).
//!
//! Runs under default features, no services, no keys, no Docker, no
//! `integration` feature — libSQL is an embedded SQLite file in a `TempDir`
//! dropped at test end.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::{IronClawIntegrationHarness, StorageMode};
use ironclaw_support::reply::IronClawScriptedReply;
use rstest::rstest;

/// Backend-parity self-test (design §7): the same golden turn must produce the
/// same finalized reply on every storage backend. The canonical matrix
/// exemplar — add a backend by adding one `#[case]`.
#[rstest]
#[case(StorageMode::InMemory)]
#[case(StorageMode::LibSql)]
#[case(StorageMode::Postgres)]
#[tokio::test]
async fn backend_parity_replies_to_greeting(#[case] storage: StorageMode) {
    let harness = IronClawIntegrationHarness::test_default()
        .storage(storage)
        .script([IronClawScriptedReply::text("Hello! How can I help?")])
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

/// Persistence correctness (design §3.8): the reply must survive to the
/// SQLite file and read back through a fresh database handle, not an
/// in-process cache. InMemory cannot make this assertion (nothing reaches disk).
#[tokio::test]
async fn libsql_persists_reply_across_reopen() {
    let harness = IronClawIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([IronClawScriptedReply::text("durable answer")])
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
/// expected text is absent, proving the reopen assertion isn't vacuously
/// green — it inspects real on-disk history.
#[tokio::test]
async fn persistence_assertion_fails_on_mismatch_after_reopen() {
    let harness = IronClawIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .script([IronClawScriptedReply::text("durable answer")])
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
