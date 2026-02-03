-- NEAR Agent Database Schema
-- V1: Initial schema

-- Conversations from various channels
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

-- Messages in conversations
CREATE TABLE conversation_messages (
    id UUID PRIMARY KEY,
    conversation_id UUID NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_conversation_messages_conversation ON conversation_messages(conversation_id);

-- Jobs we've worked on
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

-- Actions taken during job execution (event sourcing)
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

-- Dynamic tools built by the agent
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

-- LLM calls for cost tracking
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

-- Estimation history for continuous learning
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

-- Self-repair history
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
