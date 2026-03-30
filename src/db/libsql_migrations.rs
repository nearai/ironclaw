//! SQLite-dialect migrations for the libSQL/Turso backend.
//!
//! Consolidates all PostgreSQL migrations (V1-V8) into a single SQLite-compatible
//! schema. Run once on database creation; idempotent via `IF NOT EXISTS`.
//!
//! Incremental migrations (V9+) are tracked in the `_migrations` table and run
//! exactly once per database, in version order.

/// Consolidated schema for libSQL.
///
/// Translates PostgreSQL types and features:
/// - `UUID` -> `TEXT` (store as hex string)
/// - `TIMESTAMPTZ` -> `TEXT` (ISO-8601)
/// - `JSONB` -> `TEXT` (JSON encoded)
/// - `BYTEA` -> `BLOB`
/// - `NUMERIC` -> `TEXT` (preserve precision for rust_decimal)
/// - `TEXT[]` -> `TEXT` (JSON array)
/// - `VECTOR` -> `BLOB` (raw little-endian F32 bytes, any dimension)
/// - `TSVECTOR` -> FTS5 virtual table
/// - `BIGSERIAL` -> `INTEGER PRIMARY KEY AUTOINCREMENT`
/// - PL/pgSQL functions -> SQLite triggers
pub const SCHEMA: &str = r#"

-- ==================== Migration tracking ====================

CREATE TABLE IF NOT EXISTS _migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Conversations ====================

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_activity TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_conversations_channel ON conversations(channel);
CREATE INDEX IF NOT EXISTS idx_conversations_user ON conversations(user_id);
CREATE INDEX IF NOT EXISTS idx_conversations_last_activity ON conversations(last_activity);

-- Partial unique indexes to prevent duplicate singleton conversations.
CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_routine
ON conversations (user_id, json_extract(metadata, '$.routine_id'))
WHERE json_extract(metadata, '$.routine_id') IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_heartbeat
ON conversations (user_id)
WHERE json_extract(metadata, '$.thread_type') = 'heartbeat';

CREATE TABLE IF NOT EXISTS conversation_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_conversation_messages_conversation
    ON conversation_messages(conversation_id);

-- ==================== Agent Jobs ====================

