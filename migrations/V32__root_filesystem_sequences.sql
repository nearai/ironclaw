-- Path-local monotonic sequence allocator for row-shaped stores.
--
-- `root_filesystem_events` assigns globally increasing ids, which are correct
-- for event replay cursors but not for per-record-set ordering such as thread
-- message sequences. This table keeps one atomic counter per virtual path.

CREATE TABLE IF NOT EXISTS root_filesystem_sequences (
    path TEXT PRIMARY KEY CHECK (path LIKE '/%'),
    next_seq BIGINT NOT NULL CHECK (next_seq > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
