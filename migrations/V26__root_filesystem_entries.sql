-- Reborn RootFilesystem database backend storage.
-- Stores canonical virtual-path file contents; directories are inferred from path prefixes.

CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY CHECK (path LIKE '/%'),
    contents BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_root_filesystem_entries_path
    ON root_filesystem_entries(path);
