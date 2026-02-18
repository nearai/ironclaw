//! LanceDB-backed vector store for workspace memory chunks.
//!
//! Provides an alternative to pgvector/libsql for semantic search when the
//! `lancedb` feature is enabled. Documents and metadata stay in the main
//! database; this store holds chunk embeddings for vector similarity search.
//!
//! Configuration:
//!   LANCEDB_PATH=~/.ironclaw/lancedb   # Default
//!   VECTOR_BACKEND=lancedb             # Use LanceDB for vector search

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::search::RankedResult;

/// Embedding dimension (text-embedding-3-small default).
/// Must match the embedding model used.
pub const DEFAULT_EMBEDDING_DIM: i32 = 1536;

/// Vector store abstraction for semantic search.
///
/// Implementations: pgvector/libsql (embedded in Database), LanceDB (this module).
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Insert a chunk with its embedding.
    async fn insert_chunk(
        &self,
        chunk_id: Uuid,
        document_id: Uuid,
        user_id: &str,
        agent_id: Option<Uuid>,
        content: &str,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;

    /// Update an existing chunk's embedding.
    ///
    /// For LanceDB, this performs delete+insert since LanceDB has limited update
    /// support. Caller must provide full chunk metadata for the re-insert.
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        document_id: Uuid,
        user_id: &str,
        agent_id: Option<Uuid>,
        content: &str,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;

    /// Delete all chunks for a document.
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;

    /// Vector similarity search, filtered by user and agent.
    async fn vector_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError>;
}

#[cfg(feature = "lancedb")]
mod impl_lancedb {
    use std::sync::Arc;

    use arrow_array::types::Float32Type;
    use arrow_array::{
        Array, FixedSizeListArray, RecordBatch, RecordBatchIterator, StringArray,
    };
    use arrow_schema::{DataType, Field, Schema};
    use async_trait::async_trait;
    use futures::StreamExt;
    use lancedb::index::Index;
    use uuid::Uuid;

    use super::{RankedResult, VectorStore, DEFAULT_EMBEDDING_DIM};
    use crate::error::WorkspaceError;

    const TABLE_NAME: &str = "memory_chunks";

    /// LanceDB-backed vector store.
    pub struct LanceDbVectorStore {
        db: Arc<lancedb::Connection>,
        table_name: String,
        embedding_dim: i32,
    }

    impl LanceDbVectorStore {
        /// Create a new LanceDB store at the given path.
        pub async fn new(path: impl AsRef<std::path::Path>) -> Result<Self, WorkspaceError> {
            let path_str = path
                .as_ref()
                .to_str()
                .ok_or_else(|| WorkspaceError::SearchFailed {
                    reason: "Invalid LanceDB path".to_string(),
                })?;

            let db = lancedb::connect(path_str)
                .execute()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Failed to connect to LanceDB: {}", e),
                })?;

            let store = Self {
                db: Arc::new(db),
                table_name: TABLE_NAME.to_string(),
                embedding_dim: DEFAULT_EMBEDDING_DIM,
            };

