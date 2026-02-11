-- Settings table: key-value store for all user configuration.
--
-- Replaces ~/.ironclaw/settings.json, session.json, and mcp-servers.json.
-- Keys use dotted paths matching the existing Settings.get()/set() convention
-- (e.g., "agent.name", "sandbox.enabled", "mcp_servers").
-- One row per setting so individual values can be updated atomically.

CREATE TABLE IF NOT EXISTS settings (
    user_id    TEXT        NOT NULL,
    key        TEXT        NOT NULL,
    value      JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_settings_user ON settings (user_id);
