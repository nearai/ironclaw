#!/usr/bin/env bash
# Launch IronClaw Reborn with the everything-dev BOS toolchain.
#
#   --local           starts the everything-dev dev stack locally.
#   --tunnel          starts Reborn locally + exposes via tunnel for the production UI.
#   <account>/<gateway>  starts a production-like host against a remote gateway.
#
# Usage:
#   scripts/bos-dev.sh --local                                # local everything-dev stack
#   scripts/bos-dev.sh --tunnel                               # Reborn + tunnel
#   scripts/bos-dev.sh work.efiz.near/ironclaw.everything.dev  # remote gateway
#
# Before running, export your provider's API key, e.g.:
#   export NEARAI_API_KEY=...      # or OPENAI_API_KEY / ANTHROPIC_API_KEY
#
# Overridable via environment:
#   PROVIDER      provider id        (default: nearai)
#   MODEL         model id           (default: $NEARAI_MODEL)
#   REBORN_HOST   listen host        (default: 127.0.0.1)
#   REBORN_PORT   listen port        (default: 3001)
#   IRONCLAW_REBORN_HOME             (default: $HOME/.ironclaw-reborn-demo)
#   IRONCLAW_REBORN_WEBUI_USER_ID    (default: home's [identity].default_owner)
#   IRONCLAW_REBORN_WEBUI_TOKEN      (default: auto-generated random token)
#   NEARAI_MODEL                     (default: deepseek-ai/DeepSeek-V4-Flash)
#   NEARAI_BASE_URL                  (default: https://cloud-api.near.ai)
#   IRONCLAW_REBORN_PROFILE          (default: local-dev-yolo)
#   IRONCLAW_REBORN_LOG              (default: info)
#   IRONCLAW_TRIGGER_POLLER_ENABLED  (default: true)

if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

set -euo pipefail

PROVIDER="${PROVIDER:-nearai}"
MODEL="${MODEL:-}"
REBORN_HOST="${REBORN_HOST:-127.0.0.1}"
REBORN_PORT="${REBORN_PORT:-3001}"
NEARAI_MODEL="${NEARAI_MODEL:-deepseek-ai/DeepSeek-V4-Flash}"
NEARAI_BASE_URL="${NEARAI_BASE_URL:-https://cloud-api.near.ai}"
IRONCLAW_REBORN_PROFILE="${IRONCLAW_REBORN_PROFILE:-local-dev-yolo}"
IRONCLAW_REBORN_LOG="${IRONCLAW_REBORN_LOG:-info}"
IRONCLAW_TRIGGER_POLLER_ENABLED="${IRONCLAW_TRIGGER_POLLER_ENABLED:-true}"

ARG="${1:-}"
if [ "$ARG" = "--local" ]; then
  MODE="local"
elif [ "$ARG" = "--tunnel" ]; then
  MODE="tunnel"
elif [[ "$ARG" =~ ^[a-z0-9_.-]+/[a-z0-9_.-]+$ ]]; then
  MODE="remote"
  ACCOUNT="${ARG%%/*}"
  DOMAIN="${ARG#*/}"
else
  echo "Usage: $0 [--local | --tunnel | <account>/<gateway>]" >&2
  echo "" >&2
  echo "Examples:" >&2
  echo "  $0 --local                                           # Local everything-dev stack" >&2
  echo "  $0 --tunnel                                          # Reborn + tunnel" >&2
  echo "  $0 work.efiz.near/ironclaw.everything.dev            # Remote gateway" >&2
  exit 1
fi

# Kill any existing process on our port (cross-platform: lsof on macOS/Linux, skip on Windows)
stale_pid=""
if command -v lsof &>/dev/null; then
  stale_pid="$(lsof -ti "tcp:$REBORN_PORT" 2>/dev/null || true)"
elif command -v netstat &>/dev/null && command -v grep &>/dev/null; then
  stale_pid="$(netstat -ano 2>/dev/null | grep ":$REBORN_PORT " | grep LISTENING | awk '{print $NF}' | head -1 || true)"
fi
if [ -n "$stale_pid" ]; then
  echo "==> Killing stale process on port $REBORN_PORT (PID: $stale_pid)"
  kill "$stale_pid" 2>/dev/null || true
  sleep 1
fi

REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

export IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME:-$HOME/.ironclaw-reborn-demo}"

