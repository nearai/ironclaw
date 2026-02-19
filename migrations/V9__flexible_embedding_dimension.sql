-- Allow embedding vectors of any dimension (not just 1536).
-- This supports Ollama models (768-dim nomic-embed-text, 1024-dim mxbai-embed-large)
-- alongside OpenAI models (1536-dim text-embedding-3-small, 3072-dim text-embedding-3-large).
--
-- NOTE: HNSW indexes require a fixed dimension, so we drop the index.
-- Exact (sequential) cosine distance search still works without the index.
-- For a personal assistant workspace the dataset is small enough that this
-- has negligible impact on query latency.

-- Drop dependent views first
DROP VIEW IF EXISTS chunks_pending_embedding;
DROP VIEW IF EXISTS memory_documents_summary;

DROP INDEX IF EXISTS idx_memory_chunks_embedding;

ALTER TABLE memory_chunks
    ALTER COLUMN embedding TYPE vector
    USING embedding::vector;

-- Recreate the views
CREATE VIEW memory_documents_summary AS
SELECT
    d.id,
    d.user_id,
    d.path,
    d.created_at,
    d.updated_at,
    COUNT(c.id) as chunk_count,
    COUNT(c.embedding) as embedded_chunk_count
FROM memory_documents d
LEFT JOIN memory_chunks c ON c.document_id = d.id
GROUP BY d.id;

CREATE VIEW chunks_pending_embedding AS
SELECT
    c.id as chunk_id,
    c.document_id,
    d.user_id,
    d.path,
    LENGTH(c.content) as content_length
FROM memory_chunks c
JOIN memory_documents d ON d.id = c.document_id
WHERE c.embedding IS NULL;
