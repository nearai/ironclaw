-- Document version history for workspace files.
-- Every content update saves the previous content as a version,
-- enabling rollback and audit trails.

CREATE TABLE memory_document_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    changed_by TEXT,
    UNIQUE(document_id, version)
);

CREATE INDEX idx_doc_versions_lookup
    ON memory_document_versions(document_id, version DESC);

-- GIN index on metadata for future JSON containment queries
-- (e.g., WHERE metadata @> '{"hygiene": {"enabled": true}}').
-- Not used by current queries (which use path LIKE) but enables
-- efficient metadata-based filtering without a full table scan.
CREATE INDEX idx_memory_documents_metadata
    ON memory_documents USING GIN (metadata jsonb_path_ops);
