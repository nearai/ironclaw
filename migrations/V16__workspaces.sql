-- Workspace entities with membership and shared data scoping.

CREATE TABLE IF NOT EXISTS workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by TEXT NOT NULL REFERENCES users(id),
    settings JSONB NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_workspaces_status ON workspaces(status);

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    joined_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invited_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_members_user ON workspace_members(user_id);

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_conversations_workspace ON conversations(workspace_id);

DROP INDEX IF EXISTS uq_conv_routine;
DROP INDEX IF EXISTS uq_conv_heartbeat;

CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_routine_personal
ON conversations (user_id, (metadata->>'routine_id'))
WHERE workspace_id IS NULL AND metadata ? 'routine_id';

CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_routine_workspace
ON conversations (workspace_id, (metadata->>'routine_id'))
WHERE workspace_id IS NOT NULL AND metadata ? 'routine_id';

CREATE UNIQUE INDEX IF NOT EXISTS uq_conv_heartbeat_personal
ON conversations (user_id)
WHERE workspace_id IS NULL AND metadata->>'thread_type' = 'heartbeat';

ALTER TABLE agent_jobs
    ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS idx_agent_jobs_workspace ON agent_jobs(workspace_id);

ALTER TABLE memory_documents
    ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE;

ALTER TABLE memory_documents
    DROP CONSTRAINT IF EXISTS unique_path_per_user;

CREATE UNIQUE INDEX IF NOT EXISTS uq_memory_documents_personal
ON memory_documents (user_id, agent_id, path)
WHERE workspace_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_memory_documents_workspace
ON memory_documents (workspace_id, agent_id, path)
WHERE workspace_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memory_documents_workspace
ON memory_documents(workspace_id);

CREATE INDEX IF NOT EXISTS idx_memory_documents_workspace_path
ON memory_documents(workspace_id, path)
WHERE workspace_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memory_documents_workspace_path_prefix
ON memory_documents(workspace_id, path text_pattern_ops)
WHERE workspace_id IS NOT NULL;

ALTER TABLE routines
    ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE;

ALTER TABLE routines
    DROP CONSTRAINT IF EXISTS routines_user_id_name_key;

CREATE UNIQUE INDEX IF NOT EXISTS uq_routines_personal_name
ON routines (user_id, name)
WHERE workspace_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_routines_workspace_name
ON routines (workspace_id, name)
WHERE workspace_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_routines_workspace ON routines(workspace_id);

DROP INDEX IF EXISTS idx_routines_event_triggers;

CREATE INDEX IF NOT EXISTS idx_routines_event_triggers_personal
ON routines (user_id)
WHERE enabled AND trigger_type = 'event' AND workspace_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_routines_event_triggers_workspace
ON routines (workspace_id)
WHERE enabled AND trigger_type = 'event' AND workspace_id IS NOT NULL;

ALTER TABLE settings
    ADD COLUMN IF NOT EXISTS workspace_id UUID REFERENCES workspaces(id) ON DELETE CASCADE;

ALTER TABLE settings
    DROP CONSTRAINT IF EXISTS settings_pkey;

CREATE UNIQUE INDEX IF NOT EXISTS uq_settings_personal
ON settings (user_id, key)
WHERE workspace_id IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_settings_workspace
ON settings (workspace_id, key)
WHERE workspace_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_settings_workspace ON settings(workspace_id);
