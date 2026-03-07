-- Add wit_version column to wasm_tools for WIT interface version tracking
ALTER TABLE wasm_tools ADD COLUMN IF NOT EXISTS wit_version TEXT NOT NULL DEFAULT '0.1.0';

-- Create wasm_channels table for DB-stored channel extensions
CREATE TABLE IF NOT EXISTS wasm_channels (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '0.1.0',
    wit_version TEXT NOT NULL DEFAULT '0.1.0',
    description TEXT NOT NULL DEFAULT '',
    wasm_binary BYTEA NOT NULL,
    binary_hash BYTEA NOT NULL,
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_wasm_channel UNIQUE (user_id, name)
);
