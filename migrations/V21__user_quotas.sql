-- Per-user quota columns for fail-closed enforcement.
-- NULL = no quota assigned (denied for non-admin users).
ALTER TABLE users ADD COLUMN max_agents INTEGER DEFAULT NULL;
ALTER TABLE users ADD COLUMN max_tokens BIGINT DEFAULT NULL;
