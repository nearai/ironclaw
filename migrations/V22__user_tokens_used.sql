-- Cumulative token usage counter for quota enforcement.
ALTER TABLE users ADD COLUMN tokens_used BIGINT NOT NULL DEFAULT 0;
