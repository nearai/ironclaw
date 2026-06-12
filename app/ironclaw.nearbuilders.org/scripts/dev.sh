#!/bin/bash
# Local development sidecar — runs ironclaw-reborn AND everything-dev together.
#
# Prerequisites:
#   1. cp .env.example .env
#   2. bun install
#   3. Make sure the ironclaw repo is built or buildable
#
# Usage:
#   scripts/dev.sh              # Use defaults
#   IRONCLAW_DIR=../ironclaw scripts/dev.sh   # Custom path to ironclaw repo

set -eu

IRONCLAW_DIR="${IRONCLAW_DIR:-$(realpath "${BASH_SOURCE[0]}/../../../..")}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

export IRONCLAW_REBORN_SERVE_HOST="${IRONCLAW_REBORN_SERVE_HOST:-127.0.0.1}"
export IRONCLAW_REBORN_SERVE_PORT="${IRONCLAW_REBORN_SERVE_PORT:-3000}"
export IRONCLAW_REBORN_CORS_ORIGINS="${IRONCLAW_REBORN_CORS_ORIGINS:-http://localhost:3000}"

# Default token — generate a random one if not set
if [ -z "${IRONCLAW_REBORN_WEBUI_TOKEN:-}" ]; then
  export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
fi

# .env file loading
if [ -f "$PROJECT_DIR/.env" ]; then
  set -a
  source "$PROJECT_DIR/.env"
  set +a
fi

cleanup() {
  echo ""
  echo "Shutting down..."
  if [ -n "${IRONCLAW_PID:-}" ]; then
    kill "$IRONCLAW_PID" 2>/dev/null || true
  fi
  exit 0
}
trap cleanup SIGINT SIGTERM

echo "═══ ironclaw.nearbuilders.org — Development ═══"
echo ""
echo "  ironclaw port   : $IRONCLAW_REBORN_SERVE_PORT"
  echo "  everything-dev  : http://localhost:${PORT:-3000}"
echo "  CORS origins    : $IRONCLAW_REBORN_CORS_ORIGINS"
echo "  token           : $IRONCLAW_REBORN_WEBUI_TOKEN"
echo ""

# Start ironclaw-reborn in the background
echo "[1/2] Starting ironclaw-reborn..."
cd "$IRONCLAW_DIR"

cargo run -q -p ironclaw_reborn_cli \
  --features webui-v2-beta \
  --bin ironclaw-reborn -- \
  serve \
  --host "$IRONCLAW_REBORN_SERVE_HOST" \
  --port "$IRONCLAW_REBORN_SERVE_PORT" &
IRONCLAW_PID=$!

echo "[2/2] Starting everything-dev..."
cd "$PROJECT_DIR"
bun run dev

# Only reached if bun run dev exits
cleanup