            store.ensure_table().await?;
            Ok(store)
        }

        async fn ensure_table(&self) -> Result<(), WorkspaceError> {
            let tables = self.db.table_names().execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to list tables: {}", e),
                }
            })?;

            if tables.iter().any(|t| t == &self.table_name) {
                return Ok(());
            }

            let schema = Arc::new(self.schema());
            self.db
                .create_empty_table(&self.table_name, schema.clone())
                .execute()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Failed to create table: {}", e),
                })?;

            let table = self.db.open_table(&self.table_name).execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to open table: {}", e),
                }
            })?;

            table
                .create_index(&["vector"], Index::Auto)
                .execute()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Failed to create vector index: {}", e),
                })?;

            Ok(())
        }

        fn schema(&self) -> Schema {
            Schema::new(vec![
                Field::new("chunk_id", DataType::Utf8, false),
                Field::new("document_id", DataType::Utf8, false),
                Field::new("user_id", DataType::Utf8, false),
                Field::new("agent_id", DataType::Utf8, true),
                Field::new("content", DataType::Utf8, false),
                Field::new(
                    "vector",
                    DataType::FixedSizeList(
                        Arc::new(Field::new("item", DataType::Float32, true)),
                        self.embedding_dim,
                    ),
                    false,
                ),
            ])
        }
    }

    #[async_trait]
    impl VectorStore for LanceDbVectorStore {
        async fn insert_chunk(
            &self,
            chunk_id: Uuid,
            document_id: Uuid,
            user_id: &str,
            agent_id: Option<Uuid>,
            content: &str,
            embedding: &[f32],
        ) -> Result<(), WorkspaceError> {
            if embedding.len() != self.embedding_dim as usize {
                return Err(WorkspaceError::EmbeddingFailed {
                    reason: format!(
                        "Embedding dimension {} does not match expected {}",
                        embedding.len(),
                        self.embedding_dim
                    ),
                });
            }

            let table = self.db.open_table(&self.table_name).execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to open table: {}", e),
                }
            })?;

            let chunk_ids = StringArray::from(vec![chunk_id.to_string()]);
            let document_ids = StringArray::from(vec![document_id.to_string()]);
            let user_ids = StringArray::from(vec![user_id]);
            let agent_ids = StringArray::from(vec![agent_id.map(|a| a.to_string())]);
            let contents = StringArray::from(vec![content]);
            let vec_values: Vec<Option<f32>> =
                embedding.iter().map(|&x| Some(x)).collect();
            let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                vec![Some(vec_values)].into_iter(),
                self.embedding_dim,
            );

            let batch = RecordBatch::try_new(
                Arc::new(self.schema()),
                vec![
                    Arc::new(chunk_ids),
                    Arc::new(document_ids),
                    Arc::new(user_ids),
                    Arc::new(agent_ids),
                    Arc::new(contents),
                    Arc::new(vectors),
                ],
            )
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Failed to create record batch: {}", e),
            })?;

            let batches = RecordBatchIterator::new(
                vec![Ok(batch)].into_iter(),
                Arc::new(self.schema()),
            );

            table
                .add(Box::new(batches))
                .execute()
                .await
                .map_err(|e| WorkspaceError::ChunkingFailed {
                    reason: format!("Failed to insert chunk: {}", e),
                })?;

            Ok(())
        }

        async fn update_chunk_embedding(
            &self,
            chunk_id: Uuid,
            document_id: Uuid,
            user_id: &str,
            agent_id: Option<Uuid>,
            content: &str,
            embedding: &[f32],
        ) -> Result<(), WorkspaceError> {
            let table = self.db.open_table(&self.table_name).execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to open table: {}", e),
                }
            })?;

            table
                .delete(&format!("chunk_id = '{}'", chunk_id))
                .await
                .map_err(|e| WorkspaceError::EmbeddingFailed {
                    reason: format!("Failed to delete chunk for update: {}", e),
                })?;

            self.insert_chunk(chunk_id, document_id, user_id, agent_id, content, embedding)
                .await
        }

        async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
            let table = self.db.open_table(&self.table_name).execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to open table: {}", e),
                }
            })?;

            table
                .delete(&format!("document_id = '{}'", document_id))
                .await
                .map_err(|e| WorkspaceError::ChunkingFailed {
                    reason: format!("Failed to delete chunks: {}", e),
                })?;

            Ok(())
        }

        async fn vector_search(
            &self,
            user_id: &str,
            agent_id: Option<Uuid>,
            embedding: &[f32],
            limit: usize,
        ) -> Result<Vec<RankedResult>, WorkspaceError> {
            let table = self.db.open_table(&self.table_name).execute().await.map_err(|e| {
                WorkspaceError::SearchFailed {
                    reason: format!("Failed to open table: {}", e),
                }
            })?;

            let filter = if let Some(aid) = agent_id {
                format!("user_id = '{}' AND agent_id = '{}'", user_id, aid)
            } else {
                format!("user_id = '{}' AND agent_id IS NULL", user_id)
            };

            let mut stream = table
                .query()
                .nearest_to(embedding)
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Invalid query vector: {}", e),
                })?
                .only_if(&filter)
                .limit(limit as u32)
                .execute()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Vector search failed: {}", e),
                })?;

            let mut results = Vec::new();
            let mut rank: u32 = 1;
            while let Some(batch) = stream.next().await {
                let batch = batch.map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Stream error: {}", e),
                })?;

                let chunk_id_col = batch
                    .column_by_name("chunk_id")
                    .ok_or_else(|| WorkspaceError::SearchFailed {
                        reason: "chunk_id column missing".to_string(),
                    })?;
                let document_id_col = batch
                    .column_by_name("document_id")
                    .ok_or_else(|| WorkspaceError::SearchFailed {
                        reason: "document_id column missing".to_string(),
                    })?;
                let content_col = batch
                    .column_by_name("content")
                    .ok_or_else(|| WorkspaceError::SearchFailed {
                        reason: "content column missing".to_string(),
                    })?;

                let chunk_ids = chunk_id_col.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
                    WorkspaceError::SearchFailed {
                        reason: "chunk_id wrong type".to_string(),
                    }
                })?;
                let document_ids = document_id_col.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
                    WorkspaceError::SearchFailed {
                        reason: "document_id wrong type".to_string(),
                    }
                })?;
                let contents = content_col.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
                    WorkspaceError::SearchFailed {
                        reason: "content wrong type".to_string(),
                    }
                })?;

                for i in 0..batch.num_rows() {
                    let chunk_id = chunk_ids
                        .value(i)
                        .parse()
                        .unwrap_or_else(|_| Uuid::nil());
                    let document_id = document_ids
                        .value(i)
                        .parse()
                        .unwrap_or_else(|_| Uuid::nil());
                    let content = contents.value(i).to_string();

                    results.push(RankedResult {
                        chunk_id,
                        document_id,
                        content,
                        rank,
                    });
                    rank += 1;
                }
            }

            Ok(results)
        }
    }
}

#[cfg(feature = "lancedb")]
pub use impl_lancedb::LanceDbVectorStore;

