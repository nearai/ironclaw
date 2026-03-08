//! Integration tests for LanceDB vector store with Workspace composition.
//!
//! Requires: cargo test --features "libsql,lancedb"
//!
//! Verifies that Workspace correctly composes FTS from libSQL with vector
//! search from LanceDB via the VectorStore trait.

#![cfg(all(feature = "libsql", feature = "lancedb"))]

use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql_backend::LibSqlBackend;
use ironclaw::workspace::{LanceDbVectorStore, SearchConfig, Workspace};
use tempfile::TempDir;

const EMBEDDING_DIM: usize = 1536;

fn make_embedding(seed: f32) -> Vec<f32> {
    (0..EMBEDDING_DIM)
        .map(|i| (seed * (i as f32 + 1.0)).sin())
        .collect()
}

/// Mock embedding provider that returns deterministic embeddings.
struct FixedEmbeddings {
    embedding: Vec<f32>,
}

#[async_trait::async_trait]
impl ironclaw::workspace::EmbeddingProvider for FixedEmbeddings {
    fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }

    fn model_name(&self) -> &str {
        "fixed-test"
    }

    fn max_input_length(&self) -> usize {
        8192
    }

    async fn embed(
        &self,
        _text: &str,
    ) -> Result<Vec<f32>, ironclaw::workspace::embeddings::EmbeddingError> {
        Ok(self.embedding.clone())
    }
}

async fn setup_workspace() -> (Workspace, TempDir, TempDir) {
    // Use a temp file (not :memory:) because libSQL in-memory DBs are connection-local
    let db_dir = TempDir::new().unwrap();
    let db_path = db_dir.path().join("test.db");
    let libsql = LibSqlBackend::new_local(&db_path).await.unwrap();
    libsql.run_migrations().await.unwrap();

    let lancedb_dir = TempDir::new().unwrap();
    let store = LanceDbVectorStore::new(lancedb_dir.path()).await.unwrap();

    let embedding = make_embedding(1.0);
    let ws = Workspace::new_with_db("test_user", Arc::new(libsql) as Arc<dyn Database>)
        .with_vector_store(Arc::new(store))
        .with_embeddings(Arc::new(FixedEmbeddings { embedding }));

    (ws, lancedb_dir, db_dir)
}

#[tokio::test]
async fn test_workspace_hybrid_search_with_lancedb() {
    let (ws, _keep_lance, _keep_db) = setup_workspace().await;

    // Write a document — this triggers chunking + embedding + LanceDB sync
    ws.write(
        "context/rust.md",
        "Rust is a systems programming language focused on safety.",
    )
    .await
    .unwrap();

    // Hybrid search: FTS for "Rust" + vector from LanceDB
    let results = ws.search("Rust", 5).await.unwrap();

    assert!(!results.is_empty(), "hybrid search should return results");
    assert!(results[0].content.contains("Rust"));
}

#[tokio::test]
async fn test_workspace_delete_removes_from_lancedb() {
    let (ws, _keep_lance, _keep_db) = setup_workspace().await;

    ws.write("notes/deleted.md", "Content to be deleted.")
        .await
        .unwrap();

    let before = ws.search("deleted", 5).await.unwrap();
    assert_eq!(before.len(), 1);

    ws.delete("notes/deleted.md").await.unwrap();

    let after = ws.search("deleted", 5).await.unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn test_workspace_vector_only_search_uses_lancedb() {
    let (ws, _keep_lance, _keep_db) = setup_workspace().await;

    ws.write("sync/test.md", "Semantic content for vector search")
        .await
        .unwrap();

    // Vector-only search should find via LanceDB even with non-matching FTS query
    let config = SearchConfig::default().vector_only().with_limit(5);
    let results = ws
        .search_with_config("nonexistent_fts_term", config)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("Semantic content"));
}
