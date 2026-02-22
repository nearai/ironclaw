-- Add chunk versioning to support automatic re-indexing when chunk parameters change.
--
-- ## Why?
-- When chunk size or overlap changes (e.g., 800→300 words for better recall),
-- existing chunks have stale boundaries and embeddings computed on different text.
-- Versioning allows detecting and re-indexing affected documents on startup.
--
-- ## How?
-- - New chunks are inserted with the current CHUNK_VERSION (from chunker.rs)
-- - On startup, documents with chunks at older versions are re-indexed
-- - Re-indexing: delete old chunks, re-chunk content, generate new embeddings
--
-- ## Version History
-- - V1: Initial schema (800 words, 15% overlap)
-- - V2: Smaller chunks for better recall (300 words, 15% overlap)

ALTER TABLE memory_chunks 
ADD COLUMN chunk_version INTEGER NOT NULL DEFAULT 1;

-- Index for efficiently finding stale chunks
-- Query pattern: SELECT DISTINCT document_id WHERE chunk_version < ?
CREATE INDEX idx_memory_chunks_version ON memory_chunks(chunk_version);

COMMENT ON COLUMN memory_chunks.chunk_version IS 
    'Chunk schema version; chunks with version < current are re-indexed on startup';
