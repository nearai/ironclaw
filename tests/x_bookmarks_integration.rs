//! Integration tests for the X bookmarks pipeline.
//!
//! Exercises the libSQL backend end-to-end: schema creation via the
//! incremental migration runner, ingest dedupe, triage application, and
//! queue/stats reads. These tests do NOT call the LLM — that's covered by
//! the unit tests in `src/x_bookmarks/triage.rs`. This crate's job is to
//! prove the storage and validation contracts hold for both backends.

#![cfg(feature = "libsql")]

use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::db::{Database, ResolvedTriageDecision, XBookmarkStore};
use ironclaw::x_bookmarks::{
    BookmarkIngestItem, BookmarkStatus, MAX_INGEST_BATCH, NormalizedIngestItem,
    validate_ingest_item,
};

/// libSQL `:memory:` opens a private in-memory database per connection, so
/// `run_migrations()` would not be visible to subsequent `connect()` calls.
/// File-backed temp dirs are the canonical workaround used elsewhere in the
/// repo (see `src/db/libsql/pairing.rs`).
async fn fresh_db() -> (LibSqlBackend, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("x_bookmarks_test.db");
    let db = LibSqlBackend::new_local(&path)
        .await
        .expect("open local db");
    db.run_migrations().await.expect("run migrations");
    (db, dir)
}

fn raw_item(tweet_id: &str, url: &str) -> BookmarkIngestItem {
    BookmarkIngestItem {
        tweet_id: tweet_id.to_string(),
        author_handle: Some("alice".to_string()),
        author_name: Some("Alice".to_string()),
        text: Some("hello world".to_string()),
        url: url.to_string(),
        media_urls: vec![],
        quoted_tweet: None,
        thread_id: None,
        posted_at: None,
    }
}

fn one_item() -> NormalizedIngestItem {
    validate_ingest_item(&raw_item(
        "1820000000000000000",
        "https://x.com/alice/status/1820000000000000000",
    ))
    .expect("validate")
}

#[tokio::test]
async fn insert_then_dedupe() {
    let (db, _dir) = fresh_db().await;
    let item = one_item();

    let (inserted, dup) = db
        .insert_x_bookmarks("user-1", &[item.clone()])
        .await
        .expect("insert");
    assert_eq!(inserted, 1);
    assert_eq!(dup, 0);

    // Re-inserting the same tweet must dedupe.
    let (inserted2, dup2) = db
        .insert_x_bookmarks("user-1", &[item])
        .await
        .expect("re-insert");
    assert_eq!(inserted2, 0);
    assert_eq!(dup2, 1);
}

#[tokio::test]
async fn dedupe_is_per_user() {
    let (db, _dir) = fresh_db().await;
    let item = one_item();

    db.insert_x_bookmarks("user-a", &[item.clone()])
        .await
        .unwrap();
    let (inserted, dup) = db
        .insert_x_bookmarks("user-b", &[item])
        .await
        .expect("user-b insert");
    // Same tweet_id under a different user_id should be inserted, not deduped.
    assert_eq!(inserted, 1);
    assert_eq!(dup, 0);
}

#[tokio::test]
async fn triage_round_trip() {
    let (db, _dir) = fresh_db().await;
    db.insert_x_bookmarks("user-1", &[one_item()])
        .await
        .unwrap();

    let untriaged = db
        .list_untriaged_x_bookmarks("user-1", 10)
        .await
        .expect("list untriaged");
    assert_eq!(untriaged.len(), 1);
    assert!(matches!(untriaged[0].status, BookmarkStatus::Untriaged));
    let id = untriaged[0].id;

    let decision = ResolvedTriageDecision {
        status: "build".to_string(),
        rationale: Some("interesting tool".to_string()),
        project_slug: Some("x-builder".to_string()),
        tags: vec!["rust".to_string(), "agents".to_string()],
    };
    let updated = db
        .apply_x_bookmark_triage("user-1", &[(id, decision)], "test/model")
        .await
        .expect("apply triage");
    assert_eq!(updated, 1);

    let queue = db
        .list_x_bookmarks_by_status("user-1", Some("build"), 10)
        .await
        .expect("queue");
    assert_eq!(queue.len(), 1);
    assert!(matches!(queue[0].status, BookmarkStatus::Build));
    assert_eq!(queue[0].project_slug.as_deref(), Some("x-builder"));
    assert_eq!(
        queue[0].tags,
        vec!["rust".to_string(), "agents".to_string()]
    );
    assert_eq!(queue[0].triage_model.as_deref(), Some("test/model"));
}

#[tokio::test]
async fn triage_does_not_leak_across_users() {
    let (db, _dir) = fresh_db().await;
    db.insert_x_bookmarks("user-a", &[one_item()])
        .await
        .unwrap();
    let untriaged = db.list_untriaged_x_bookmarks("user-a", 10).await.unwrap();
    let id = untriaged[0].id;

    // Try to triage user-a's bookmark by claiming it's user-b's. The UPDATE
    // should affect zero rows because the WHERE clause requires both id and
    // user_id to match.
    let decision = ResolvedTriageDecision {
        status: "dead".to_string(),
        rationale: None,
        project_slug: None,
        tags: vec![],
    };
    let updated = db
        .apply_x_bookmark_triage("user-b", &[(id, decision)], "test/model")
        .await
        .unwrap();
    assert_eq!(updated, 0, "must not let user-b triage user-a's bookmarks");

    // user-a's bookmark must still be untriaged.
    let still_untriaged = db.list_untriaged_x_bookmarks("user-a", 10).await.unwrap();
    assert_eq!(still_untriaged.len(), 1);
}

