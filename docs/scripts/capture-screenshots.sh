#!/bin/bash
set -e

# Screenshot capture pipeline for IronClaw documentation
# Usage: ./docs/scripts/capture-screenshots.sh

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCS_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(dirname "$DOCS_DIR")"

# Source env config if present (from docs folder)
if [ -f "$DOCS_DIR/.env.screenshot" ]; then
  echo "Loading configuration from docs/.env.screenshot..."
  source "$DOCS_DIR/.env.screenshot"
  # Export variables so they're available to child processes
  export SCREENSHOT_PORT
  export SCREENSHOT_HOST
  export IRONCLAW_URL
  export GATEWAY_AUTH_TOKEN
  export SCREENSHOT_VIEWPORT
  export HEALTH_TIMEOUT
fi

# Load ~/.ironclaw/.env and prefer GATEWAY_AUTH_TOKEN from it
if [ -f "$HOME/.ironclaw/.env" ]; then
  echo "Loading configuration from ~/.ironclaw/.env..."
  while IFS='=' read -r key value; do
    # Skip comments and empty lines
    [[ "$key" =~ ^[[:space:]]*# ]] && continue
    [[ -z "$key" ]] && continue
    # Strip inline comments and surrounding quotes from value
    value="${value%%#*}"
    value="${value%"${value##*[![:space:]]}"}"
    value="${value#\"}" ; value="${value%\"}"
    value="${value#\'}" ; value="${value%\'}"
    if [ "$key" = "GATEWAY_AUTH_TOKEN" ]; then
      GATEWAY_AUTH_TOKEN="$value"
      export GATEWAY_AUTH_TOKEN
      echo "  Using GATEWAY_AUTH_TOKEN from ~/.ironclaw/.env"
    fi
  done < "$HOME/.ironclaw/.env"
fi

# Function to check if a port is available
is_port_available() {
  local port=$1
  if lsof -Pi :$port -sTCP:LISTEN -t > /dev/null 2>&1; then
    return 1  # Port is in use
  fi
  return 0  # Port is available
}

# Function to find an available port starting from the given port
find_available_port() {
  local start_port=$1
  local port=$start_port
  local max_port=$((start_port + 100))

  while [ $port -lt $max_port ]; do
    if is_port_available $port; then
      echo $port
      return 0
    fi
    port=$((port + 1))
  done

  return 1  # No available port found
}

# Function to check if a port returns HTTP 200 for IronClaw health endpoint
# Uses GET request to /api/health endpoint
check_port_health() {
  local port=$1
  local response
  response=$(curl -s -o /dev/null -w "%{http_code}" --max-time 3 \
    "http://127.0.0.1:$port/api/health" 2>/dev/null || echo "000")
  if [ "$response" = "200" ]; then
    return 0
  fi
  return 1
}

# Function to find all listening ports for IronClaw process
get_ironclaw_ports() {
  local pid=$1
  local ports=""

  if command -v lsof > /dev/null 2>&1; then
    # Get all TCP listening ports for the process, sorted numerically
    ports=$(lsof -Pan -p "$pid" -iTCP -sTCP:LISTEN 2>/dev/null | \
      grep LISTEN | awk -F: '{print $2}' | awk '{print $1}' | sort -n | uniq)
  elif command -v ss > /dev/null 2>&1; then
    ports=$(ss -tlnp 2>/dev/null | grep "pid=$pid" | awk '{print $4}' | \
      awk -F: '{print $NF}' | sort -n | uniq)
  elif command -v netstat > /dev/null 2>&1; then
    ports=$(netstat -tlnp 2>/dev/null | grep "$pid/" | awk '{print $4}' | \
      awk -F: '{print $NF}' | sort -n | uniq)
  fi

  echo "$ports"
}

# Function to find IronClaw HTTP API port
# Checks multiple candidate ports starting from lowest
find_ironclaw_http_port() {
  local pid=""
  local candidate_ports=""
  local port=""

  # Try to find ironclaw process by name
  if command -v pgrep > /dev/null 2>&1; then
    pid=$(pgrep -x "ironclaw" 2>/dev/null | head -1)
  fi

  # Fallback to ps if pgrep not available or no result
  if [ -z "$pid" ]; then
    pid=$(ps aux | grep -E '[i]ronclaw$|[i]ronclaw ' | awk '{print $2}' | head -1)
  fi

  if [ -z "$pid" ]; then
    return 1  # No ironclaw process found
  fi

  # Build list of candidate ports to check
  # Priority: known IronClaw ports first (3000-3010), then any ports from process
  candidate_ports="3000 3001 3002 3003 3004 3005 3006 3007 3008 3009 3010 8080 13001"

  # Add ports from process (filter out high ports likely used for WebSocket)
  local process_ports
  process_ports=$(get_ironclaw_ports "$pid" | grep -E '^[0-9]+$' | while read -r p; do
    if [ "$p" -lt 40000 ] 2>/dev/null; then
      echo "$p"
    fi
  done)

  # Merge process ports into candidate list (they'll be checked in order)
  for p in $process_ports; do
    if [[ ! " $candidate_ports " =~ " $p " ]]; then
      candidate_ports="$p $candidate_ports"
    fi
  done

  # Check each candidate port with HTTP HEAD request
  for port in $candidate_ports; do
    if check_port_health "$port"; then
      echo "$pid:$port"
      return 0
    fi
  done

  return 1  # Couldn't find HTTP port
}

# Configuration with defaults
IRONCLAW_BIN="${IRONCLAW_BIN:-$REPO_ROOT/target/release/ironclaw}"
SCREENSHOT_HOST="${SCREENSHOT_HOST:-127.0.0.1}"
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-60}"

# Check for existing IronClaw process
IRONCLAW_PID=""
EXISTING_PORT=""
MANAGE_IRONCLAW=true

echo "Checking for existing IronClaw process..."
IRONCLAW_PROC=$(find_ironclaw_http_port 2>/dev/null || true)

if [ -n "$IRONCLAW_PROC" ]; then
  IRONCLAW_PID=$(echo "$IRONCLAW_PROC" | cut -d: -f1)
  EXISTING_PORT=$(echo "$IRONCLAW_PROC" | cut -d: -f2)
  echo "Found IronClaw process (PID: $IRONCLAW_PID) HTTP API on port $EXISTING_PORT"
  SCREENSHOT_PORT=$EXISTING_PORT
  MANAGE_IRONCLAW=false
else
  echo "No existing IronClaw HTTP API found"
fi

# If no existing IronClaw, find an available port
if [ "$MANAGE_IRONCLAW" = true ]; then
  PREFERRED_PORT="${SCREENSHOT_PORT:-13001}"
  SCREENSHOT_PORT=$(find_available_port $PREFERRED_PORT)
  if [ $? -ne 0 ]; then
    echo "Error: Could not find an available port starting from $PREFERRED_PORT"
    exit 1
  fi
fi

IRONCLAW_URL="${IRONCLAW_URL:-http://$SCREENSHOT_HOST:$SCREENSHOT_PORT}"

export IRONCLAW_URL

# Prompt for auth token if not provided
if [ -z "$GATEWAY_AUTH_TOKEN" ]; then
  echo ""
  echo "No GATEWAY_AUTH_TOKEN found (checked ~/.ironclaw/.env and docs/.env.screenshot)."
  echo "Enter the auth token for the IronClaw instance (or press Enter to use default):"
  read -r GATEWAY_AUTH_TOKEN
  if [ -z "$GATEWAY_AUTH_TOKEN" ]; then
    GATEWAY_AUTH_TOKEN="screenshot-test-token"
    echo "Using default token: $GATEWAY_AUTH_TOKEN"
  fi
fi
export GATEWAY_AUTH_TOKEN
# IRONCLAW_TOKEN is what seed.ts reads; mirror GATEWAY_AUTH_TOKEN into it
export IRONCLAW_TOKEN="${IRONCLAW_TOKEN:-$GATEWAY_AUTH_TOKEN}"

export SCREENSHOT_VIEWPORT="${SCREENSHOT_VIEWPORT:-1280x800}"

echo ""
echo "========================================"
echo "IronClaw Screenshot Capture Pipeline"
echo "========================================"
echo ""
echo "Configuration:"
echo "  Binary: $IRONCLAW_BIN"
echo "  URL: $IRONCLAW_URL"
echo "  Viewport: $SCREENSHOT_VIEWPORT"
if [ "$MANAGE_IRONCLAW" = true ]; then
  echo "  Mode: Start new IronClaw instance on port $SCREENSHOT_PORT"
else
  echo "  Mode: Use existing IronClaw instance (PID: $IRONCLAW_PID, Port: $EXISTING_PORT)"
fi
echo ""

# Check for Node.js and npm
if ! command -v npm &> /dev/null; then
  echo "Error: npm is required but not installed"
  exit 1
fi

# Only build and start IronClaw if we're managing it
if [ "$MANAGE_IRONCLAW" = true ]; then
  # Build if needed (unless SKIP_BUILD is set)
  if [ ! -f "$IRONCLAW_BIN" ]; then
    if [ "${SKIP_BUILD}" = "true" ]; then
      echo "Warning: IronClaw binary not found and SKIP_BUILD is set."
      echo "  Expected: $IRONCLAW_BIN"
      echo "  Set IRONCLAW_BIN to the correct path or unset SKIP_BUILD to build."
      exit 1
    fi

    if ! command -v cargo &> /dev/null; then
      echo "Error: cargo is required to build IronClaw but not installed"
      echo "  Install Rust toolchain: https://rustup.rs/"
      echo "  Or set IRONCLAW_BIN to a pre-built binary path"
      exit 1
    fi

    echo "IronClaw binary not found. Building..."
    echo ""
    (cd "$REPO_ROOT" && cargo build --release)
    echo ""
    echo "Build complete."
    echo ""
  fi

  # Start IronClaw
  echo "Starting IronClaw on port $SCREENSHOT_PORT..."
  GATEWAY_ENABLED=true \
  GATEWAY_PORT=$SCREENSHOT_PORT \
  GATEWAY_HOST=$SCREENSHOT_HOST \
  GATEWAY_AUTH_TOKEN=$GATEWAY_AUTH_TOKEN \
  CLI_ENABLED=false \
  DATABASE_BACKEND=libsql \
  LIBSQL_PATH=/tmp/ironclaw-screenshot.db \
    "$IRONCLAW_BIN" &
  IRONCLAW_PID=$!

  # Wait for health endpoint
  echo ""
  echo "Waiting for IronClaw to be ready..."
  echo "  Health endpoint: $IRONCLAW_URL/api/health"
  echo ""

  for i in $(seq 1 $HEALTH_TIMEOUT); do
    if curl -s "$IRONCLAW_URL/api/health" > /dev/null 2>&1; then
      echo "IronClaw is ready!"
      echo ""
      break
    fi
    if [ $i -eq $HEALTH_TIMEOUT ]; then
      echo "Error: IronClaw failed to start within $HEALTH_TIMEOUT seconds"
      exit 1
    fi
    echo -n "."
    sleep 1
  done
fi

# Cleanup function
cleanup() {
  if [ "$MANAGE_IRONCLAW" = true ] && [ -n "$IRONCLAW_PID" ]; then
    echo ""
    echo "Cleaning up..."

    if kill -0 $IRONCLAW_PID 2> /dev/null; then
      echo "  Stopping IronClaw (PID: $IRONCLAW_PID)..."
      kill $IRONCLAW_PID 2> /dev/null || true
      wait $IRONCLAW_PID 2> /dev/null || true
    fi

    if [ -f /tmp/ironclaw-screenshot.db ]; then
      echo "  Removing temporary database..."
      rm -f /tmp/ironclaw-screenshot.db
      rm -f /tmp/ironclaw-screenshot.db-*
    fi

    echo "  Cleanup complete."
  fi
}

# Ensure cleanup runs on exit
trap cleanup EXIT INT TERM

# Install test dependencies
echo "Installing test dependencies..."
cd "$DOCS_DIR/tests"
pnpm install

# Install Playwright browsers if not present
if [ ! -d "node_modules/playwright-core/.local-browsers" ]; then
  echo "Installing Playwright browsers (first run)..."
  pnpm dlx playwright install chromium
fi

echo ""
echo "========================================"
echo "Running Playwright tests..."
echo "========================================"
echo ""

# Run Playwright tests
if ! pnpm exec playwright test; then
  echo ""
  echo "Error: Playwright tests failed"
  echo "Check the test output above for details"
  exit 1
fi

echo ""
echo "========================================"
echo "Generating documentation..."
echo "========================================"
echo ""

# Install generator dependencies and run
cd "$DOCS_DIR/scripts"
pnpm install
pnpm run generate

echo ""
echo "========================================"
echo "Screenshot Pipeline Complete!"
echo "========================================"
echo ""
echo "Outputs:"
echo "  Screenshots: docs/assets/screenshots/"
echo "  Documentation: docs/ui-reference/"
echo ""
echo "Review the changes before committing:"
echo "  git diff docs/"
echo "  git status docs/assets/screenshots/"
echo ""
echo "To update screenshots after UI changes:"
echo "  ./docs/scripts/capture-screenshots.sh"
echo ""
