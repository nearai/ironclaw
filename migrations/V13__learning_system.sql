-- V13: Learning system tables (session search, user profiles, synthesized skills)
-- Rollback: DROP TABLE IF EXISTS synthesized_skills, user_profile_facts, session_summaries CASCADE;
-- These are new tables only — no changes to existing schema, full backward compat.

-- Session-level summaries for search
CREATE TABLE session_summaries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id UUID NOT NULL UNIQUE REFERENCES conversations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'default',
    summary TEXT NOT NULL,
    topics TEXT[] NOT NULL DEFAULT '{}',
    tool_names TEXT[] NOT NULL DEFAULT '{}',
    message_count INTEGER NOT NULL DEFAULT 0,
    search_vector tsvector GENERATED ALWAYS AS (to_tsvector('english', summary)) STORED,
    embedding vector,  -- unbounded dimension (matches V9 workspace pattern)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_summaries_user ON session_summaries(user_id, agent_id);
CREATE INDEX idx_session_summaries_created ON session_summaries(created_at DESC);
CREATE INDEX idx_session_summaries_fts ON session_summaries USING gin(search_vector);

-- User profile facts (encrypted at application layer via SecretsCrypto HKDF + AES-256-GCM)
CREATE TABLE user_profile_facts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'default',
    category TEXT NOT NULL,
    fact_key TEXT NOT NULL,
    fact_value_encrypted BYTEA NOT NULL,  -- HKDF-derived AES-256-GCM ciphertext
    key_salt BYTEA NOT NULL,              -- per-fact HKDF salt (32 bytes)
    confidence REAL NOT NULL DEFAULT 0.5,
    source TEXT NOT NULL DEFAULT 'inferred',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, agent_id, category, fact_key)
);

CREATE INDEX idx_user_profile_user ON user_profile_facts(user_id, agent_id);

-- Synthesized skill audit log
CREATE TABLE synthesized_skills (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT 'default',
    skill_name TEXT NOT NULL,
    skill_content TEXT,                   -- generated SKILL.md content (stored for approval review)
    skill_content_hash TEXT NOT NULL,
    source_conversation_id UUID REFERENCES conversations(id),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'accepted', 'rejected')),
    safety_scan_passed BOOLEAN NOT NULL DEFAULT FALSE,
    quality_score INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ
);

CREATE INDEX idx_synthesized_skills_user ON synthesized_skills(user_id, agent_id);
CREATE INDEX idx_synthesized_skills_status ON synthesized_skills(status);
CREATE UNIQUE INDEX idx_synthesized_skills_dedup ON synthesized_skills(user_id, skill_content_hash);
