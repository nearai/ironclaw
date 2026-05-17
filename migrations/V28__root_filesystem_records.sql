-- Extend root_filesystem_entries to carry Entry record metadata (kind,
-- indexed projection, content type, version) in addition to opaque bytes.
-- A row with `kind IS NULL` and `indexed = '{}'` is an opaque file — the
-- legacy shape, preserved by safe defaults so existing entries round-trip.
-- A row with `kind` set carries a typed record whose `indexed` projection
-- is queryable by future query/ensure_index ops. `version` enables
-- compare-and-swap semantics on `put`.

ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS content_type TEXT NOT NULL DEFAULT 'application/octet-stream';

ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS kind TEXT;

ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS indexed JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 0;
