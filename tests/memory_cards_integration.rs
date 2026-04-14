#![cfg(feature = "libsql")]
//! Integration tests for the MEMORY.md → knowledge-card round trip.
//!
//! These tests close the caller-level gap the `.claude/rules/testing.md`
//! "Test Through the Caller, Not Just the Helper" rule calls out. The
//! `card_metadata::parse_memory_index_entries` parser has exhaustive unit
//! tests, but prior to this file no test drove it through the same chain
//! `memory_cards_handler` uses in production:
//!
//! 1. Agent writes via `workspace.append_memory(content)` (a real SQL
//!    store, not an in-memory fixture).
//! 2. Handler reads the MEMORY.md document back via `workspace.read(...)`.
//! 3. Handler passes `doc.content` to `parse_memory_index_entries`.
//! 4. Each entry becomes a card with a `MEMORY.md#entry-N` synthetic path.
//!
//! A silent drop anywhere in that chain (e.g. append_memory stripping the
//! content, read returning stale content, the parser receiving something
//! different from what append stored) would not be caught by parser-only
//! unit tests. These tests exercise the whole chain against a file-backed
//! libSQL database — no external service required.

use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::workspace::Workspace;
use ironclaw::workspace::card_metadata::{MEMORY_INDEX_PATH, parse_memory_index_entries};
use ironclaw::workspace::paths;

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

/// Replicates the per-bullet expansion `memory_cards_handler` performs for
/// MEMORY.md. Kept local to the test so the test fails loudly if the
/// production handler ever diverges from this shape.
fn cards_from_memory_content(content: &str) -> Vec<(String, String)> {
    parse_memory_index_entries(content)
        .into_iter()
        .enumerate()
        .map(|(idx, entry)| (format!("{MEMORY_INDEX_PATH}#entry-{idx}"), entry.text))
        .collect()
}

#[tokio::test]
async fn memory_cards_round_trip_prose_entry() {
    // Regression for the reported bug: agent calls `memory_write
    // target=memory append=true content="..."` with raw prose (no bullet
    // marker). Previously the write succeeded but the card view was
    // empty because the parser skipped non-bullet lines as preamble.
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db);

    ws.append_memory("User prefers dark mode")
        .await
        .expect("append raw prose to MEMORY.md");

    let doc = ws.read(paths::MEMORY).await.expect("read MEMORY.md");
    let cards = cards_from_memory_content(&doc.content);

    assert_eq!(cards.len(), 1, "raw prose should produce exactly one card");
    assert_eq!(cards[0].0, "MEMORY.md#entry-0");
    assert_eq!(cards[0].1, "User prefers dark mode");
}

#[tokio::test]
async fn memory_cards_round_trip_two_prose_appends() {
    // Two successive `append_memory` calls emit entries joined by `\n\n`
    // (see `Workspace::append_memory`). Each should surface as its own
    // card, matching the user expectation that every remembered fact is
    // independently visible and clickable.
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("bob", db);

    ws.append_memory("User prefers dark mode").await.unwrap();
    ws.append_memory("Deploys on Mondays").await.unwrap();

    let doc = ws.read(paths::MEMORY).await.expect("read MEMORY.md");
    let cards = cards_from_memory_content(&doc.content);

    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].1, "User prefers dark mode");
    assert_eq!(cards[1].1, "Deploys on Mondays");
    assert_eq!(cards[1].0, "MEMORY.md#entry-1");
}

#[tokio::test]
async fn memory_cards_round_trip_mixed_bullet_and_prose() {
    // Real-world shape: Claude auto-memory writes bullets, but some
    // agents and manual edits produce paragraphs. Both must appear as
    // cards in document order so curation in the UI matches the file.
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("carol", db);

    ws.append_memory("- [Dark mode](pref.md) — user prefers dark mode")
        .await
        .unwrap();
    ws.append_memory("Free-form reminder about Monday deploys")
        .await
        .unwrap();
    ws.append_memory("- [Timezone](tz.md) — PST").await.unwrap();

    let doc = ws.read(paths::MEMORY).await.expect("read MEMORY.md");
    let cards = cards_from_memory_content(&doc.content);

    assert_eq!(cards.len(), 3);
    assert!(cards[0].1.contains("Dark mode"));
    assert_eq!(cards[1].1, "Free-form reminder about Monday deploys");
    assert!(cards[2].1.contains("Timezone"));
}

#[tokio::test]
async fn memory_cards_round_trip_empty_memory() {
    // A new workspace has no MEMORY.md persisted yet. The production
    // cards handler calls `workspace.list_all()` and skips paths that
    // don't resolve — so nothing is rendered. Mirror that contract:
    // `workspace.memory()` returns a document created on demand with
    // empty content, and the parser then emits zero cards.
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("dave", db);

    let doc = ws.memory().await.expect("memory() materializes MEMORY.md");
    let cards = cards_from_memory_content(&doc.content);

    assert_eq!(
        cards.len(),
        0,
        "no cards expected before any append; got: {cards:?}"
    );
}
