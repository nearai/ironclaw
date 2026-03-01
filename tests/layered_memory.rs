#![cfg(feature = "libsql")]
//! Integration tests for layered memory using file-backed libSQL.

use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::workspace::Workspace;
use ironclaw::workspace::layer::{LayerSensitivity, MemoryLayer};

async fn setup() -> (Arc<dyn Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path).await.expect("create db");
    backend.run_migrations().await.expect("run migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

fn test_layers() -> Vec<MemoryLayer> {
    vec![
        MemoryLayer {
            name: "private".into(),
            scope: "alice".into(),
            writable: true,
            sensitivity: LayerSensitivity::Private,
        },
        MemoryLayer {
            name: "shared".into(),
            scope: "shared".into(),
            writable: true,
            sensitivity: LayerSensitivity::Shared,
        },
        MemoryLayer {
            name: "reports".into(),
            scope: "reports".into(),
            writable: false,
            sensitivity: LayerSensitivity::Shared,
        },
    ]
}

#[tokio::test]
async fn write_to_private_layer() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    let result = ws
        .write_to_layer("private", "notes/test.md", "Private note")
        .await
        .expect("write should succeed");
    assert_eq!(result.document.content, "Private note");
    assert!(!result.redirected);
    assert_eq!(result.actual_layer, "private");
}

#[tokio::test]
async fn write_to_shared_layer() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    let result = ws
        .write_to_layer("shared", "plans/dinner.md", "Dinner Saturday at 6")
        .await
        .expect("write should succeed");
    assert_eq!(result.document.content, "Dinner Saturday at 6");
    assert!(!result.redirected);
    assert_eq!(result.actual_layer, "shared");
}

#[tokio::test]
async fn write_to_read_only_layer_fails() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    let result = ws
        .write_to_layer("reports", "notes/budget.md", "Some budget note")
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn write_to_unknown_layer_fails() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    let result = ws
        .write_to_layer("nonexistent", "notes/test.md", "content")
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn sensitive_content_redirected_to_private() {
    let (db, _dir) = setup().await;
    let db_clone = db.clone();
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    // Write sensitive content to shared layer -- should be redirected
    let result = ws
        .write_to_layer(
            "shared",
            "notes/health.md",
            "Started new medication for anxiety",
        )
        .await
        .expect("write should succeed (redirected)");

    // WriteResult should indicate redirect to private layer
    assert!(result.redirected, "Should be redirected");
    assert_eq!(result.actual_layer, "private");
    assert_eq!(result.document.content, "Started new medication for anxiety");

    // Content should be in the private scope (alice), not the shared scope
    let private_doc = ws.read("notes/health.md").await;
    assert!(
        private_doc.is_ok(),
        "Should find content in private scope (alice)"
    );
    assert_eq!(
        private_doc.unwrap().content,
        "Started new medication for anxiety"
    );

    // Verify content is NOT in the shared scope (same DB, different user_id)
    let ws_shared = Workspace::new_with_db("shared", db_clone);
    let shared_doc = ws_shared.read("notes/health.md").await;
    assert!(
        shared_doc.is_err(),
        "Should NOT find content in shared scope"
    );
}

#[tokio::test]
async fn default_write_still_works() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    // Regular write (no layer) should still work
    let doc = ws
        .write("notes/test.md", "Regular note")
        .await
        .expect("write should succeed");
    assert_eq!(doc.content, "Regular note");
}

#[tokio::test]
async fn append_to_layer_works() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    // Write initial content to a layer
    ws.write_to_layer("private", "notes/log.md", "Entry one")
        .await
        .expect("initial write should succeed");

    // Append to the same layer path
    let result = ws
        .append_to_layer("private", "notes/log.md", "Entry two")
        .await
        .expect("append should succeed");

    // Content should be concatenated with double newline
    assert!(
        result.document.content.contains("Entry one"),
        "Should contain first entry"
    );
    assert!(
        result.document.content.contains("Entry two"),
        "Should contain second entry"
    );
}

#[tokio::test]
async fn sensitive_content_fails_without_private_layer() {
    let (db, _dir) = setup().await;

    // Workspace with only shared layers (no private layer for redirect)
    let shared_only_layers = vec![
        MemoryLayer {
            name: "shared".into(),
            scope: "shared".into(),
            writable: true,
            sensitivity: LayerSensitivity::Shared,
        },
    ];
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(shared_only_layers);

    // Writing sensitive content should fail (no private layer to redirect to)
    let result = ws
        .write_to_layer(
            "shared",
            "notes/health.md",
            "Started new medication for anxiety",
        )
        .await;
    assert!(
        result.is_err(),
        "Should fail when no private layer available for redirect"
    );
}

#[tokio::test]
async fn append_sensitive_to_shared_redirects() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    // Append sensitive content to shared layer -- should be redirected
    let result = ws
        .append_to_layer(
            "shared",
            "notes/health.md",
            "My doctor prescribed new medication",
        )
        .await
        .expect("append should succeed (redirected)");

    assert!(result.redirected, "Should be redirected");
    assert_eq!(result.actual_layer, "private");
    assert!(result.document.content.contains("medication"));
}

#[tokio::test]
async fn search_finds_private_layer_content() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("alice", db).with_memory_layers(test_layers());

    // Write to the private layer (scope = "alice" = user_id)
    ws.write_to_layer(
        "private",
        "notes/private.md",
        "My private thought about waffles",
    )
    .await
    .unwrap();

    // Search should find content in the primary scope
    let results = ws.search("waffles", 10).await.unwrap();
    assert!(
        !results.is_empty(),
        "Should find results in the private layer"
    );
}