case "$IRONCLAW_REBORN_HOME" in
  /*) home_abs="$IRONCLAW_REBORN_HOME" ;;
  *)  home_abs="$PWD/$IRONCLAW_REBORN_HOME" ;;
esac
home_parent="$(cd "$(dirname "$home_abs")" 2>/dev/null && pwd -P || true)"
repo_canonical="$(cd "$REPO_ROOT" && pwd -P)"
if [ -n "$home_parent" ]; then
  home_canonical="$home_parent/$(basename "$home_abs")"
  case "$home_canonical/" in
    "$repo_canonical"/*)
      echo "error: IRONCLAW_REBORN_HOME ($home_canonical) is inside the repo ($repo_canonical)." >&2
      echo "       serve uses the cwd as the workspace root and rejects overlap." >&2
      echo "       Point it somewhere else, e.g. \$HOME/.ironclaw-reborn-demo." >&2
      exit 1
      ;;
  esac
fi

export IRONCLAW_UNSAFE_RAW_HTTP_EGRESS_ERRORS=1
export IRONCLAW_REBORN_LOG
export IRONCLAW_REBORN_PROFILE
export IRONCLAW_TRIGGER_POLLER_ENABLED
export NEARAI_BASE_URL
export NEARAI_MODEL

if [ -z "${IRONCLAW_REBORN_WEBUI_TOKEN:-}" ]; then
  if command -v openssl &>/dev/null; then
    IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
  else
    IRONCLAW_REBORN_WEBUI_TOKEN="local-dev-token"
  fi
fi
export IRONCLAW_REBORN_WEBUI_TOKEN

key_env=""
if [ "$PROVIDER" = "nearai" ]; then
  key_env="NEARAI_API_KEY"
elif [ "$PROVIDER" = "openai" ]; then
  key_env="OPENAI_API_KEY"
elif [ "$PROVIDER" = "anthropic" ]; then
  key_env="ANTHROPIC_API_KEY"
fi

if [ -n "$key_env" ] && [ -z "${!key_env:-}" ]; then
  echo "==> $key_env is not set."
  read -r -p "    Enter your $PROVIDER API key (or press Enter to skip and set it later): " api_key
  if [ -n "$api_key" ]; then
    export "$key_env=$api_key"
  else
    echo "    error: $key_env is required for the $PROVIDER model provider." >&2
    echo "           Export it beforehand or re-run and enter it." >&2
    exit 1
  fi
fi

CARGO=(cargo run -p ironclaw_reborn_cli --features webui-v2-beta --)

set_provider_args=(models set-provider "$PROVIDER")
if [ -n "$MODEL" ]; then
  set_provider_args+=(--model "$MODEL")
fi
echo "==> Configuring model route: provider=$PROVIDER ${MODEL:+model=$MODEL}"
"${CARGO[@]}" "${set_provider_args[@]}"

config_file="$IRONCLAW_REBORN_HOME/config.toml"
config_owner=""
if [ -f "$config_file" ]; then
  config_owner="$(sed -n 's/^[[:space:]]*default_owner[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$config_file" | head -1)"
fi
export IRONCLAW_REBORN_WEBUI_USER_ID="${IRONCLAW_REBORN_WEBUI_USER_ID:-${config_owner:-reborn-cli}}"

key_env="$("${CARGO[@]}" models status 2>/dev/null \
  | sed -n 's/^default\.api_key_env: //p' || true)"
if [ -n "$key_env" ] && [ -z "${!key_env:-}" ]; then
  echo "warning: $key_env is not set. Required-key providers (openai, anthropic, …)" >&2
  echo "         fail at startup; export it before turns will work." >&2
fi

cleanup() {
  echo ""
  echo "Shutting down..."
  if [ -n "${TAIL_PID:-}" ]; then
    kill "$TAIL_PID" 2>/dev/null || true
  fi
  if [ -n "${TUNNEL_PID:-}" ]; then
    kill "$TUNNEL_PID" 2>/dev/null || true
  fi
  if [ -n "${REBORN_PID:-}" ]; then
    kill "$REBORN_PID" 2>/dev/null || true
  fi
  rm -f "${REBORN_LOG:-}" "${TUNNEL_LOG:-}" 2>/dev/null || true
  exit 0
}
trap cleanup SIGINT SIGTERM

# ─────────────────────────────────────────────────────────────────────
# Local mode — start Reborn + everything-dev dev stack on localhost
# ─────────────────────────────────────────────────────────────────────
if [ "$MODE" = "local" ]; then
  EVDEV_DIR="$REPO_ROOT/app/ironclaw.everything.dev"

  if [ ! -d "$EVDEV_DIR" ]; then
    echo "error: everything-dev project not found at $EVDEV_DIR" >&2
    exit 1
  fi

  if [ ! -f "$EVDEV_DIR/.env" ]; then
    echo "==> $EVDEV_DIR/.env not found — creating from .env.example"
    cp "$EVDEV_DIR/.env.example" "$EVDEV_DIR/.env"
    echo "    info: filling required secrets." >&2
  fi

  if grep -q '^BETTER_AUTH_SECRET=$' "$EVDEV_DIR/.env" 2>/dev/null; then
    secret="$(openssl rand -base64 32)"
    if [[ "$OSTYPE" == "darwin"* ]]; then
      sed -i '' "s/^BETTER_AUTH_SECRET=$/BETTER_AUTH_SECRET=$secret/" "$EVDEV_DIR/.env"
    else
      sed -i "s/^BETTER_AUTH_SECRET=$/BETTER_AUTH_SECRET=$secret/" "$EVDEV_DIR/.env"
    fi
    echo "    info: auto-generated BETTER_AUTH_SECRET in .env" >&2
  fi

  if [ ! -f "$EVDEV_DIR/node_modules/.bin/bos" ]; then
    echo "==> Installing everything-dev dependencies..."
    (cd "$EVDEV_DIR" && bun install)
  fi

  export IRONCLAW_BASE_URL="http://$REBORN_HOST:$REBORN_PORT"
  export IRONCLAW_API_TOKEN="$IRONCLAW_REBORN_WEBUI_TOKEN"
  export IRONCLAW_REBORN_CORS_ORIGINS="http://localhost:3000"

  REBORN_LOG="$(mktemp)"
  echo "==> Starting ironclaw-reborn on http://$REBORN_HOST:$REBORN_PORT (log: $REBORN_LOG)"
  "${CARGO[@]}" serve --confirm-host-access --host "$REBORN_HOST" --port "$REBORN_PORT" > "$REBORN_LOG" 2>&1 &
  REBORN_PID=$!
  sleep 1

  cat << BANNER

══════════════════════════════════════════════════════════════════
 IronClaw Reborn — MODE: LOCAL
══════════════════════════════════════════════════════════════════
  Plugin auto-discovers via IRONCLAW_BASE_URL — no setup needed.

  Open http://localhost:3000 — sidebar shows (●) Connected.

  API        : http://$REBORN_HOST:$REBORN_PORT
  Token      : $IRONCLAW_REBORN_WEBUI_TOKEN
  Reborn home: $IRONCLAW_REBORN_HOME
  Reborn log : $REBORN_LOG

  Press Ctrl+C to stop

BANNER

  echo "==> Starting everything-dev dev stack..."
  cd "$EVDEV_DIR"
  tail -f "$REBORN_LOG" &
  TAIL_PID=$!
  bun run dev || true
  cleanup
fi

# ─────────────────────────────────────────────────────────────────────
# Tunnel mode — start Reborn locally + ngrok tunnel.
# Ngrok provides HTTPS with full SSE/WebSocket/grpc support.
# Free tier works: sign up at https://dashboard.ngrok.com
# ─────────────────────────────────────────────────────────────────────
if [ "$MODE" = "tunnel" ]; then
  REBORN_LOG="$(mktemp)"
  echo "==> Starting ironclaw-reborn on http://$REBORN_HOST:$REBORN_PORT (log: $REBORN_LOG)"
  "${CARGO[@]}" serve --confirm-host-access --host "$REBORN_HOST" --port "$REBORN_PORT" > "$REBORN_LOG" 2>&1 &
  REBORN_PID=$!

  sleep 2

  # ── Ngrok check and install guide ──────────────────────────
  if ! command -v ngrok &>/dev/null; then
    echo ""
    echo "==> ngrok is required for tunnel mode. Install it:"
    echo ""
    case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
      darwin)
        echo "    brew install ngrok/ngrok/ngrok"
        echo "    ngrok config add-authtoken <your-token>"
        ;;
      linux)
        echo "    curl -sSL https://ngrok-agent.s3.amazonaws.com/ngrok.asc | \\"
        echo "      sudo tee /etc/apt/trusted.gpg.d/ngrok.asc >/dev/null"
        echo "    echo 'deb https://ngrok-agent.s3.amazonaws.com buster main' | \\"
        echo "      sudo tee /etc/apt/sources.list.d/ngrok.list >/dev/null"
        echo "    sudo apt update && sudo apt install ngrok"
        echo "    ngrok config add-authtoken <your-token>"
        ;;
      mingw*|msys*|cygwin*)
        echo "    winget install ngrok"
        echo "    # Or download from https://ngrok.com/download"
        echo "    ngrok config add-authtoken <your-token>"
        ;;
      *)
        echo "    Download ngrok from https://ngrok.com/download"
        echo "    ngrok config add-authtoken <your-token>"
        ;;
    esac
    echo ""
    echo "    Get your free auth token at:"
    echo "    https://dashboard.ngrok.com/get-started/your-authtoken"
    echo ""
    cleanup
    exit 1
  fi

  # Check if ngrok auth token is configured
  if ! ngrok config check 2>&1 | grep -qi "valid configuration"; then
    echo ""
    echo "==> ngrok auth token not configured."
    echo "    Run:  ngrok config add-authtoken <your-token>"
    echo ""
    echo "    Get your free token at:"
    echo "    https://dashboard.ngrok.com/get-started/your-authtoken"
    echo ""
    cleanup
    exit 1
  fi

  printf "==> Starting ngrok tunnel..."
  TUNNEL_LOG="$(mktemp)"
  TUNNEL_URL=""

  ngrok http "$REBORN_HOST:$REBORN_PORT" --log=stdout > "$TUNNEL_LOG" 2>&1 &
  TUNNEL_PID=$!
  for i in $(seq 1 15); do
    TUNNEL_URL="$(grep -oE 'url=https://[^ ]+' "$TUNNEL_LOG" 2>/dev/null | head -1 | sed 's/^url=//' || true)"
    [ -n "$TUNNEL_URL" ] && break
    printf "."
    sleep 1
  done
  printf "\n"

  if [ -z "$TUNNEL_URL" ]; then
    printf "\n"
    printf "warning: could not detect ngrok tunnel URL (last lines of tunnel log):\n" >&2
    tail -5 "$TUNNEL_LOG" >&2
    TUNNEL_URL="(see tunnel log: $TUNNEL_LOG)"
  fi

  cat << BANNER

══════════════════════════════════════════════════════════════════
 IronClaw Reborn — MODE: TUNNEL  (ngrok)
══════════════════════════════════════════════════════════════════

  >>> Tunnel URL:  $TUNNEL_URL
  >>> Token:       $IRONCLAW_REBORN_WEBUI_TOKEN

  Go to Settings → IronClaw and fill in:
    Tunnel URL:   $TUNNEL_URL
    API Token:    $IRONCLAW_REBORN_WEBUI_TOKEN

  Reborn home: $IRONCLAW_REBORN_HOME

  Press Ctrl+C to stop

BANNER

  tail -f "$REBORN_LOG" &
  TAIL_PID=$!

  wait
fi

# ─────────────────────────────────────────────────────────────────────
# Remote mode — start Reborn + bos start against a remote gateway
# ─────────────────────────────────────────────────────────────────────
if [ "$MODE" = "remote" ]; then
  EVDEV_DIR="$REPO_ROOT/app/ironclaw.everything.dev"

  if [ ! -d "$EVDEV_DIR" ]; then
    echo "error: everything-dev project not found at $EVDEV_DIR" >&2
    exit 1
  fi

  if [ ! -f "$EVDEV_DIR/node_modules/.bin/bos" ]; then
    echo "==> Installing everything-dev dependencies..."
    (cd "$EVDEV_DIR" && bun install)
  fi

  export IRONCLAW_BASE_URL="http://$REBORN_HOST:$REBORN_PORT"
  export IRONCLAW_API_TOKEN="$IRONCLAW_REBORN_WEBUI_TOKEN"
  export IRONCLAW_REBORN_CORS_ORIGINS="http://localhost:3000"

  echo "==> Starting ironclaw-reborn on http://$REBORN_HOST:$REBORN_PORT"
  "${CARGO[@]}" serve --confirm-host-access --host "$REBORN_HOST" --port "$REBORN_PORT" &
  REBORN_PID=$!
  sleep 1

  cat << BANNER

══════════════════════════════════════════════════════════════════
 IronClaw Reborn — MODE: REMOTE GATEWAY
══════════════════════════════════════════════════════════════════
 Running local Reborn with the everything-dev host configured
 for the remote gateway. The ironclaw plugin loads from the
 published deployment and connects to your local Reborn.

  Account      : $ACCOUNT
  Gateway      : $DOMAIN
  API          : http://$REBORN_HOST:$REBORN_PORT
  Token        : $IRONCLAW_REBORN_WEBUI_TOKEN
  Reborn home  : $IRONCLAW_REBORN_HOME

  The everything-dev host prints its local URL when ready.
  Check 'bos status' or the host output for the exact port.

  Press Ctrl+C to stop

BANNER

  echo "==> Starting everything-dev host against $ACCOUNT/$DOMAIN..."
  cd "$EVDEV_DIR"
  bun run start -- --account "$ACCOUNT" --domain "$DOMAIN" || true
  cleanup
fi
