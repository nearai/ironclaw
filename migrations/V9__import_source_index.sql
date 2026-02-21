-- Speed up idempotent import lookups by source tuple.
CREATE INDEX IF NOT EXISTS idx_conversations_import_source_lookup
    ON conversations (
        user_id,
        channel,
        ((metadata->'import'->>'source')),
        ((metadata->'import'->>'source_id'))
    )
    WHERE metadata ? 'import';
