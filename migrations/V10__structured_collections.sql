-- Structured collections: typed, queryable records stored as JSONB.
--
-- Each collection has a schema defining named fields with types, required flags,
-- and defaults. Records are validated against the schema on insert/update by the
-- application layer, stored as JSONB, and queryable via filters and aggregations.
--
-- Two tables:
--   structured_schemas  — one row per (user_id, collection), holds the schema JSON
--   structured_records  — JSONB records referencing a schema, with GIN index for queries
--
-- All operations are scoped by user_id to enforce tenant isolation (same model as
-- workspace memory_documents). The application layer validates records against the
-- schema before insert/update; the database stores the validated JSONB as-is.

-- ==================== Schema Registry ====================

CREATE TABLE IF NOT EXISTS structured_schemas (
    user_id     TEXT        NOT NULL,
    collection  TEXT        NOT NULL,
    schema      JSONB       NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, collection)
);

CREATE TRIGGER update_structured_schemas_updated_at
    BEFORE UPDATE ON structured_schemas
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ==================== Records ====================

CREATE TABLE IF NOT EXISTS structured_records (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     TEXT        NOT NULL,
    collection  TEXT        NOT NULL,
    data        JSONB       NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Deleting a schema cascades to all its records, preventing orphans.
    FOREIGN KEY (user_id, collection)
        REFERENCES structured_schemas (user_id, collection)
        ON DELETE CASCADE
);

-- GIN index on data for efficient JSONB containment and key-exists queries
-- (e.g., data @> '{"status": "completed"}', data ? 'notes').
CREATE INDEX idx_structured_records_data
    ON structured_records USING GIN (data);

-- Composite index for the most common query pattern: list records in a
-- collection for a user, ordered by creation time.
CREATE INDEX idx_structured_records_lookup
    ON structured_records (user_id, collection, created_at);

CREATE TRIGGER update_structured_records_updated_at
    BEFORE UPDATE ON structured_records
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
