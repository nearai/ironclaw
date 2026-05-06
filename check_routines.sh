#!/bin/bash
# Script to check routines in IronClaw libSQL database

# Find the database file
DB_PATH="${LIBSQL_PATH:-$HOME/.ironclaw/ironclaw.db}"

if [ ! -f "$DB_PATH" ]; then
    echo "Database not found at: $DB_PATH"
    echo "Checking for database in ~/.ironclaw..."
    if [ -d "$HOME/.ironclaw" ]; then
        echo "Found files in ~/.ironclaw:"
        ls -la "$HOME/.ironclaw"/*.db 2>/dev/null || echo "No .db files found"
    fi
    exit 1
fi

echo "Checking routines in: $DB_PATH"
echo "=================================="
echo ""

# Check all routines
echo "1. All routines:"
sqlite3 "$DB_PATH" "SELECT id, name, user_id, enabled, trigger_type, action_type FROM routines ORDER BY created_at DESC;"

echo ""
echo "2. Routine runs (last 10):"
sqlite3 "$DB_PATH" "SELECT rr.id, substr(r.name, 1, 30) as routine_name, rr.status, rr.started_at FROM routine_runs rr JOIN routines r ON rr.routine_id = r.id ORDER BY rr.started_at DESC LIMIT 10;"

echo ""
echo "3. Routines by status:"
sqlite3 "$DB_PATH" "SELECT enabled, trigger_type, COUNT(*) FROM routines GROUP BY enabled, trigger_type;"

echo ""
echo "4. Routines with failures:"
sqlite3 "$DB_PATH" "SELECT name, consecutive_failures, last_run_at FROM routines WHERE consecutive_failures > 0;"

echo ""
echo "5. Due cron routines:"
sqlite3 "$DB_PATH" "SELECT name, next_fire_at FROM routines WHERE trigger_type = 'cron' AND enabled = 1 AND datetime(next_fire_at) <= datetime('now') ORDER BY next_fire_at;"
