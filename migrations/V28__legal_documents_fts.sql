-- V28: Per-project full-text search across legal_documents.extracted_text.
--
-- Adds a Postgres tsvector column + GIN index + an auto-update trigger so
-- the gateway's per-project search endpoint
-- (GET /api/skills/legal/projects/:id/search?q=...) can answer queries
-- without a per-row table scan.
--
-- Schema parity: the libSQL backend mirrors this migration as
-- src/db/libsql_migrations.rs entry version 28 ("legal_documents_fts"),
-- which builds an FTS5 virtual table with the same shape. Keep both in
-- sync when extending.
--
-- The Postgres legal query layer is not wired in v1 (libSQL is the v1
-- deployment target), so this migration ships the schema and trigger
-- but no Rust caller. The gateway returns 501 from the Postgres backend,
-- matching the chat / foundation handlers' shape.

ALTER TABLE legal_documents
    ADD COLUMN IF NOT EXISTS search_tsv tsvector;

UPDATE legal_documents
   SET search_tsv = to_tsvector(
       'english',
       COALESCE(filename, '') || ' ' || COALESCE(extracted_text, '')
   )
 WHERE search_tsv IS NULL;

CREATE INDEX IF NOT EXISTS idx_legal_documents_search_tsv
    ON legal_documents USING GIN (search_tsv);

CREATE OR REPLACE FUNCTION legal_documents_tsv_update() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.search_tsv := to_tsvector(
        'english',
        COALESCE(NEW.filename, '') || ' ' || COALESCE(NEW.extracted_text, '')
    );
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS legal_documents_tsv_trigger ON legal_documents;
CREATE TRIGGER legal_documents_tsv_trigger
    BEFORE INSERT OR UPDATE OF filename, extracted_text
    ON legal_documents
    FOR EACH ROW EXECUTE FUNCTION legal_documents_tsv_update();
