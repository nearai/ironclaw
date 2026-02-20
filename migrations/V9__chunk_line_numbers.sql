-- Add line number tracking to memory chunks for citation support
-- Allows citations like "Source: daily/2026-02-12.md#lines 15-23"

ALTER TABLE memory_chunks
ADD COLUMN line_start INT,
ADD COLUMN line_end INT;

-- Add index for line-based queries (optional, useful for range lookups)
CREATE INDEX idx_memory_chunks_lines ON memory_chunks(document_id, line_start, line_end)
WHERE line_start IS NOT NULL;

COMMENT ON COLUMN memory_chunks.line_start IS 'Starting line number in source document (1-based)';
COMMENT ON COLUMN memory_chunks.line_end IS 'Ending line number in source document (1-based, inclusive)';
