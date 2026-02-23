#![cfg(feature = "libsql")]
//! Integration tests for multi-scope workspace reads using file-backed libSQL.
//!
//! Guards the PR2 contract: workspaces can read from multiple user scopes
//! while writes remain isolated to the primary scope.

use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::workspace::Workspace;

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

#[tokio::test]
async fn read_across_scopes() {
    let (db, _dir) = setup().await;

    // Write docs as the "shared" user
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write("docs/team-standup.md", "Team standup notes from Monday")
        .await
        .expect("shared write failed");

    // Alice's workspace with "shared" as an additional read scope
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    // Alice can read shared docs
    let doc = ws_alice
        .read("docs/team-standup.md")
        .await
        .expect("cross-scope read failed");
    assert_eq!(doc.content, "Team standup notes from Monday");
}

#[tokio::test]
async fn write_stays_in_primary_scope() {
    let (db, _dir) = setup().await;

    // Alice has "shared" as a read scope
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    // Alice writes a personal note
    ws_alice
        .write("notes/personal.md", "Alice's private note")
        .await
        .expect("alice write failed");

    // The "shared" workspace should NOT see Alice's note
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    let result = ws_shared.read("notes/personal.md").await;
    assert!(result.is_err(), "Shared scope should not see Alice's note");
}

#[tokio::test]
async fn list_paths_merges_across_scopes() {
    let (db, _dir) = setup().await;

    // Write as alice
    let ws_alice_plain = Workspace::new_with_db("alice", Arc::clone(&db));
    ws_alice_plain
        .write("notes/personal.md", "My notes")
        .await
        .expect("alice write failed");

    // Write as shared
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write("docs/shared-doc.md", "Shared document")
        .await
        .expect("shared write failed");

    // Alice with multi-scope should see both
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    let all_paths = ws_alice.list_all().await.expect("list_all failed");
    assert!(
        all_paths.contains(&"notes/personal.md".to_string()),
        "Should contain alice's note: {:?}",
        all_paths
    );
    assert!(
        all_paths.contains(&"docs/shared-doc.md".to_string()),
        "Should contain shared doc: {:?}",
        all_paths
    );
}

#[tokio::test]
async fn list_directory_merges_across_scopes() {
    let (db, _dir) = setup().await;

    // Alice writes to docs/
    let ws_alice_plain = Workspace::new_with_db("alice", Arc::clone(&db));
    ws_alice_plain
        .write("docs/alice-doc.md", "Alice's doc")
        .await
        .expect("alice write failed");

    // Shared writes to docs/
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write("docs/shared-doc.md", "Shared doc")
        .await
        .expect("shared write failed");

    // Alice with multi-scope lists docs/
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    let entries = ws_alice.list("docs").await.expect("list failed");
    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"docs/alice-doc.md"),
        "Should contain alice's doc: {:?}",
        paths
    );
    assert!(
        paths.contains(&"docs/shared-doc.md"),
        "Should contain shared doc: {:?}",
        paths
    );
}

#[tokio::test]
async fn search_spans_scopes() {
    let (db, _dir) = setup().await;

    // Write searchable content in shared scope
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write(
            "docs/architecture.md",
            "The microservice architecture uses gRPC for inter-service communication",
        )
        .await
        .expect("shared write failed");

    // Write searchable content in alice scope
    let ws_alice_plain = Workspace::new_with_db("alice", Arc::clone(&db));
    ws_alice_plain
        .write("notes/ideas.md", "Consider switching to GraphQL federation")
        .await
        .expect("alice write failed");

    // Alice with multi-scope searches
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    // Search for content in the shared scope
    let results = ws_alice
        .search("microservice architecture gRPC", 10)
        .await
        .expect("search failed");
    assert!(
        !results.is_empty(),
        "Should find results from shared scope"
    );
}

#[tokio::test]
async fn read_priority_primary_first() {
    let (db, _dir) = setup().await;

    // Write same path in both scopes
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write("config/settings.md", "Shared settings v1")
        .await
        .expect("shared write failed");

    let ws_alice_plain = Workspace::new_with_db("alice", Arc::clone(&db));
    ws_alice_plain
        .write("config/settings.md", "Alice's settings override")
        .await
        .expect("alice write failed");

    // Alice with multi-scope should get her own version (primary scope wins)
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);

    let doc = ws_alice
        .read("config/settings.md")
        .await
        .expect("read failed");
    assert_eq!(
        doc.content, "Alice's settings override",
        "Primary scope should take priority"
    );
}

#[tokio::test]
async fn exists_spans_scopes() {
    let (db, _dir) = setup().await;

    // Write a doc as "shared"
    let ws_shared = Workspace::new_with_db("shared", Arc::clone(&db));
    ws_shared
        .write("docs/shared-only.md", "Shared content")
        .await
        .expect("shared write failed");

    // Alice without multi-scope should NOT see it
    let ws_alice_plain = Workspace::new_with_db("alice", Arc::clone(&db));
    assert!(
        !ws_alice_plain
            .exists("docs/shared-only.md")
            .await
            .expect("exists failed"),
        "Alice without multi-scope should not see shared doc"
    );

    // Alice with multi-scope should see it
    let ws_alice = Workspace::new_with_db("alice", Arc::clone(&db))
        .with_additional_read_scopes(vec!["shared".to_string()]);
    assert!(
        ws_alice
            .exists("docs/shared-only.md")
            .await
            .expect("exists failed"),
        "Alice with multi-scope should see shared doc"
    );
}
