-- Reborn RootFilesystem event-plane (`append`/`tail`) backing table.
-- Stores monotonic per-path event records. `id` is the assigned `SeqNo`;
-- `path` matches the canonical virtual path. `created_at` is informational
-- only — ordering is by `id` so a clock skew cannot reshuffle the stream.

CREATE TABLE IF NOT EXISTS root_filesystem_events (
    id BIGSERIAL PRIMARY KEY,
    path TEXT NOT NULL CHECK (path LIKE '/%'),
    payload BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index supports the canonical `tail(path, from)` query:
-- `WHERE path = $1 AND id > $2 ORDER BY id ASC`.
CREATE INDEX IF NOT EXISTS idx_root_filesystem_events_path_id
    ON root_filesystem_events(path, id);
