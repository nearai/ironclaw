CREATE TABLE log_entries (
    id         BIGSERIAL PRIMARY KEY,
    level      TEXT        NOT NULL,
    target     TEXT        NOT NULL,
    message    TEXT        NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_log_entries_recorded_at ON log_entries (recorded_at DESC);
