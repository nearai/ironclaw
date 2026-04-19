#!/bin/sh
# entrypoint.sh — IronClaw Codex Worker startup script
# Supports two modes:
#   --mode cli       (default) run Codex as a one-shot CLI worker
#   --mode websocket           run Codex with WebSocket agent communication
set -euo pipefail

# ── helpers ────────────────────────────────────────────────────────────────────
log()  { printf '[%s] %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
die()  { log "ERROR: $*" >&2; exit 1; }

# ── defaults ───────────────────────────────────────────────────────────────────
MODE="${CODEX_MODE:-cli}"
HEALTH_PORT="${HEALTH_PORT:-8443}"
PROTOCOL_CONFIG="${PROTOCOL_CONFIG:-/app/config/agent_comm_protocol.json}"
CODEX_CONFIG="${CODEX_CONFIG:-/home/codex/.codex/config.toml}"
WS_ROLE="${WS_ROLE:-client}"
WS_STATE_FILE="${WS_STATE_FILE:-/tmp/ironclaw_ws_state.json}"
FILE_UMASK="${FILE_UMASK:-0002}"
umask "$FILE_UMASK"

# ── parse CLI args (override env vars) ────────────────────────────────────────
while [ $# -gt 0 ]; do
  case "$1" in
    --mode)      MODE="$2";        shift 2 ;;
    --port)      HEALTH_PORT="$2"; shift 2 ;;
    --protocol)  PROTOCOL_CONFIG="$2"; shift 2 ;;
    --)          shift; break ;;
    *)           break ;;
  esac
done

# ── validate config symlink ────────────────────────────────────────────────────
mkdir -p "$(dirname "$CODEX_CONFIG")"
if [ "$CODEX_CONFIG" = "/home/codex/.codex/config.toml" ]; then
  ln -sfn /app/config/codex.toml "$CODEX_CONFIG"
fi

if [ ! -f "$CODEX_CONFIG" ]; then
  log "WARN: Codex config not found at $CODEX_CONFIG — using defaults"
fi

# ── always start the health server in the background ──────────────────────────
log "Starting health server on port $HEALTH_PORT"
export WS_STATE_FILE
python3 /app/health_server.py --port "$HEALTH_PORT" &
HEALTH_PID=$!

# Give the health server a moment to bind
sleep 1

cleanup() {
  log "Shutting down..."
  kill "$HEALTH_PID" 2>/dev/null || true
  wait "$HEALTH_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# ── mode dispatch ──────────────────────────────────────────────────────────────
case "$MODE" in

  cli)
    log "Mode: CLI — running Codex with args: $*"
    exec node /app/node_modules/.bin/codex "$@"
    ;;

  websocket)
    log "Mode: WebSocket — role: $WS_ROLE"

    case "$WS_ROLE" in
      client)
        [ -f "$PROTOCOL_CONFIG" ] || die "Protocol config not found: $PROTOCOL_CONFIG"

        DEFAULT_WS_URI=$(python3 -c "
import json, sys
cfg = json.load(open('$PROTOCOL_CONFIG'))
print(cfg['connection']['uri'])
")
        WS_URI="${WS_URL:-$DEFAULT_WS_URI}"
        log "Connecting to WebSocket hub at: $WS_URI"
        export WS_URL="$WS_URI"
        export RECONNECT_MS=$(python3 -c "
import json
cfg = json.load(open('$PROTOCOL_CONFIG'))
print(cfg['connection'].get('reconnect_ms', 3000))
")
        exec python3 /app/scripts/codex_agent_client.py
        ;;
      server)
        export WS_BIND_HOST="${WS_BIND_HOST:-0.0.0.0}"
        export WS_PORT="${WS_PORT:-9090}"
        export WS_PATH="${WS_PATH:-/ws/agent}"
        log "Listening for inbound IronClaw agent connections at ws://$WS_BIND_HOST:$WS_PORT$WS_PATH"
        exec python3 /app/scripts/codex_agent_server.py
        ;;
      *)
        die "Unknown WS_ROLE '$WS_ROLE'. Use client or server"
        ;;
    esac
    ;;

  *)
    die "Unknown mode '$MODE'. Use --mode cli or --mode websocket"
    ;;

esac
