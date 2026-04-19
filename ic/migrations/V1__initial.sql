-- NEAR Agent Database Schema
-- V1: Complete schema with workspace and memory system

-- Enable pgvector extension for semantic search
-- NOTE: Requires pgvector to be installed on PostgreSQL server
CREATE EXTENSION IF NOT EXISTS vector;

-- ==================== Conversations ====================

CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    channel TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_activity TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_conversations_channel ON conversations(channel);
CREATE INDEX idx_conversations_user ON conversations(user_id);
CREATE INDEX idx_conversations_last_activity ON conversations(last_activity);

CREATE TABLE conversation_messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_conversation_messages_conversation ON conversation_messages(conversation_id);

-- ==================== Agent Jobs ====================

CREATE TABLE agent_jobs (
    id UUID PRIMARY KEY,
    marketplace_job_id UUID,
    conversation_id UUID REFERENCES conversations(id),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    category TEXT,
    status TEXT NOT NULL,
    source TEXT NOT NULL,
    budget_amount NUMERIC,
    budget_token TEXT,
    bid_amount NUMERIC,
    estimated_cost NUMERIC,
    estimated_time_secs INTEGER,
    estimated_value NUMERIC,
    actual_cost NUMERIC,
    actual_time_secs INTEGER,
    success BOOLEAN,
    failure_reason TEXT,
    stuck_since TIMESTAMPTZ,
    repair_attempts INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE INDEX idx_agent_jobs_status ON agent_jobs(status);
CREATE INDEX idx_agent_jobs_marketplace ON agent_jobs(marketplace_job_id);
CREATE INDEX idx_agent_jobs_conversation ON agent_jobs(conversation_id);
CREATE INDEX idx_agent_jobs_stuck ON agent_jobs(stuck_since) WHERE stuck_since IS NOT NULL;

CREATE TABLE job_actions (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    sequence_num INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    input JSONB NOT NULL,
    output_raw TEXT,
    output_sanitized JSONB,
    sanitization_warnings JSONB,
    cost NUMERIC,
    duration_ms INTEGER,
    success BOOLEAN NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(job_id, sequence_num)
);

CREATE INDEX idx_job_actions_job_id ON job_actions(job_id);
CREATE INDEX idx_job_actions_tool ON job_actions(tool_name);

-- ==================== Dynamic Tools ====================

CREATE TABLE dynamic_tools (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    parameters_schema JSONB NOT NULL,
    code TEXT NOT NULL,
    sandbox_config JSONB NOT NULL,
    created_by_job_id UUID REFERENCES agent_jobs(id),
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_dynamic_tools_status ON dynamic_tools(status);
CREATE INDEX idx_dynamic_tools_name ON dynamic_tools(name);

-- ==================== LLM Calls ====================

CREATE TABLE llm_calls (
    id UUID PRIMARY KEY,
    job_id UUID REFERENCES agent_jobs(id) ON DELETE CASCADE,
    conversation_id UUID REFERENCES conversations(id),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost NUMERIC NOT NULL,
    purpose TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_llm_calls_job ON llm_calls(job_id);
CREATE INDEX idx_llm_calls_conversation ON llm_calls(conversation_id);
CREATE INDEX idx_llm_calls_provider ON llm_calls(provider);

-- ==================== Estimation ====================

CREATE TABLE estimation_snapshots (
    id UUID PRIMARY KEY,
    job_id UUID NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    tool_names TEXT[] NOT NULL,
    estimated_cost NUMERIC NOT NULL,
    actual_cost NUMERIC,
    estimated_time_secs INTEGER NOT NULL,
    actual_time_secs INTEGER,
    estimated_value NUMERIC NOT NULL,
    actual_value NUMERIC,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_estimation_category ON estimation_snapshots(category);
CREATE INDEX idx_estimation_job ON estimation_snapshots(job_id);

-- ==================== Self Repair ====================

CREATE TABLE repair_attempts (
    id UUID PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id UUID NOT NULL,
    diagnosis TEXT NOT NULL,
    action_taken TEXT NOT NULL,
    success BOOLEAN NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_repair_attempts_target ON repair_attempts(target_type, target_id);
CREATE INDEX idx_repair_attempts_created ON repair_attempts(created_at);

-- ==================== Workspace: Memory Documents ====================
-- Flexible filesystem-like structure for agent memory.
-- Agents can create arbitrary paths like:
--   "README.md", "context/vision.md", "daily/2024-01-15.md", "projects/alpha/notes.md"

CREATE TABLE memory_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id UUID,  -- NULL = shared across all agents for this user

    -- File path within workspace (e.g., "context/vision.md")
    path TEXT NOT NULL,
    content TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}',

    CONSTRAINT unique_path_per_user UNIQUE (user_id, agent_id, path)
);

CREATE INDEX idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX idx_memory_documents_path_prefix ON memory_documents(user_id, path text_pattern_ops);
CREATE INDEX idx_memory_documents_updated ON memory_documents(updated_at DESC);

-- ==================== Workspace: Memory Chunks ====================
-- Documents are chunked for hybrid search (FTS + vector)

CREATE TABLE memory_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,

    -- Full-text search vector
    content_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED,

    -- Semantic search embedding (text-embedding-3-small = 1536 dims)
    embedding VECTOR(1536),

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_chunk_per_doc UNIQUE (document_id, chunk_index)
);

CREATE INDEX idx_memory_chunks_tsv ON memory_chunks USING GIN(content_tsv);
CREATE INDEX idx_memory_chunks_embedding ON memory_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);
CREATE INDEX idx_memory_chunks_document ON memory_chunks(document_id);

-- ==================== Workspace: Heartbeat State ====================

CREATE TABLE heartbeat_state (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id UUID,
    last_run TIMESTAMPTZ,
    next_run TIMESTAMPTZ,
    interval_seconds INT NOT NULL DEFAULT 1800,
    enabled BOOLEAN NOT NULL DEFAULT true,
    consecutive_failures INT NOT NULL DEFAULT 0,
    last_checks JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT unique_heartbeat_per_user UNIQUE (user_id, agent_id)
);

CREATE INDEX idx_heartbeat_user ON heartbeat_state(user_id);
CREATE INDEX idx_heartbeat_next_run ON heartbeat_state(next_run) WHERE enabled = true;

-- ==================== Helper Functions ====================

CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_memory_documents_updated_at
    BEFORE UPDATE ON memory_documents
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- Function to list files in a directory (prefix match)
CREATE OR REPLACE FUNCTION list_workspace_files(
    p_user_id TEXT,
    p_agent_id UUID,
    p_directory TEXT DEFAULT ''
)
RETURNS TABLE (
    path TEXT,
    is_directory BOOLEAN,
    updated_at TIMESTAMPTZ,
    content_preview TEXT
) AS $$
BEGIN
    -- Normalize directory path (ensure trailing slash for non-root)
    IF p_directory != '' AND NOT p_directory LIKE '%/' THEN
        p_directory := p_directory || '/';
    END IF;

    RETURN QUERY
    WITH files AS (
        SELECT
            d.path,
            d.updated_at,
            LEFT(d.content, 200) as content_preview,
            -- Extract the immediate child name
            CASE
                WHEN p_directory = '' THEN
                    CASE
                        WHEN position('/' in d.path) > 0
                        THEN substring(d.path from 1 for position('/' in d.path) - 1)
                        ELSE d.path
                    END
                ELSE
                    CASE
                        WHEN position('/' in substring(d.path from length(p_directory) + 1)) > 0
                        THEN substring(
                            substring(d.path from length(p_directory) + 1)
                            from 1
                            for position('/' in substring(d.path from length(p_directory) + 1)) - 1
                        )
                        ELSE substring(d.path from length(p_directory) + 1)
                    END
            END as child_name
        FROM memory_documents d
        WHERE d.user_id = p_user_id
          AND d.agent_id IS NOT DISTINCT FROM p_agent_id
          AND (p_directory = '' OR d.path LIKE p_directory || '%')
    )
    SELECT DISTINCT ON (f.child_name)
        CASE
            WHEN p_directory = '' THEN f.child_name
            ELSE p_directory || f.child_name
        END as path,
        EXISTS (
            SELECT 1 FROM memory_documents d2
            WHERE d2.user_id = p_user_id
              AND d2.agent_id IS NOT DISTINCT FROM p_agent_id
              AND d2.path LIKE
                CASE WHEN p_directory = '' THEN f.child_name ELSE p_directory || f.child_name END
                || '/%'
        ) as is_directory,
        MAX(f.updated_at) as updated_at,
        CASE
            WHEN EXISTS (
                SELECT 1 FROM memory_documents d2
                WHERE d2.user_id = p_user_id
                  AND d2.agent_id IS NOT DISTINCT FROM p_agent_id
                  AND d2.path LIKE
                    CASE WHEN p_directory = '' THEN f.child_name ELSE p_directory || f.child_name END
                    || '/%'
            ) THEN NULL
            ELSE MAX(f.content_preview)
        END as content_preview
    FROM files f
    WHERE f.child_name != '' AND f.child_name IS NOT NULL
    GROUP BY f.child_name
    ORDER BY f.child_name, is_directory DESC;
END;
$$ LANGUAGE plpgsql;

-- ==================== Views ====================

CREATE VIEW memory_documents_summary AS
SELECT
    d.id,
    d.user_id,
    d.path,
    d.created_at,
    d.updated_at,
    COUNT(c.id) as chunk_count,
    COUNT(c.embedding) as embedded_chunk_count
FROM memory_documents d
LEFT JOIN memory_chunks c ON c.document_id = d.id
GROUP BY d.id;

CREATE VIEW chunks_pending_embedding AS
SELECT
    c.id as chunk_id,
    c.document_id,
    d.user_id,
    d.path,
    LENGTH(c.content) as content_length
FROM memory_chunks c
JOIN memory_documents d ON d.id = c.document_id
WHERE c.embedding IS NULL;
