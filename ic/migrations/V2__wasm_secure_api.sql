-- WASM Secure API Extension
-- V2: Secrets management, WASM tool storage, capabilities, and leak detection

-- ==================== Secrets ====================
-- Encrypted secret storage for credential injection into WASM HTTP requests.
-- WASM tools NEVER see plaintext secrets; injection happens at host boundary.

CREATE TABLE secrets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,

    -- AES-256-GCM encrypted value (nonce || ciphertext || tag)
    encrypted_value BYTEA NOT NULL,
    -- Per-secret key derivation salt (for HKDF)
    key_salt BYTEA NOT NULL,

    -- Optional metadata
    provider TEXT,  -- e.g., "openai", "anthropic", "stripe"
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ,
    usage_count BIGINT NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_secret_per_user UNIQUE (user_id, name)
);

CREATE INDEX idx_secrets_user ON secrets(user_id);
CREATE INDEX idx_secrets_provider ON secrets(provider) WHERE provider IS NOT NULL;
CREATE INDEX idx_secrets_expires ON secrets(expires_at) WHERE expires_at IS NOT NULL;

-- Trigger to update updated_at
CREATE TRIGGER update_secrets_updated_at
    BEFORE UPDATE ON secrets
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ==================== WASM Tools ====================
-- Store compiled WASM binaries with integrity verification.

CREATE TABLE wasm_tools (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',

    description TEXT NOT NULL,
    wasm_binary BYTEA NOT NULL,
    -- BLAKE3 hash for integrity verification on load
    binary_hash BYTEA NOT NULL,
    parameters_schema JSONB NOT NULL,

    -- Provenance
    source_url TEXT,
    -- Trust levels: 'system' (built-in), 'verified' (audited), 'user' (untrusted)
    trust_level TEXT NOT NULL DEFAULT 'user',

    -- Status: 'active', 'disabled', 'quarantined'
    status TEXT NOT NULL DEFAULT 'active',

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_wasm_tool_version UNIQUE (user_id, name, version)
);

CREATE INDEX idx_wasm_tools_user ON wasm_tools(user_id);
CREATE INDEX idx_wasm_tools_name ON wasm_tools(user_id, name);
CREATE INDEX idx_wasm_tools_status ON wasm_tools(status);
CREATE INDEX idx_wasm_tools_trust ON wasm_tools(trust_level);

CREATE TRIGGER update_wasm_tools_updated_at
    BEFORE UPDATE ON wasm_tools
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ==================== Tool Capabilities ====================
-- Fine-grained capability configuration per WASM tool.
-- Follows principle of least privilege.

CREATE TABLE tool_capabilities (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wasm_tool_id UUID NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,

    -- HTTP capability: allowed endpoint patterns
    -- Each pattern is: {"host": "api.example.com", "path_prefix": "/v1/", "methods": ["GET", "POST"]}
    http_allowlist JSONB NOT NULL DEFAULT '[]',

    -- Secrets this tool can use (injected at host boundary)
    -- Tool never sees the actual secret values
    allowed_secrets TEXT[] NOT NULL DEFAULT '{}',

    -- Tool invocation aliases (indirection layer)
    -- Maps alias name to real tool name, e.g., {"search": "brave_search"}
    tool_aliases JSONB NOT NULL DEFAULT '{}',

    -- Rate limiting
    requests_per_minute INT NOT NULL DEFAULT 60,
    requests_per_hour INT NOT NULL DEFAULT 1000,

    -- Request/response size limits
    max_request_body_bytes BIGINT NOT NULL DEFAULT 1048576,   -- 1 MB
    max_response_body_bytes BIGINT NOT NULL DEFAULT 10485760, -- 10 MB

    -- Workspace access (path prefixes tool can read)
    workspace_read_prefixes TEXT[] NOT NULL DEFAULT '{}',

    -- Timeout for HTTP requests (seconds)
    http_timeout_secs INT NOT NULL DEFAULT 30,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_capabilities_per_tool UNIQUE (wasm_tool_id)
);

CREATE INDEX idx_tool_capabilities_tool ON tool_capabilities(wasm_tool_id);

