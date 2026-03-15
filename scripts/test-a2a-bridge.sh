#!/usr/bin/env bash
# Test script for the A2A bridge tool.
#
# Usage:
#   # Run unit + integration tests (no external server needed)
#   ./scripts/test-a2a-bridge.sh
#
#   # Run live E2E test against a real A2A agent
#   A2A_AGENT_URL=http://your-agent:5085 \
#   A2A_ASSISTANT_ID=your-assistant-id \
#     ./scripts/test-a2a-bridge.sh --live
#
# Environment variables (for --live mode):
#   A2A_AGENT_URL       Base URL of the A2A-compatible agent server (required)
#   A2A_ASSISTANT_ID    Assistant/graph ID to query (required)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}✓ $1${NC}"; }
fail() { echo -e "${RED}✗ $1${NC}"; exit 1; }
info() { echo -e "${YELLOW}► $1${NC}"; }

LIVE=false
for arg in "$@"; do
  case "$arg" in
    --live) LIVE=true ;;
  esac
done

# ── Step 1: Format check ────────────────────────────────────────────
info "Checking formatting..."
cargo fmt --check -- src/tools/builtin/a2a/*.rs src/config/a2a.rs \
  2>/dev/null && pass "cargo fmt" || fail "cargo fmt"

# ── Step 2: Clippy ──────────────────────────────────────────────────
info "Running clippy on A2A modules..."
cargo clippy -p ironclaw --all-features -- -D warnings \
  2>&1 | tail -3
pass "cargo clippy"

# ── Step 3: Unit tests ──────────────────────────────────────────────
info "Running A2A unit tests..."
cargo test --lib -- a2a 2>&1 | tail -5
pass "unit tests"

# ── Step 4: Integration tests (construction only) ───────────────────
info "Running A2A integration tests (construction)..."
cargo test --test a2a_bridge_integration 2>&1 | tail -5
pass "integration tests"

# ── Step 5: Feature-flag compilation ────────────────────────────────
info "Checking libsql feature compilation..."
cargo check --no-default-features --features libsql 2>&1 | tail -3
pass "libsql feature check"

# ── Step 6 (optional): Live E2E test ────────────────────────────────
if [ "$LIVE" = true ]; then
  if [ -z "${A2A_AGENT_URL:-}" ] || [ -z "${A2A_ASSISTANT_ID:-}" ]; then
    fail "Live test requires A2A_AGENT_URL and A2A_ASSISTANT_ID env vars"
  fi

  info "Running live A2A test against $A2A_AGENT_URL ..."

  # Quick connectivity check
  HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    --connect-timeout 5 "$A2A_AGENT_URL/info" 2>/dev/null || echo "000")
  if [ "$HTTP_CODE" = "000" ]; then
    fail "Cannot reach $A2A_AGENT_URL (connection refused or timeout)"
  fi
  pass "server reachable (HTTP $HTTP_CODE)"

  # Run the ignored live test
  A2A_AGENT_URL="$A2A_AGENT_URL" \
  A2A_ASSISTANT_ID="$A2A_ASSISTANT_ID" \
    cargo test --test a2a_bridge_integration -- --ignored 2>&1 | tail -5
  pass "live E2E test"
fi

echo ""
echo -e "${GREEN}All A2A bridge tests passed.${NC}"
