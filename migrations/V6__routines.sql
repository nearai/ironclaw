-- Routines: scheduled and reactive job system.
--
-- A routine is a named, persistent, user-owned task with a trigger and an action.
-- Triggers fire independently (cron, event, webhook, manual) so only the
-- relevant routine's prompt hits the LLM, not the whole checklist.

CREATE TABLE routines (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,

    -- Trigger definition
    trigger_type TEXT NOT NULL,          -- 'cron', 'event', 'webhook', 'manual'
    trigger_config JSONB NOT NULL,       -- type-specific config (schedule, pattern, etc.)

    -- Action definition
    action_type TEXT NOT NULL,           -- 'lightweight', 'full_job'
    action_config JSONB NOT NULL,        -- prompt, context_paths, max_tokens / title, max_iterations

    -- Guardrails
    cooldown_secs INTEGER NOT NULL DEFAULT 300,
    max_concurrent INTEGER NOT NULL DEFAULT 1,
    dedup_window_secs INTEGER,           -- NULL = no dedup

    -- Notification preferences
    notify_channel TEXT,                 -- NULL = use default
    notify_user TEXT NOT NULL DEFAULT 'default',
    notify_on_success BOOLEAN NOT NULL DEFAULT false,
    notify_on_failure BOOLEAN NOT NULL DEFAULT true,
    notify_on_attention BOOLEAN NOT NULL DEFAULT true,

    -- Runtime state (updated by engine)
    state JSONB NOT NULL DEFAULT '{}',
    last_run_at TIMESTAMPTZ,
    next_fire_at TIMESTAMPTZ,            -- pre-computed for cron triggers
    run_count BIGINT NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (user_id, name)
);

-- Fast lookup: "which cron routines need to fire right now?"
CREATE INDEX idx_routines_next_fire
    ON routines (next_fire_at)
    WHERE enabled AND next_fire_at IS NOT NULL;

-- Fast lookup: event triggers for a user
CREATE INDEX idx_routines_event_triggers
    ON routines (user_id)
    WHERE enabled AND trigger_type = 'event';

-- Audit log of individual routine executions.
CREATE TABLE routine_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    routine_id UUID NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    trigger_type TEXT NOT NULL,
    trigger_detail TEXT,                  -- e.g. matched message preview, cron expression
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'running',  -- running, ok, attention, failed
    result_summary TEXT,
    tokens_used INTEGER,
    job_id UUID REFERENCES agent_jobs(id),   -- non-NULL for full_job runs
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_routine_runs_routine ON routine_runs (routine_id);
CREATE INDEX idx_routine_runs_status ON routine_runs (status) WHERE status = 'running';
