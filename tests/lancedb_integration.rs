//! Integration tests for LanceDB vector store with Database wrapper.
//!
//! Requires: cargo test --features "libsql,lancedb"
//!
//! Verifies DbWithLanceVectorStore: document + chunk insert, hybrid search
//! (FTS from libSQL, vector from LanceDB), delete_chunks sync.

#![cfg(all(feature = "libsql", feature = "lancedb"))]

use std::sync::Arc;

use ironclaw::db::lancedb_wrapper::DbWithLanceVectorStore;
use ironclaw::db::Database;
use ironclaw::db::libsql_backend::LibSqlBackend;
use ironclaw::workspace::{LanceDbVectorStore, SearchConfig};
use tempfile::TempDir;
use uuid::Uuid;

const EMBEDDING_DIM: usize = 1536;

fn make_embedding(seed: f32) -> Vec<f32> {
    (0..EMBEDDING_DIM)
        .map(|i| (seed * (i as f32 + 1.0)).sin())
        .collect()
}

async fn setup_wrapped_db() -> (Arc<dyn Database>, TempDir) {
    let libsql = LibSqlBackend::new_memory().await.unwrap();
    libsql.run_migrations().await.unwrap();

    let lancedb_dir = TempDir::new().unwrap();
    let store = LanceDbVectorStore::new(lancedb_dir.path())
        .await
        .unwrap();

    let db = Arc::new(DbWithLanceVectorStore::new(
        Arc::new(libsql) as Arc<dyn Database>,
        Arc::new(store),
    )) as Arc<dyn Database>;

    (db, lancedb_dir)
}

#[tokio::test]
async fn test_wrapper_hybrid_search_combines_fts_and_vector() {
    let (db, _) = setup_wrapped_db().await;

    let user_id = "test_user";
    let agent_id: Option<Uuid> = None;

    // Create document
    let doc = db
        .get_or_create_document_by_path(user_id, agent_id, "context/rust.md")
        .await
        .unwrap();

    // Write content for FTS
    db.update_document(doc.id, "Rust is a systems programming language focused on safety and performance.")
        .await
        .unwrap();

    // Chunk and insert with embedding (triggers sync to LanceDB)
    let content = "Rust is a systems programming language focused on safety.";
    let embedding = make_embedding(1.0);

    let chunk_id = db
        .insert_chunk(doc.id, 0, content, Some(&embedding))
        .await
        .unwrap();

    // Hybrid search: FTS for "Rust" + vector for semantic
    let config = SearchConfig::default().with_limit(5);
    let results = db
        .hybrid_search(
            user_id,
            agent_id,
            "Rust",
            Some(&embedding),
            &config,
        )
        .await
        .unwrap();

    assert!(!results.is_empty(), "hybrid search should return results");
    assert_eq!(results[0].chunk_id, chunk_id);
    assert!(results[0].content.contains("Rust"));
}

#[tokio::test]
async fn test_wrapper_delete_chunks_removes_from_both() {
    let (db, _) = setup_wrapped_db().await;

    let user_id = "test_user";
    let agent_id: Option<Uuid> = None;

    let doc = db
        .get_or_create_document_by_path(user_id, agent_id, "notes/deleted.md")
        .await
        .unwrap();

    db.update_document(doc.id, "Content to be deleted.").await.unwrap();

    let embedding = make_embedding(2.0);
    db.insert_chunk(doc.id, 0, "Content to be deleted.", Some(&embedding))
        .await
        .unwrap();

    let before = db
        .hybrid_search(user_id, agent_id, "deleted", Some(&embedding), &SearchConfig::default())
        .await
        .unwrap();
    assert_eq!(before.len(), 1);

    db.delete_chunks(doc.id).await.unwrap();

    let after = db
        .hybrid_search(user_id, agent_id, "deleted", Some(&embedding), &SearchConfig::default())
        .await
        .unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn test_wrapper_insert_chunk_syncs_to_lancedb() {
    let (db, _) = setup_wrapped_db().await;

    let user_id = "sync_user";
    let agent_id: Option<Uuid> = None;

    let doc = db
        .get_or_create_document_by_path(user_id, agent_id, "sync/test.md")
        .await
        .unwrap();

    let content = "Semantic content for vector search";
    let embedding = make_embedding(3.0);

    let chunk_id = db
        .insert_chunk(doc.id, 0, content, Some(&embedding))
        .await
        .unwrap();

    // Vector-only search (no FTS query match) - should still find via LanceDB
    let config = SearchConfig::default().vector_only().with_limit(5);
    let results = db
        .hybrid_search(
            user_id,
            agent_id,
            "nonexistent_fts_term",
            Some(&embedding),
            &config,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].chunk_id, chunk_id);
    assert_eq!(results[0].content, content);
}
