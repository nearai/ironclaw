CREATE TABLE IF NOT EXISTS reborn_event_streams (
    stream_kind TEXT NOT NULL CHECK (stream_kind IN ('runtime', 'audit')),
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT '',
    next_cursor INTEGER NOT NULL DEFAULT 0 CHECK (next_cursor >= 0),
    earliest_retained INTEGER NOT NULL DEFAULT 0 CHECK (earliest_retained >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (stream_kind, tenant_id, user_id, agent_id)
);

CREATE TABLE IF NOT EXISTS reborn_event_entries (
    stream_kind TEXT NOT NULL CHECK (stream_kind IN ('runtime', 'audit')),
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT '',
    cursor INTEGER NOT NULL CHECK (cursor > 0),
    record_id TEXT NOT NULL,
    record_kind TEXT NOT NULL,
    project_id TEXT,
    mission_id TEXT,
    thread_id TEXT,
    process_id TEXT,
    occurred_at TEXT NOT NULL,
    record_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (stream_kind, tenant_id, user_id, agent_id, cursor),
    FOREIGN KEY (stream_kind, tenant_id, user_id, agent_id)
        REFERENCES reborn_event_streams (stream_kind, tenant_id, user_id, agent_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS reborn_event_entries_scope_idx
    ON reborn_event_entries (
        stream_kind,
        tenant_id,
        user_id,
        agent_id,
        project_id,
        mission_id,
        thread_id,
        process_id,
        cursor
    );

CREATE INDEX IF NOT EXISTS reborn_event_entries_record_kind_idx
    ON reborn_event_entries (stream_kind, record_kind, occurred_at);
