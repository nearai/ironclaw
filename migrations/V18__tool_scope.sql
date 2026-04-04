-- Add scope column to tool tables for future admin-promoted global tools.
-- All existing rows default to 'user'. Nothing sets 'global' yet.
ALTER TABLE wasm_tools    ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'user';
ALTER TABLE dynamic_tools ADD COLUMN IF NOT EXISTS scope TEXT NOT NULL DEFAULT 'user';
