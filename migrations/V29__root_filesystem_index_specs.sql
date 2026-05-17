-- Track index declarations for root_filesystem_entries. Records the
-- (prefix, name, keys, kind) of every `ensure_index` call so re-declaration
-- is conflict-aware and idempotent across processes. The actual JSONB
-- expression indexes are created out-of-band by `ensure_index` and named
-- deterministically (`idx_rfs_<sanitized_prefix>_<name>`) so re-running
-- migrations doesn't recreate them.

CREATE TABLE IF NOT EXISTS root_filesystem_index_specs (
    prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    keys JSONB NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (prefix, name)
);
