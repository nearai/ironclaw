#![cfg(feature = "libsql")]
//! Integration tests for workspace versioning, metadata resolution, and patch features.
//!
//! Uses libSQL in-memory or file-backed backend. Gate: `cargo test --features libsql`.

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

// =========================================================================
// Metadata resolution
// =========================================================================

#[tokio::test]
async fn test_resolve_metadata_no_config() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    ws.write("notes.md", "Hello").await.expect("write notes");

    let meta = ws.resolve_metadata("notes.md").await;
    assert_eq!(meta.skip_indexing, None, "skip_indexing should be None by default");
    assert_eq!(meta.skip_versioning, None, "skip_versioning should be None by default");
    assert_eq!(meta.hygiene, None, "hygiene should be None by default");
}

#[tokio::test]
async fn test_resolve_metadata_inherits_from_config() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    // Create a .config in the projects/ directory
    let config_doc = ws.write("projects/.config", "").await.expect("write .config");
    ws.update_metadata(
        config_doc.id,
        &serde_json::json!({"skip_indexing": true}),
    )
    .await
    .expect("set metadata");

    // Write a deeply nested document
    ws.write("projects/alpha/notes.md", "Some notes")
        .await
        .expect("write notes");

    let meta = ws.resolve_metadata("projects/alpha/notes.md").await;
    assert_eq!(
        meta.skip_indexing,
        Some(true),
        "should inherit skip_indexing from projects/.config"
    );
}

#[tokio::test]
async fn test_resolve_metadata_document_overrides_config() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    // Config says skip_indexing=true
    let config_doc = ws.write("projects/.config", "").await.expect("write .config");
    ws.update_metadata(
        config_doc.id,
        &serde_json::json!({"skip_indexing": true}),
    )
    .await
    .expect("set config metadata");

    // Document overrides to skip_indexing=false
    let doc = ws
        .write("projects/alpha.md", "Alpha content")
        .await
        .expect("write alpha");
    ws.update_metadata(doc.id, &serde_json::json!({"skip_indexing": false}))
        .await
        .expect("set doc metadata");

    let meta = ws.resolve_metadata("projects/alpha.md").await;
    assert_eq!(
        meta.skip_indexing,
        Some(false),
        "document metadata should override .config (nearest wins)"
    );
}

#[tokio::test]
async fn test_resolve_metadata_nearest_ancestor_wins() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    // Root .config says skip_indexing=true
    let root_config = ws.write(".config", "").await.expect("write root .config");
    ws.update_metadata(
        root_config.id,
        &serde_json::json!({"skip_indexing": true}),
    )
    .await
    .expect("set root config metadata");

    // projects/.config says skip_indexing=false (overrides root)
    let proj_config = ws
        .write("projects/.config", "")
        .await
        .expect("write projects .config");
    ws.update_metadata(
        proj_config.id,
        &serde_json::json!({"skip_indexing": false}),
    )
    .await
    .expect("set projects config metadata");

    let meta = ws.resolve_metadata("projects/notes.md").await;
    assert_eq!(
        meta.skip_indexing,
        Some(false),
        "nearest ancestor .config (projects/) should win over root .config"
    );
}

// =========================================================================
// Versioning
// =========================================================================

#[tokio::test]
async fn test_write_creates_version() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    let doc = ws.write("file.md", "v1").await.expect("write v1");
    ws.write("file.md", "v2").await.expect("write v2");

    let versions = ws.list_versions(doc.id, 100).await.expect("list versions");
    assert_eq!(
        versions.len(),
        1,
        "should have 1 version (the pre-v2 content 'v1')"
    );

    let v1 = ws.get_version(doc.id, versions[0].version).await.expect("get v1");
    assert_eq!(v1.content, "v1", "version content should be the original 'v1'");
}

#[tokio::test]
async fn test_write_deduplicates_identical_content() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    let doc = ws.write("file.md", "same").await.expect("write first");
    ws.write("file.md", "same").await.expect("write second (identical)");

    let versions = ws.list_versions(doc.id, 100).await.expect("list versions");
    assert!(
        versions.len() <= 1,
        "identical writes should not create duplicate versions, got {}",
        versions.len()
    );
}