CREATE TABLE IF NOT EXISTS agent_jobs (
    id TEXT PRIMARY KEY,
    marketplace_job_id TEXT,
    conversation_id TEXT REFERENCES conversations(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT 'default',
    project_dir TEXT,
    job_mode TEXT NOT NULL DEFAULT 'worker',
    budget_amount TEXT,
    budget_token TEXT,
    bid_amount TEXT,
    estimated_cost TEXT,
    estimated_time_secs INTEGER,
    estimated_value TEXT,
    actual_cost TEXT,
    actual_time_secs INTEGER,
    success INTEGER,
    failure_reason TEXT,
    stuck_since TEXT,
    repair_attempts INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    started_at TEXT,
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_jobs_status ON agent_jobs(status);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_marketplace ON agent_jobs(marketplace_job_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_conversation ON agent_jobs(conversation_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_source ON agent_jobs(source);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_user ON agent_jobs(user_id);
CREATE INDEX IF NOT EXISTS idx_agent_jobs_created ON agent_jobs(created_at DESC);

CREATE TABLE IF NOT EXISTS job_actions (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    input TEXT NOT NULL,
    output_raw TEXT,
    output_sanitized TEXT,
    sanitization_warnings TEXT,
    cost TEXT,
    duration_ms INTEGER,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(job_id, sequence_num)
);

CREATE INDEX IF NOT EXISTS idx_job_actions_job_id ON job_actions(job_id);
CREATE INDEX IF NOT EXISTS idx_job_actions_tool ON job_actions(tool_name);

-- ==================== Dynamic Tools ====================

CREATE TABLE IF NOT EXISTS dynamic_tools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    parameters_schema TEXT NOT NULL,
    code TEXT NOT NULL,
    sandbox_config TEXT NOT NULL,
    created_by_job_id TEXT REFERENCES agent_jobs(id),
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_dynamic_tools_status ON dynamic_tools(status);
CREATE INDEX IF NOT EXISTS idx_dynamic_tools_name ON dynamic_tools(name);

-- ==================== LLM Calls ====================

CREATE TABLE IF NOT EXISTS llm_calls (
    id TEXT PRIMARY KEY,
    job_id TEXT REFERENCES agent_jobs(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES conversations(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost TEXT NOT NULL,
    purpose TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_llm_calls_job ON llm_calls(job_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_conversation ON llm_calls(conversation_id);
CREATE INDEX IF NOT EXISTS idx_llm_calls_provider ON llm_calls(provider);

-- ==================== Estimation ====================

CREATE TABLE IF NOT EXISTS estimation_snapshots (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    tool_names TEXT NOT NULL DEFAULT '[]',
    estimated_cost TEXT NOT NULL,
    actual_cost TEXT,
    estimated_time_secs INTEGER NOT NULL,
    actual_time_secs INTEGER,
    estimated_value TEXT NOT NULL,
    actual_value TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_estimation_category ON estimation_snapshots(category);
CREATE INDEX IF NOT EXISTS idx_estimation_job ON estimation_snapshots(job_id);

-- ==================== Self Repair ====================

CREATE TABLE IF NOT EXISTS repair_attempts (
    id TEXT PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    diagnosis TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_repair_attempts_target ON repair_attempts(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_repair_attempts_created ON repair_attempts(created_at);

-- ==================== Workspace: Memory Documents ====================

CREATE TABLE IF NOT EXISTS memory_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id, path)
);

CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX IF NOT EXISTS idx_memory_documents_updated ON memory_documents(updated_at DESC);

-- Trigger to auto-update updated_at on memory_documents
CREATE TRIGGER IF NOT EXISTS update_memory_documents_updated_at
    AFTER UPDATE ON memory_documents
    FOR EACH ROW
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN
        UPDATE memory_documents SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
    END;

-- ==================== Workspace: Memory Chunks ====================

CREATE TABLE IF NOT EXISTS memory_chunks (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

-- No vector index in base schema: BLOB column accepts any embedding dimension.
-- Vector index is created dynamically by ensure_vector_index() during
-- run_migrations() when embeddings are configured (EMBEDDING_ENABLED=true).

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_fts USING fts5(
    content,
    content='memory_chunks',
    content_rowid='_rowid'
);

-- Triggers to keep FTS5 in sync with memory_chunks
CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

-- ==================== Workspace: Heartbeat State ====================

CREATE TABLE IF NOT EXISTS heartbeat_state (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    last_run TEXT,
    next_run TEXT,
    interval_seconds INTEGER NOT NULL DEFAULT 1800,
    enabled INTEGER NOT NULL DEFAULT 1,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_checks TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id)
);

CREATE INDEX IF NOT EXISTS idx_heartbeat_user ON heartbeat_state(user_id);

-- ==================== Secrets ====================

CREATE TABLE IF NOT EXISTS secrets (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    key_salt BLOB NOT NULL,
    provider TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_secrets_user ON secrets(user_id);

-- ==================== WASM Tools ====================

CREATE TABLE IF NOT EXISTS wasm_tools (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    wit_version TEXT NOT NULL DEFAULT '0.1.0',
    description TEXT NOT NULL,
    wasm_binary BLOB NOT NULL,
    binary_hash BLOB NOT NULL,
    parameters_schema TEXT NOT NULL,
    source_url TEXT,
    trust_level TEXT NOT NULL DEFAULT 'user',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name, version)
);

CREATE INDEX IF NOT EXISTS idx_wasm_tools_user ON wasm_tools(user_id);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_name ON wasm_tools(user_id, name);
CREATE INDEX IF NOT EXISTS idx_wasm_tools_status ON wasm_tools(status);

-- ==================== WASM Channel Extensions ====================

CREATE TABLE IF NOT EXISTS wasm_channels (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '0.1.0',
    wit_version TEXT NOT NULL DEFAULT '0.1.0',
    description TEXT NOT NULL DEFAULT '',
    wasm_binary BLOB NOT NULL,
    binary_hash BLOB NOT NULL,
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

-- ==================== Tool Capabilities ====================

CREATE TABLE IF NOT EXISTS tool_capabilities (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    http_allowlist TEXT NOT NULL DEFAULT '[]',
    allowed_secrets TEXT NOT NULL DEFAULT '[]',
    tool_aliases TEXT NOT NULL DEFAULT '{}',
    requests_per_minute INTEGER NOT NULL DEFAULT 60,
    requests_per_hour INTEGER NOT NULL DEFAULT 1000,
    max_request_body_bytes INTEGER NOT NULL DEFAULT 1048576,
    max_response_body_bytes INTEGER NOT NULL DEFAULT 10485760,
    workspace_read_prefixes TEXT NOT NULL DEFAULT '[]',
    http_timeout_secs INTEGER NOT NULL DEFAULT 30,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (wasm_tool_id)
);

-- ==================== Leak Detection Patterns ====================

CREATE TABLE IF NOT EXISTS leak_detection_patterns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    pattern TEXT NOT NULL,
    severity TEXT NOT NULL DEFAULT 'high',
    action TEXT NOT NULL DEFAULT 'block',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Rate Limit State ====================

CREATE TABLE IF NOT EXISTS tool_rate_limit_state (
    id TEXT PRIMARY KEY,
    wasm_tool_id TEXT NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    minute_window_start TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    minute_count INTEGER NOT NULL DEFAULT 0,
    hour_window_start TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    hour_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (wasm_tool_id, user_id)
);

-- ==================== Secret Usage Audit Log ====================

CREATE TABLE IF NOT EXISTS secret_usage_log (
    id TEXT PRIMARY KEY,
    secret_id TEXT NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    target_host TEXT NOT NULL,
    target_path TEXT,
    success INTEGER NOT NULL,
    error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_secret_usage_user ON secret_usage_log(user_id);

-- ==================== Leak Detection Events ====================

CREATE TABLE IF NOT EXISTS leak_detection_events (
    id TEXT PRIMARY KEY,
    pattern_id TEXT REFERENCES leak_detection_patterns(id) ON DELETE SET NULL,
    wasm_tool_id TEXT REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,
    source TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    context_preview TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ==================== Tool Failures ====================

CREATE TABLE IF NOT EXISTS tool_failures (
    id TEXT PRIMARY KEY,
    tool_name TEXT NOT NULL UNIQUE,
    error_message TEXT,
    error_count INTEGER DEFAULT 1,
    first_failure TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_failure TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_build_result TEXT,
    repaired_at TEXT,
    repair_attempts INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_tool_failures_name ON tool_failures(tool_name);

-- ==================== Job Events ====================

CREATE TABLE IF NOT EXISTS job_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL REFERENCES agent_jobs(id),
    event_type TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_job_events_job ON job_events(job_id, id);

-- ==================== Routines ====================

CREATE TABLE IF NOT EXISTS routines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,
    notify_channel TEXT,
    notify_user TEXT,
    notify_on_success INTEGER NOT NULL DEFAULT 0,
    notify_on_failure INTEGER NOT NULL DEFAULT 1,
    notify_on_attention INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT '{}',
    last_run_at TEXT,
    next_fire_at TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_routines_user ON routines(user_id);

-- ==================== Routine Runs ====================

CREATE TABLE IF NOT EXISTS routine_runs (
    id TEXT PRIMARY KEY,
    routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    trigger_type TEXT NOT NULL,
    trigger_detail TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    completed_at TEXT,
    status TEXT NOT NULL DEFAULT 'running',
    result_summary TEXT,
    tokens_used INTEGER,
    job_id TEXT REFERENCES agent_jobs(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_routine_runs_routine ON routine_runs(routine_id);

-- ==================== Settings ====================

CREATE TABLE IF NOT EXISTS settings (
    user_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_settings_user ON settings(user_id);

-- ==================== Missing indexes (parity with PostgreSQL) ====================

-- agent_jobs
CREATE INDEX IF NOT EXISTS idx_agent_jobs_stuck ON agent_jobs(stuck_since);

-- secrets
CREATE INDEX IF NOT EXISTS idx_secrets_provider ON secrets(provider);
CREATE INDEX IF NOT EXISTS idx_secrets_expires ON secrets(expires_at);

-- wasm_tools
CREATE INDEX IF NOT EXISTS idx_wasm_tools_trust ON wasm_tools(trust_level);

-- tool_capabilities
CREATE INDEX IF NOT EXISTS idx_tool_capabilities_tool ON tool_capabilities(wasm_tool_id);

-- leak_detection_patterns
CREATE INDEX IF NOT EXISTS idx_leak_patterns_enabled ON leak_detection_patterns(enabled);

-- tool_rate_limit_state
CREATE INDEX IF NOT EXISTS idx_rate_limit_tool ON tool_rate_limit_state(wasm_tool_id);

-- secret_usage_log
CREATE INDEX IF NOT EXISTS idx_secret_usage_secret ON secret_usage_log(secret_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_tool ON secret_usage_log(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_secret_usage_created ON secret_usage_log(created_at DESC);

-- leak_detection_events
CREATE INDEX IF NOT EXISTS idx_leak_events_pattern ON leak_detection_events(pattern_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_tool ON leak_detection_events(wasm_tool_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_user ON leak_detection_events(user_id);
CREATE INDEX IF NOT EXISTS idx_leak_events_created ON leak_detection_events(created_at DESC);

-- tool_failures
CREATE INDEX IF NOT EXISTS idx_tool_failures_count ON tool_failures(error_count DESC);
CREATE INDEX IF NOT EXISTS idx_tool_failures_unrepaired ON tool_failures(tool_name);

-- routines
CREATE INDEX IF NOT EXISTS idx_routines_next_fire ON routines(next_fire_at);
CREATE INDEX IF NOT EXISTS idx_routines_event_triggers
    ON routines(trigger_type, user_id)
    WHERE enabled = 1 AND trigger_type IN ('event', 'system_event');

-- routine_runs
CREATE INDEX IF NOT EXISTS idx_routine_runs_status ON routine_runs(status);

-- heartbeat_state
CREATE INDEX IF NOT EXISTS idx_heartbeat_next_run ON heartbeat_state(next_run);

-- ==================== Seed data ====================

-- Pre-populate leak detection patterns (matches PostgreSQL V2 migration).
INSERT OR IGNORE INTO leak_detection_patterns (id, name, pattern, severity, action, enabled, created_at) VALUES
    ('550e8400-e29b-41d4-a716-446655440001', 'openai_api_key', 'sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440002', 'anthropic_api_key', 'sk-ant-api[a-zA-Z0-9_-]{90,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440003', 'aws_access_key', 'AKIA[0-9A-Z]{16}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440004', 'aws_secret_key', '(?<![A-Za-z0-9/+=])[A-Za-z0-9/+=]{40}(?![A-Za-z0-9/+=])', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440005', 'github_token', 'gh[pousr]_[A-Za-z0-9_]{36,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440006', 'github_fine_grained_pat', 'github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440007', 'stripe_api_key', 'sk_(?:live|test)_[a-zA-Z0-9]{24,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440008', 'nearai_session', 'sess_[a-zA-Z0-9]{32,}', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440009', 'bearer_token', 'Bearer\s+[a-zA-Z0-9_-]{20,}', 'high', 'redact', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000a', 'pem_private_key', '-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000b', 'ssh_private_key', '-----BEGIN\s+(?:OPENSSH|EC|DSA)\s+PRIVATE\s+KEY-----', 'critical', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000c', 'google_api_key', 'AIza[0-9A-Za-z_-]{35}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000d', 'slack_token', 'xox[baprs]-[0-9a-zA-Z-]{10,}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000e', 'discord_token', '[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-44665544000f', 'twilio_api_key', 'SK[a-fA-F0-9]{32}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440010', 'sendgrid_api_key', 'SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}', 'high', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440011', 'mailchimp_api_key', '[a-f0-9]{32}-us[0-9]{1,2}', 'medium', 'block', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('550e8400-e29b-41d4-a716-446655440012', 'high_entropy_hex', '(?<![a-fA-F0-9])[a-fA-F0-9]{64}(?![a-fA-F0-9])', 'medium', 'warn', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));


-- ==================== User management (V14) ====================

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    role TEXT NOT NULL DEFAULT 'member',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_login_at TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL,
    token_prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);

"#;

/// Incremental migrations applied after the base schema.
///
/// Each entry is `(version, name, sql)`. Migrations are idempotent: the
/// `_migrations` table tracks which versions have been applied.
pub const INCREMENTAL_MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        9,
        "flexible_embedding_dimension",
        // Rebuild memory_chunks to remove the fixed F32_BLOB(1536) type
        // constraint so any embedding dimension works. Existing embeddings
        // are preserved; users only need to re-embed if they change models.
        //
        // The vector index is dropped here; ensure_vector_index() recreates
        // it with the correct F32_BLOB(N) dimension during run_migrations()
        // when embeddings are configured.
        //
        // SQLite cannot ALTER COLUMN types, so we recreate the table.
        r#"
-- Drop vector index (requires fixed F32_BLOB(N), incompatible with flexible dimensions)
DROP INDEX IF EXISTS idx_memory_chunks_embedding;

-- Drop FTS triggers that reference the old table
DROP TRIGGER IF EXISTS memory_chunks_fts_insert;
DROP TRIGGER IF EXISTS memory_chunks_fts_delete;
DROP TRIGGER IF EXISTS memory_chunks_fts_update;

-- Recreate table with flexible BLOB column (any embedding dimension)
CREATE TABLE IF NOT EXISTS memory_chunks_new (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

-- Copy all existing data (embeddings preserved as-is)
INSERT OR IGNORE INTO memory_chunks_new (_rowid, id, document_id, chunk_index, content, embedding, created_at)
    SELECT _rowid, id, document_id, chunk_index, content, embedding, created_at FROM memory_chunks;

-- Swap tables
DROP TABLE memory_chunks;
ALTER TABLE memory_chunks_new RENAME TO memory_chunks;

-- Recreate indexes (no vector index — see comment above)
CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

-- Recreate FTS triggers
CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
END;

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
    INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
END;
"#,
    ),
    (
        12,
        "job_token_budget",
        // Add token budget tracking columns to agent_jobs.
        // SQLite supports ALTER TABLE ADD COLUMN, so no table rebuild needed.
        r#"
ALTER TABLE agent_jobs ADD COLUMN max_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_jobs ADD COLUMN total_tokens_used INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        13,
        "routine_notify_user_nullable",
        // Remove the legacy 'default' sentinel from routine notify_user.
        // SQLite cannot drop NOT NULL / DEFAULT constraints in place, so we
        // rebuild the table and normalize existing 'default' values to NULL.
        r#"
PRAGMA foreign_keys=OFF;

CREATE TABLE IF NOT EXISTS routines_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    trigger_type TEXT NOT NULL,
    trigger_config TEXT NOT NULL,
    action_type TEXT NOT NULL,
    action_config TEXT NOT NULL,
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,
    notify_channel TEXT,
    notify_user TEXT,
    notify_on_success INTEGER NOT NULL DEFAULT 0,
    notify_on_failure INTEGER NOT NULL DEFAULT 1,
    notify_on_attention INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT '{}',
    last_run_at TEXT,
    next_fire_at TEXT,
    run_count INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (user_id, name)
);

INSERT INTO routines_new (
    id, name, description, user_id, enabled,
    trigger_type, trigger_config, action_type, action_config,
    cooldown_secs, max_concurrent, dedup_window_secs,
    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention,
    state, last_run_at, next_fire_at, run_count, consecutive_failures,
    created_at, updated_at
)
SELECT
    id, name, description, user_id, enabled,
    trigger_type, trigger_config, action_type, action_config,
    cooldown_secs, max_concurrent, dedup_window_secs,
    notify_channel,
    CASE WHEN notify_user = 'default' THEN NULL ELSE notify_user END,
    notify_on_success, notify_on_failure, notify_on_attention,
    state, last_run_at, next_fire_at, run_count, consecutive_failures,
    created_at, updated_at
FROM routines;

DROP TABLE routines;
ALTER TABLE routines_new RENAME TO routines;

CREATE INDEX IF NOT EXISTS idx_routines_user ON routines(user_id);
CREATE INDEX IF NOT EXISTS idx_routines_next_fire ON routines(next_fire_at);
CREATE INDEX IF NOT EXISTS idx_routines_event_triggers
    ON routines(trigger_type, user_id)
    WHERE enabled = 1 AND trigger_type IN ('event', 'system_event');

PRAGMA foreign_keys=ON;
"#,
    ),
    (
        14,
        "users",
        r#"
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    role TEXT NOT NULL DEFAULT 'member',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    last_login_at TEXT,
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL,
    token_prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    expires_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);
"#,
    ),
    (
        15,
        "judge_verdicts",
        // Audit trail for LLM-as-Judge evaluations. Append-only; no rows are
        // ever updated or deleted. confidence=0.0 with reasoning="Judge
        // unavailable — failing open" is the sentinel for fail-open events.
        r#"
CREATE TABLE IF NOT EXISTS judge_verdicts (
    id          TEXT    PRIMARY KEY,
    tool_name   TEXT    NOT NULL,
    verdict     TEXT    NOT NULL,
    attack_type TEXT,
    confidence  REAL    NOT NULL,
    reasoning   TEXT    NOT NULL,
    latency_ms  INTEGER NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_judge_verdicts_tool    ON judge_verdicts(tool_name);
CREATE INDEX IF NOT EXISTS idx_judge_verdicts_verdict ON judge_verdicts(verdict);
CREATE INDEX IF NOT EXISTS idx_judge_verdicts_created ON judge_verdicts(created_at DESC);
"#,
    ),
];

/// Run incremental migrations that haven't been applied yet.
///
/// Each migration is wrapped in a transaction. On success the version is
/// recorded in `_migrations` so it won't run again.
pub async fn run_incremental(conn: &libsql::Connection) -> Result<(), crate::error::DatabaseError> {
    use crate::error::DatabaseError;

    let mut applied_count = 0;
    for &(version, name, sql) in INCREMENTAL_MIGRATIONS {
        // Check if already applied
        let mut rows = conn
            .query(
                "SELECT 1 FROM _migrations WHERE version = ?1",
                libsql::params![version],
            )
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Failed to check migration {version}: {e}"))
            })?;

        if rows.next().await.ok().flatten().is_some() {
            continue; // Already applied
        }

        // Wrap migration + recording in a transaction for atomicity.
        // If the process crashes mid-migration, the transaction rolls back
        // and the migration will be retried on next startup.
        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "libSQL migration V{version}: failed to start transaction: {e}"
            ))
        })?;

        tx.execute_batch(sql).await.map_err(|e| {
            DatabaseError::Migration(format!("libSQL migration V{version} ({name}) failed: {e}"))
        })?;

        // Record as applied (inside the same transaction)
        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (?1, ?2)",
            libsql::params![version, name],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to record migration V{version} ({name}): {e}"
            ))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "libSQL migration V{version} ({name}): commit failed: {e}"
            ))
        })?;

        applied_count += 1;
        tracing::debug!(version, name, "libSQL: migration applied");
    }

    if applied_count > 0 {
        tracing::info!("libSQL: applied {} incremental migrations", applied_count);
    }

    Ok(())
}