CREATE TRIGGER update_tool_capabilities_updated_at
    BEFORE UPDATE ON tool_capabilities
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- ==================== Leak Detection Patterns ====================
-- Patterns for detecting secret leakage in tool outputs.
-- Scanned before returning data to WASM or LLM.

CREATE TABLE leak_detection_patterns (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL UNIQUE,

    -- Regex pattern for detection
    pattern TEXT NOT NULL,

    -- Severity: 'critical', 'high', 'medium', 'low'
    severity TEXT NOT NULL DEFAULT 'high',

    -- Action: 'block' (fail request), 'redact' (mask secret), 'warn' (log only)
    action TEXT NOT NULL DEFAULT 'block',

    enabled BOOLEAN NOT NULL DEFAULT true,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_leak_patterns_enabled ON leak_detection_patterns(enabled) WHERE enabled = true;

-- Pre-populate with common API key patterns
INSERT INTO leak_detection_patterns (name, pattern, severity, action) VALUES
    -- OpenAI (sk-proj-... or sk-... followed by alphanumeric)
    ('openai_api_key', 'sk-(?:proj-)?[a-zA-Z0-9]{20,}(?:T3BlbkFJ[a-zA-Z0-9_-]*)?', 'critical', 'block'),

    -- Anthropic (sk-ant-api followed by 90+ chars)
    ('anthropic_api_key', 'sk-ant-api[a-zA-Z0-9_-]{90,}', 'critical', 'block'),

    -- AWS Access Key ID (starts with AKIA)
    ('aws_access_key', 'AKIA[0-9A-Z]{16}', 'critical', 'block'),

    -- AWS Secret Access Key (40 char base64-ish)
    ('aws_secret_key', '(?<![A-Za-z0-9/+=])[A-Za-z0-9/+=]{40}(?![A-Za-z0-9/+=])', 'high', 'block'),

    -- GitHub tokens (gh[pousr]_...)
    ('github_token', 'gh[pousr]_[A-Za-z0-9_]{36,}', 'critical', 'block'),

    -- GitHub fine-grained PAT
    ('github_fine_grained_pat', 'github_pat_[a-zA-Z0-9]{22}_[a-zA-Z0-9]{59}', 'critical', 'block'),

    -- Stripe keys (sk_live_... or sk_test_...)
    ('stripe_api_key', 'sk_(?:live|test)_[a-zA-Z0-9]{24,}', 'critical', 'block'),

    -- NEAR AI session tokens
    ('nearai_session', 'sess_[a-zA-Z0-9]{32,}', 'critical', 'block'),

    -- Generic Bearer tokens in headers
    ('bearer_token', 'Bearer\s+[a-zA-Z0-9_-]{20,}', 'high', 'redact'),

    -- PEM private keys
    ('pem_private_key', '-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----', 'critical', 'block'),

    -- SSH private keys
    ('ssh_private_key', '-----BEGIN\s+(?:OPENSSH|EC|DSA)\s+PRIVATE\s+KEY-----', 'critical', 'block'),

    -- Google API keys
    ('google_api_key', 'AIza[0-9A-Za-z_-]{35}', 'high', 'block'),

    -- Slack tokens
    ('slack_token', 'xox[baprs]-[0-9a-zA-Z-]{10,}', 'high', 'block'),

    -- Discord tokens
    ('discord_token', '[MN][A-Za-z\d]{23,}\.[\w-]{6}\.[\w-]{27}', 'high', 'block'),

    -- Twilio (starts with SK)
    ('twilio_api_key', 'SK[a-fA-F0-9]{32}', 'high', 'block'),

    -- SendGrid
    ('sendgrid_api_key', 'SG\.[a-zA-Z0-9_-]{22}\.[a-zA-Z0-9_-]{43}', 'high', 'block'),

    -- Mailchimp
    ('mailchimp_api_key', '[a-f0-9]{32}-us[0-9]{1,2}', 'medium', 'block'),

    -- Generic high-entropy strings (potential secrets) - careful with false positives
    ('high_entropy_hex', '(?<![a-fA-F0-9])[a-fA-F0-9]{64}(?![a-fA-F0-9])', 'medium', 'warn');

-- ==================== Rate Limit State ====================
-- Track rate limit consumption per tool per user.

CREATE TABLE tool_rate_limit_state (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wasm_tool_id UUID NOT NULL REFERENCES wasm_tools(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,

    -- Sliding window counters
    minute_window_start TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    minute_count INT NOT NULL DEFAULT 0,
    hour_window_start TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hour_count INT NOT NULL DEFAULT 0,

    CONSTRAINT unique_rate_limit_per_tool_user UNIQUE (wasm_tool_id, user_id)
);

CREATE INDEX idx_rate_limit_tool ON tool_rate_limit_state(wasm_tool_id);
CREATE INDEX idx_rate_limit_user ON tool_rate_limit_state(user_id);

-- ==================== Secret Usage Audit Log ====================
-- Audit trail for secret access (credential injection events).

CREATE TABLE secret_usage_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    secret_id UUID NOT NULL REFERENCES secrets(id) ON DELETE CASCADE,
    wasm_tool_id UUID REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,

    -- What endpoint was the secret injected for
    target_host TEXT NOT NULL,
    target_path TEXT,

    -- Result of the operation
    success BOOLEAN NOT NULL,
    error_message TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_secret_usage_secret ON secret_usage_log(secret_id);
CREATE INDEX idx_secret_usage_tool ON secret_usage_log(wasm_tool_id);
CREATE INDEX idx_secret_usage_user ON secret_usage_log(user_id);
CREATE INDEX idx_secret_usage_created ON secret_usage_log(created_at DESC);

-- Partition by month for large deployments (optional, commented out)
-- CREATE TABLE secret_usage_log_y2024m01 PARTITION OF secret_usage_log
--     FOR VALUES FROM ('2024-01-01') TO ('2024-02-01');

-- ==================== Leak Detection Events ====================
-- Log when potential secret leaks are detected and blocked.

CREATE TABLE leak_detection_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pattern_id UUID REFERENCES leak_detection_patterns(id) ON DELETE SET NULL,
    wasm_tool_id UUID REFERENCES wasm_tools(id) ON DELETE SET NULL,
    user_id TEXT NOT NULL,

    -- Where the leak was detected
    source TEXT NOT NULL,  -- 'http_response', 'tool_output', 'log_message'
    action_taken TEXT NOT NULL,  -- 'blocked', 'redacted', 'warned'

    -- Redacted context (no actual secrets stored)
    context_preview TEXT,  -- First 100 chars with secret masked

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_leak_events_pattern ON leak_detection_events(pattern_id);
CREATE INDEX idx_leak_events_tool ON leak_detection_events(wasm_tool_id);
CREATE INDEX idx_leak_events_user ON leak_detection_events(user_id);
CREATE INDEX idx_leak_events_created ON leak_detection_events(created_at DESC);

-- ==================== Views ====================

-- View: Tools with their capabilities
CREATE VIEW wasm_tools_with_capabilities AS
SELECT
    t.id,
    t.user_id,
    t.name,
    t.version,
    t.description,
    t.trust_level,
    t.status,
    t.created_at,
    t.updated_at,
    c.http_allowlist,
    c.allowed_secrets,
    c.tool_aliases,
    c.requests_per_minute,
    c.requests_per_hour,
    c.workspace_read_prefixes
FROM wasm_tools t
LEFT JOIN tool_capabilities c ON c.wasm_tool_id = t.id;

-- View: Active leak detection patterns
CREATE VIEW active_leak_patterns AS
SELECT id, name, pattern, severity, action
FROM leak_detection_patterns
WHERE enabled = true;

-- View: Recent leak events summary
CREATE VIEW recent_leak_events AS
SELECT
    le.created_at,
    le.source,
    le.action_taken,
    lp.name as pattern_name,
    lp.severity,
    wt.name as tool_name,
    le.user_id
FROM leak_detection_events le
LEFT JOIN leak_detection_patterns lp ON lp.id = le.pattern_id
LEFT JOIN wasm_tools wt ON wt.id = le.wasm_tool_id
WHERE le.created_at > NOW() - INTERVAL '24 hours'
ORDER BY le.created_at DESC;