#[tokio::test]
async fn test_append_versions_pre_append_content() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    let doc = ws.write("log.md", "line1").await.expect("write line1");
    ws.append("log.md", "line2").await.expect("append line2");

    let versions = ws.list_versions(doc.id, 100).await.expect("list versions");
    assert_eq!(versions.len(), 1, "append should create 1 version of pre-append content");

    let v1 = ws.get_version(doc.id, versions[0].version).await.expect("get v1");
    assert_eq!(v1.content, "line1", "version should capture pre-append content");
}

// =========================================================================
// Patch
// =========================================================================

#[tokio::test]
async fn test_patch_single_replacement() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    ws.write("doc.md", "hello world hello")
        .await
        .expect("write");

    let result = ws
        .patch("doc.md", "hello", "hi", false)
        .await
        .expect("patch");
    assert_eq!(result.document.content, "hi world hello");
    assert_eq!(result.replacements, 1);
}

#[tokio::test]
async fn test_patch_replace_all() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    ws.write("doc.md", "hello world hello")
        .await
        .expect("write");

    let result = ws
        .patch("doc.md", "hello", "hi", true)
        .await
        .expect("patch");
    assert_eq!(result.document.content, "hi world hi");
    assert_eq!(result.replacements, 2);
}

#[tokio::test]
async fn test_patch_not_found_error() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    ws.write("doc.md", "hello").await.expect("write");

    let err = ws
        .patch("doc.md", "xyz", "abc", false)
        .await
        .expect_err("patch should fail");

    let err_str = format!("{err}");
    assert!(
        err_str.contains("not found") || err_str.contains("PatchFailed") || err_str.contains("patch"),
        "error should indicate patch failure: {err_str}"
    );
}

#[tokio::test]
async fn test_patch_creates_version() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    let doc = ws.write("doc.md", "original").await.expect("write");
    ws.patch("doc.md", "original", "modified", false)
        .await
        .expect("patch");

    let versions = ws.list_versions(doc.id, 100).await.expect("list versions");
    assert!(
        !versions.is_empty(),
        "patch should create a version of the pre-patch content"
    );

    let v1 = ws.get_version(doc.id, versions[0].version).await.expect("get v1");
    assert_eq!(v1.content, "original", "version should contain pre-patch content");
}

// =========================================================================
// Metadata-driven behavior: skip_indexing and skip_versioning
// =========================================================================

#[tokio::test]
async fn test_skip_indexing_via_config() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db.clone());

    // Create frontend/.config with skip_indexing
    let config_doc = ws
        .write("frontend/.config", "")
        .await
        .expect("write .config");
    ws.update_metadata(
        config_doc.id,
        &serde_json::json!({"skip_indexing": true}),
    )
    .await
    .expect("set config metadata");

    // Write a document under frontend/
    ws.write("frontend/widget.js", "function render() { return <div>Hello</div>; }")
        .await
        .expect("write widget");

    // Check that no chunks exist for this user scope (skip_indexing should
    // have prevented chunk creation and deleted any pre-existing chunks).
    let chunks = db
        .get_chunks_without_embeddings("default", None, 1000)
        .await
        .expect("get chunks");

    // Filter chunks belonging to the frontend/widget.js document
    let widget_doc = ws.read("frontend/widget.js").await.expect("read widget");
    let widget_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.document_id == widget_doc.id)
        .collect();
    assert!(
        widget_chunks.is_empty(),
        "skip_indexing should prevent chunk creation, found {} chunks",
        widget_chunks.len()
    );
}

#[tokio::test]
async fn test_skip_versioning_via_config() {
    let (db, _dir) = setup().await;
    let ws = Workspace::new_with_db("default", db);

    // Create daily/.config with skip_versioning
    let config_doc = ws.write("daily/.config", "").await.expect("write .config");
    ws.update_metadata(
        config_doc.id,
        &serde_json::json!({"skip_versioning": true}),
    )
    .await
    .expect("set config metadata");

    // Write twice to daily/log.md
    let doc = ws
        .write("daily/log.md", "first entry")
        .await
        .expect("write first");
    ws.write("daily/log.md", "second entry")
        .await
        .expect("write second");

    let versions = ws.list_versions(doc.id, 100).await.expect("list versions");
    assert_eq!(
        versions.len(),
        0,
        "skip_versioning should prevent version creation, got {} versions",
        versions.len()
    );
}
