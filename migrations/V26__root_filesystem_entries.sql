-- Reborn RootFilesystem database backend storage.
-- Stores canonical virtual-path file contents; directories are inferred from path prefixes.

CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY CHECK (path LIKE '/%'),
    contents BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index uses text_pattern_ops so prefix `LIKE '/path/%'` queries used by the
-- DB backend's child-entry scans can be served from the index. Equality
-- lookups already use the PRIMARY KEY's btree.
CREATE INDEX IF NOT EXISTS idx_root_filesystem_entries_path
    ON root_filesystem_entries(path text_pattern_ops);
