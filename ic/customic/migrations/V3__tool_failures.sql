-- Track tool execution failures for self-repair
-- Tools that fail repeatedly can be automatically repaired by the builder

CREATE TABLE IF NOT EXISTS tool_failures (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tool_name VARCHAR(255) NOT NULL,
    error_message TEXT,
    error_count INTEGER DEFAULT 1,
    first_failure TIMESTAMPTZ DEFAULT NOW(),
    last_failure TIMESTAMPTZ DEFAULT NOW(),
    -- Store BuildResult for repair context
    last_build_result JSONB,
    repaired_at TIMESTAMPTZ,
    repair_attempts INTEGER DEFAULT 0,
    UNIQUE(tool_name)
);

CREATE INDEX idx_tool_failures_name ON tool_failures(tool_name);
CREATE INDEX idx_tool_failures_count ON tool_failures(error_count DESC);
CREATE INDEX idx_tool_failures_unrepaired ON tool_failures(tool_name) WHERE repaired_at IS NULL;
