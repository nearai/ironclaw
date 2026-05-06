-- Query to check if routines are correctly stored in libSQL database
-- Usage: sqlite3 ~/.ironclaw/ironclaw.db < check_routines.sql

-- Check all routines
SELECT
    id,
    name,
    user_id,
    enabled,
    trigger_type,
    action_type,
    notify_channel,
    notify_user,
    last_run_at,
    next_fire_at,
    run_count,
    consecutive_failures,
    created_at,
    updated_at
FROM routines
ORDER BY created_at DESC;

-- Check routine runs with their status
SELECT
    rr.id,
    r.name as routine_name,
    rr.trigger_type,
    rr.status,
    rr.started_at,
    rr.completed_at,
    rr.result_summary,
    rr.tokens_used,
    rr.job_id
FROM routine_runs rr
JOIN routines r ON rr.routine_id = r.id
ORDER BY rr.started_at DESC
LIMIT 20;

-- Count routines by status
SELECT
    enabled,
    trigger_type,
    action_type,
    COUNT(*) as count
FROM routines
GROUP BY enabled, trigger_type, action_type;

-- Check for any routines with consecutive failures
SELECT
    id,
    name,
    user_id,
    consecutive_failures,
    last_run_at,
    next_fire_at
FROM routines
WHERE consecutive_failures > 0
ORDER BY consecutive_failures DESC;

-- Check cron routines that should fire
SELECT
    id,
    name,
    next_fire_at,
    enabled,
    json_extract(trigger_config, '$.schedule') as schedule
FROM routines
WHERE trigger_type = 'cron' AND enabled = 1
ORDER BY next_fire_at ASC;