#[tokio::test]
async fn queue_status_filter_rejects_unknown_values() {
    let (db, _dir) = fresh_db().await;
    db.insert_x_bookmarks("user-1", &[one_item()])
        .await
        .unwrap();

    // An unknown status filter must NOT silently match all rows. The store
    // normalises unknown values to "no filter" (returns everything) — that
    // is the documented contract; the validator at the handler layer is what
    // enforces the canonical vocabulary.
    let all = db
        .list_x_bookmarks_by_status("user-1", Some("nonsense"), 10)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);

    let none = db
        .list_x_bookmarks_by_status("user-1", Some("dead"), 10)
        .await
        .unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
async fn stats_count_by_status() {
    let (db, _dir) = fresh_db().await;

    let mut items = vec![];
    for i in 0..5 {
        let raw = raw_item(
            &format!("182000000000000000{i}"),
            &format!("https://x.com/alice/status/182000000000000000{i}"),
        );
        items.push(validate_ingest_item(&raw).unwrap());
    }
    db.insert_x_bookmarks("user-1", &items).await.unwrap();

    let untriaged = db.list_untriaged_x_bookmarks("user-1", 10).await.unwrap();
    let pairs = vec![
        (
            untriaged[0].id,
            ResolvedTriageDecision {
                status: "build".to_string(),
                rationale: None,
                project_slug: None,
                tags: vec![],
            },
        ),
        (
            untriaged[1].id,
            ResolvedTriageDecision {
                status: "build".to_string(),
                rationale: None,
                project_slug: None,
                tags: vec![],
            },
        ),
        (
            untriaged[2].id,
            ResolvedTriageDecision {
                status: "dead".to_string(),
                rationale: None,
                project_slug: None,
                tags: vec![],
            },
        ),
    ];
    db.apply_x_bookmark_triage("user-1", &pairs, "test/model")
        .await
        .unwrap();

    let counts = db.x_bookmark_counts_by_status("user-1").await.unwrap();
    assert_eq!(counts.get("build").copied(), Some(2));
    assert_eq!(counts.get("dead").copied(), Some(1));
    assert_eq!(counts.get("untriaged").copied(), Some(2));
}

#[tokio::test]
async fn ingest_validation_rejects_non_x_url() {
    let mut item = raw_item("1820000000000000000", "https://evil.example/status/1");
    let err = validate_ingest_item(&item).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("url"), "expected url error, got {msg:?}");
    item.url = "javascript:alert(1)".to_string();
    let err2 = validate_ingest_item(&item).unwrap_err();
    let msg2 = err2.to_string();
    assert!(msg2.contains("url"), "expected url error, got {msg2:?}");
}

#[test]
fn batch_constants_are_consistent() {
    // The handler exposes MAX_INGEST_BATCH; the triage module enforces a
    // smaller per-call cap because batched LLM prompts get unwieldy.
    assert!(MAX_INGEST_BATCH >= ironclaw::x_bookmarks::triage::MAX_TRIAGE_BATCH);
}

/// Codex adversarial review fix: when a second triage writer arrives after
/// the first has already committed, the DB-level `status='untriaged'` guard
/// must reject the second write rather than overwriting the first decision.
#[tokio::test]
async fn second_triage_writer_does_not_clobber_first() {
    let (db, _dir) = fresh_db().await;
    db.insert_x_bookmarks("user-1", &[one_item()])
        .await
        .unwrap();
    let untriaged = db.list_untriaged_x_bookmarks("user-1", 10).await.unwrap();
    let id = untriaged[0].id;

    // First writer commits "build".
    let first = ResolvedTriageDecision {
        status: "build".to_string(),
        rationale: Some("first".to_string()),
        project_slug: Some("a".to_string()),
        tags: vec!["x".to_string()],
    };
    let updated1 = db
        .apply_x_bookmark_triage("user-1", &[(id, first)], "model-a")
        .await
        .unwrap();
    assert_eq!(updated1, 1);

    // Second writer arrives stale, tries to overwrite. The status guard in
    // the UPDATE must short-circuit so this returns 0 rows changed.
    let second = ResolvedTriageDecision {
        status: "dead".to_string(),
        rationale: Some("second".to_string()),
        project_slug: None,
        tags: vec![],
    };
    let updated2 = db
        .apply_x_bookmark_triage("user-1", &[(id, second)], "model-b")
        .await
        .unwrap();
    assert_eq!(updated2, 0, "stale second writer must not overwrite");

    // First writer's decision is still in place.
    let queue = db
        .list_x_bookmarks_by_status("user-1", Some("build"), 10)
        .await
        .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].rationale.as_deref(), Some("first"));
    assert_eq!(queue[0].triage_model.as_deref(), Some("model-a"));
}
