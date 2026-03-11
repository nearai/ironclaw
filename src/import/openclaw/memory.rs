//! OpenClaw memory chunk import.

use std::sync::Arc;

use crate::db::Database;
use crate::import::{ImportError, ImportOptions};

use super::reader::OpenClawMemoryChunk;

/// Import a single memory chunk into IronClaw.
pub async fn import_chunk(
    db: &Arc<dyn Database>,
    chunk: &OpenClawMemoryChunk,
    opts: &ImportOptions,
) -> Result<(), ImportError> {
    // Get or create document by path
    let doc = db
        .get_or_create_document_by_path(&opts.user_id, None, &chunk.path)
        .await
        .map_err(|e| ImportError::Database(e.to_string()))?;

    // Insert chunk
    let chunk_id = db
        .insert_chunk(
            doc.id,
            chunk.chunk_index,
            &chunk.content,
            None, // Don't set embedding yet if dimensions might not match
        )
        .await
        .map_err(|e| ImportError::Database(e.to_string()))?;

    // If we have an embedding, try to update it
    if let Some(ref embedding) = chunk.embedding {
        // Note: dimension check would go here if we had target dimensions available
        // For now, just store what we have
        db.update_chunk_embedding(chunk_id, embedding)
            .await
            .map_err(|e| ImportError::Database(e.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_chunk_import_structure() {
        // Verify that OpenClawMemoryChunk can be created with test data
        let chunk = OpenClawMemoryChunk {
            path: "test/path.md".to_string(),
            content: "Test content".to_string(),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            chunk_index: 0,
        };

        assert_eq!(chunk.path, "test/path.md");
        assert_eq!(chunk.chunk_index, 0);
        assert!(chunk.embedding.is_some());
    }
}
