CREATE TABLE IF NOT EXISTS root_filesystem_ordered_index_rows (
    index_name TEXT NOT NULL,
    path TEXT NOT NULL,
    k0 JSONB,
    k1 JSONB,
    k2 JSONB,
    k3 JSONB,
    k4 JSONB,
    k5 JSONB,
    k6 JSONB,
    k7 JSONB,
    PRIMARY KEY (index_name, path),
    FOREIGN KEY (path) REFERENCES root_filesystem_entries(path) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_root_filesystem_ordered_values_v1
    ON root_filesystem_ordered_index_rows(
        index_name, k0, k1, k2, k3, k4, k5, k6, k7, path
    );
