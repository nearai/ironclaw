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
    /// Every backend the harness can run, read from the enum's own source.
    ///
    /// Deliberately not a hand-kept array. A list restating the enum can be
    /// left behind: a new `StorageMode` variant only has to satisfy the
    /// compiler, and extending a `|` pattern is easier to remember than an
    /// array three screens away. Parsing the declaration means the
    /// denominator cannot disagree with the type it describes.
    fn declared_storage_modes() -> Vec<String> {
        let source = include_str!("support/builder.rs");
        let body = source
            .split_once("pub enum StorageMode {")
            .expect("StorageMode is declared in support/builder.rs")
            .1
            .split_once("\n}")
            .expect("the enum declaration is brace-terminated")
            .0;
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("#["))
            // Variants are bare identifiers here; a payload-carrying variant
            // would need its own handling, and the assertion below would
            // notice the parse going wrong before it went quiet.
            .map(|line| line.trim_end_matches(',').trim().to_string())
            .filter(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
            })
            .collect()
    }

    /// The attribute block attached to the parity test, and nothing else.
    ///
    /// Scoped to that one item on purpose: scanning the whole file would let
    /// an unrelated `#[case(StorageMode::Postgres)]` elsewhere stand in for
    /// the parity coverage this gate is about. Returning the block as one
    /// string also makes multiline attributes work without parsing them.
    fn parity_attribute_block() -> String {
        let source = include_str!("backend_matrix.rs");
        let lines: Vec<&str> = source.lines().collect();
        let signature = lines
            .iter()
            .position(|line| line.contains("async fn backend_parity_replies_to_greeting"))
            .expect("the parity matrix test is declared in this file");
        // Walk back to the blank line separating this item from the previous
        // one; everything between is this item's own doc and attributes.
        let mut first = signature;
        while first > 0 && !lines[first - 1].trim().is_empty() {
            first -= 1;
        }
        lines[first..signature].join("\n")
    }

    /// Every declared backend is exercised by the parity matrix.
    #[test]
    fn every_storage_backend_is_exercised_by_the_parity_matrix() {
        let modes = declared_storage_modes();
        // Guard against a silent parse failure: an empty or truncated list
        // would make every assertion below vacuous.
        assert!(
            modes.len() >= 3,
            "parsed only {modes:?} from the StorageMode declaration; the \
             parser has drifted from the enum's shape and this gate would \
             pass vacuously"
        );
        for expected in ["InMemory", "LibSql", "Postgres"] {
            assert!(
                modes.iter().any(|mode| mode == expected),
                "the StorageMode parser stopped finding `{expected}`; it reads \
                 `pub enum StorageMode` from support/builder.rs -- check \
                 whether the declaration moved or changed shape. Until this is \
                 fixed the gate cannot see new backends either. Parsed: {modes:?}"
            );
        }

        let attributes = parity_attribute_block();
        assert!(
            attributes.contains("#[case"),
            "found no #[case] attributes on the parity matrix; the gate would \
             pass vacuously. Block was:\n{attributes}"
        );

        for mode in &modes {
            let needle = format!("StorageMode::{mode}");
            assert!(
                attributes.contains(&needle),
                "storage backend `{mode}` is declared in StorageMode but never \
                 exercised by the parity matrix; add `#[case({needle})]` to \
                 `backend_parity_replies_to_greeting`, or remove the variant. \
                 A backend with no case is the shape this gate exists to catch."
            );
        }
    }
}
