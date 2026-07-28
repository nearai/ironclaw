//! Reborn integration-test framework — storage-backend matrix.
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
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
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
#[case(StorageMode::Postgres)]
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

/// Persistence correctness (design §3.8): the reply must survive to the
/// SQLite file and read back through a fresh database handle, not an
/// in-process cache. InMemory cannot make this assertion (nothing reaches disk).
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
/// expected text is absent, proving the reopen assertion isn't vacuously
/// green — it inspects real on-disk history.
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

/// Inventory gate for persistence backends (#6524 workstream 4: "apply the
/// same pattern to ... persistence backends and other closed product
/// surfaces").
///
/// The capability inventory works because the denominator comes from
/// production, not from a hand-kept list: adding a capability without
/// classifying it fails CI. Storage backends had the coverage but not the
/// gate — all three variants happen to be exercised today, and nothing would
/// have said so if a fourth arrived uncovered.
mod backend_inventory {
    use super::*;

    /// Every backend the harness can run. Adding a variant to `StorageMode`
    /// without adding it here fails to compile at `exhaustiveness_guard`
    /// below, so this list cannot silently fall behind the enum.
    const ALL_STORAGE_MODES: [StorageMode; 3] = [
        StorageMode::InMemory,
        StorageMode::LibSql,
        StorageMode::Postgres,
    ];

    /// Compile-time half of the gate.
    ///
    /// A non-exhaustive match is a compile error, so a new `StorageMode`
    /// variant breaks the build here rather than slipping through as an
    /// untested backend. This function is never called; its body is the
    /// assertion.
    #[allow(dead_code)]
    fn exhaustiveness_guard(mode: StorageMode) {
        match mode {
            StorageMode::InMemory | StorageMode::LibSql | StorageMode::Postgres => {}
        }
    }

    /// Runtime half: every backend in the list is actually run by the parity
    /// case above.
    ///
    /// Reads this file's own source rather than trusting the list, because
    /// the failure being prevented is a variant that exists and is declared
    /// but never handed to a test.
    #[test]
    fn every_storage_backend_is_exercised_by_the_parity_matrix() {
        let source = include_str!("backend_matrix.rs");
        // Only the `#[case(...)]` attributes count as coverage. Searching the
        // whole file would match this module's own list and pass vacuously.
        let cases: Vec<&str> = source
            .lines()
            .filter(|line| line.trim_start().starts_with("#[case"))
            .collect();
        assert!(
            !cases.is_empty(),
            "no #[case] attributes found; the gate would pass vacuously"
        );

        for mode in ALL_STORAGE_MODES {
            let needle = format!("StorageMode::{mode:?}");
            assert!(
                cases.iter().any(|line| line.contains(&needle)),
                "storage backend {mode:?} is declared but never exercised by \
                 the parity matrix; add `#[case({needle})]` or remove the \
                 variant. Backends without a case are the shape the capability \
                 inventory exists to prevent."
            );
        }
    }
}