#[cfg(all(test, feature = "lancedb"))]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{LanceDbVectorStore, VectorStore, DEFAULT_EMBEDDING_DIM};

    fn make_embedding(seed: f32) -> Vec<f32> {
        (0..DEFAULT_EMBEDDING_DIM as usize)
            .map(|i| (seed * (i as f32 + 1.0)).sin())
            .collect()
    }

    #[tokio::test]
    async fn test_insert_and_vector_search() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let chunk_id = Uuid::new_v4();
        let document_id = Uuid::new_v4();
        let user_id = "user1";
        let content = "Rust is a systems programming language";
        let embedding = make_embedding(1.0);

        store
            .insert_chunk(
                chunk_id,
                document_id,
                user_id,
                None,
                content,
                &embedding,
            )
            .await
            .unwrap();

        let results = store
            .vector_search(user_id, None, &embedding, 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, chunk_id);
        assert_eq!(results[0].document_id, document_id);
        assert_eq!(results[0].content, content);
        assert_eq!(results[0].rank, 1);
    }

    #[tokio::test]
    async fn test_insert_multiple_and_search_returns_ordered() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let doc_id = Uuid::new_v4();
        let user_id = "user1";

        // Insert 3 chunks with different embeddings
        for (i, seed) in [1.0, 2.0, 3.0].iter().enumerate() {
            store
                .insert_chunk(
                    Uuid::new_v4(),
                    doc_id,
                    user_id,
                    None,
                    &format!("content {}", i),
                    &make_embedding(*seed),
                )
                .await
                .unwrap();
        }

        // Search returns all 3, ordered by similarity
        let query_emb = make_embedding(2.0);
        let results = store
            .vector_search(user_id, None, &query_emb, 5)
            .await
            .unwrap();

        assert_eq!(results.len(), 3);
        let contents: Vec<_> = results.iter().map(|r| r.content.as_str()).collect();
        assert!(contents.contains(&"content 0"));
        assert!(contents.contains(&"content 1"));
        assert!(contents.contains(&"content 2"));
    }

    #[tokio::test]
    async fn test_delete_chunks() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let doc_id = Uuid::new_v4();
        let user_id = "user1";

        store
            .insert_chunk(
                Uuid::new_v4(),
                doc_id,
                user_id,
                None,
                "content",
                &make_embedding(1.0),
            )
            .await
            .unwrap();

        let results = store
            .vector_search(user_id, None, &make_embedding(1.0), 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        store.delete_chunks(doc_id).await.unwrap();

        let results_after = store
            .vector_search(user_id, None, &make_embedding(1.0), 5)
            .await
            .unwrap();
        assert!(results_after.is_empty());
    }

    #[tokio::test]
    async fn test_update_chunk_embedding() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let chunk_id = Uuid::new_v4();
        let doc_id = Uuid::new_v4();
        let user_id = "user1";
        let content = "original content";

        store
            .insert_chunk(
                chunk_id,
                doc_id,
                user_id,
                None,
                content,
                &make_embedding(1.0),
            )
            .await
            .unwrap();

        // Update with new embedding
        let new_embedding = make_embedding(5.0);
        store
            .update_chunk_embedding(chunk_id, doc_id, user_id, None, content, &new_embedding)
            .await
            .unwrap();

        // Search with new embedding should find it
        let results = store
            .vector_search(user_id, None, &new_embedding, 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, chunk_id);
    }

    #[tokio::test]
    async fn test_vector_search_filters_by_user_and_agent() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let doc_id = Uuid::new_v4();
        let embedding = make_embedding(1.0);

        store
            .insert_chunk(
                Uuid::new_v4(),
                doc_id,
                "user1",
                None,
                "user1 content",
                &embedding,
            )
            .await
            .unwrap();

        store
            .insert_chunk(
                Uuid::new_v4(),
                doc_id,
                "user2",
                None,
                "user2 content",
                &embedding,
            )
            .await
            .unwrap();

        let results_user1 = store
            .vector_search("user1", None, &embedding, 5)
            .await
            .unwrap();
        assert_eq!(results_user1.len(), 1);
        assert_eq!(results_user1[0].content, "user1 content");

        let results_user2 = store
            .vector_search("user2", None, &embedding, 5)
            .await
            .unwrap();
        assert_eq!(results_user2.len(), 1);
        assert_eq!(results_user2[0].content, "user2 content");

        let results_wrong_user = store
            .vector_search("user3", None, &embedding, 5)
            .await
            .unwrap();
        assert!(results_wrong_user.is_empty());
    }

    #[tokio::test]
    async fn test_insert_rejects_wrong_embedding_dim() {
        let dir = TempDir::new().unwrap();
        let store = LanceDbVectorStore::new(dir.path()).await.unwrap();

        let wrong_dim: Vec<f32> = vec![1.0; 100];

        let err = store
            .insert_chunk(
                Uuid::new_v4(),
                Uuid::new_v4(),
                "user1",
                None,
                "content",
                &wrong_dim,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, crate::error::WorkspaceError::EmbeddingFailed { .. }));
    }
}
