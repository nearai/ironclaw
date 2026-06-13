#!/usr/bin/env bash
# Launch IronClaw Reborn with the everything-dev web UI.
#
# Default mode (tunnel): starts Reborn locally and auto-launches a tunnel
# (cloudflared / ngrok / bore) so you can connect the deployed everything-dev
# UI at https://app.ironclaw.nearbuilders.org/settings/ironclaw.
#
#   --local  starts the everything-dev dev stack locally instead (no tunnel).
#            Requires the everything-dev project at app/ironclaw.nearbuilders.org/
#            to be set up with `cp .env.example .env && bun install`.
#
# Usage:
#   scripts/run-reborn-webui.sh                        # tunnel mode (default)
#   scripts/run-reborn-webui.sh --local                # local everything-dev mode
#   PROVIDER=openai scripts/run-reborn-webui.sh
#   PROVIDER=anthropic MODEL=claude-sonnet-4-20250514 scripts/run-reborn-webui.sh
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
#
# REBORN_HOST/REBORN_PORT are deliberately prefixed: a bare HOST would collide
# with zsh's auto-set $HOST (the machine hostname), which could bind serve to a
# non-loopback interface and expose the bearer token over plain HTTP.

set -euo pipefail

# Re-exec under bash if invoked via sh (brace expansion / arrays need bash).
if [ -z "${BASH_VERSION:-}" ]; then
  exec bash "$0" "$@"
fi

PROVIDER="${PROVIDER:-nearai}"
MODEL="${MODEL:-}"
REBORN_HOST="${REBORN_HOST:-127.0.0.1}"
REBORN_PORT="${REBORN_PORT:-3001}"
NEARAI_MODEL="${NEARAI_MODEL:-deepseek-ai/DeepSeek-V4-Flash}"
NEARAI_BASE_URL="${NEARAI_BASE_URL:-https://cloud-api.near.ai}"
IRONCLAW_REBORN_PROFILE="${IRONCLAW_REBORN_PROFILE:-local-dev-yolo}"
IRONCLAW_REBORN_LOG="${IRONCLAW_REBORN_LOG:-info}"
IRONCLAW_TRIGGER_POLLER_ENABLED="${IRONCLAW_TRIGGER_POLLER_ENABLED:-true}"

if [ "$REBORN_PORT" = "0" ]; then
  echo "error: REBORN_PORT=0 (kernel-assigned port) isn't usable with a tunnel." >&2
  echo "       Set a fixed REBORN_PORT, or run the test-harness form directly:" >&2
  echo "       cargo run -p ironclaw_reborn_cli --features webui-v2-beta -- serve --port 0" >&2
  exit 1
fi

# Detect mode
MODE="${1:-}"
if [ "$MODE" = "--local" ]; then
  MODE="local"
elif [ "$MODE" = "--deployed" ]; then
  MODE="tunnel"
elif [ -z "$MODE" ] || [ "${MODE#--}" = "$MODE" ]; then
  MODE="tunnel"
else
  echo "error: unknown flag '$MODE'. Use --local or --deployed." >&2
  exit 1
fi

# Kill any stale ironclaw-reborn on our port.
stale_pid="$(lsof -ti "tcp:$REBORN_PORT" -c ironclaw-reborn 2>/dev/null || true)"
if [ -n "$stale_pid" ]; then
  echo "==> Killing stale ironclaw-reborn (PID $stale_pid) on port $REBORN_PORT"
  kill "$stale_pid" 2>/dev/null || true
  sleep 1
fi

# Run cargo from the workspace root regardless of where the script is invoked.
REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

export IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME:-$HOME/.ironclaw-reborn-demo}"

# Reject a home inside the repo, which would trip the workspace/skill-root
# overlap validation in serve.
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

# Export runtime env vars that need to reach the spawned binary.
export IRONCLAW_UNSAFE_RAW_HTTP_EGRESS_ERRORS=1
export IRONCLAW_REBORN_LOG
export IRONCLAW_REBORN_PROFILE
export IRONCLAW_TRIGGER_POLLER_ENABLED
export NEARAI_BASE_URL
export NEARAI_MODEL

# Generate a random token by default (openssl is available on macOS/Linux).
if [ -z "${IRONCLAW_REBORN_WEBUI_TOKEN:-}" ]; then
  if command -v openssl &>/dev/null; then
    IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
  else
    IRONCLAW_REBORN_WEBUI_TOKEN="local-dev-token"
  fi
fi
export IRONCLAW_REBORN_WEBUI_TOKEN

# Resolve the API key env var name for the chosen provider.
key_env=""
if [ "$PROVIDER" = "nearai" ]; then
  key_env="NEARAI_API_KEY"
elif [ "$PROVIDER" = "openai" ]; then
  key_env="OPENAI_API_KEY"
elif [ "$PROVIDER" = "anthropic" ]; then
  key_env="ANTHROPIC_API_KEY"
fi

# Prompt for API key if unset.
if [ -n "$key_env" ] && [ -z "${!key_env:-}" ]; then
  echo "==> $key_env is not set."
  read -r -p "    Enter your $PROVIDER API key (or press Enter to skip and set it later): " api_key
  if [ -n "$api_key" ]; then
    export "$key_env=$api_key"
  else
    echo "    warning: no key provided. Turns will fail until you export $key_env." >&2
  fi
fi

CARGO=(cargo run -p ironclaw_reborn_cli --features webui-v2-beta --)

# Configure the model route (compiles the binary on first run).
set_provider_args=(models set-provider "$PROVIDER")
if [ -n "$MODEL" ]; then
  set_provider_args+=(--model "$MODEL")
fi
echo "==> Configuring model route: provider=$PROVIDER ${MODEL:+model=$MODEL}"
"${CARGO[@]}" "${set_provider_args[@]}"

# Match the WebUI user to the home's identity owner so serve's owner check
# passes (set-provider has now written/seeded config.toml). A caller-supplied
# IRONCLAW_REBORN_WEBUI_USER_ID wins; otherwise read [identity].default_owner
# from the config, falling back to reborn-cli (config init's default).
config_file="$IRONCLAW_REBORN_HOME/config.toml"
config_owner=""
if [ -f "$config_file" ]; then
  config_owner="$(sed -n 's/^[[:space:]]*default_owner[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$config_file" | head -1)"
fi
export IRONCLAW_REBORN_WEBUI_USER_ID="${IRONCLAW_REBORN_WEBUI_USER_ID:-${config_owner:-reborn-cli}}"

# Discover the credential env var for this provider and warn if it is unset.
key_env="$("${CARGO[@]}" models status 2>/dev/null \
  | sed -n 's/^default\.api_key_env: //p' || true)"
if [ -n "$key_env" ] && [ -z "${!key_env:-}" ]; then
  echo "warning: $key_env is not set. Required-key providers (openai, anthropic, …)" >&2
  echo "         fail at startup; export it before turns will work." >&2
fi

cleanup() {
  echo ""
  echo "Shutting down..."
  if [ -n "${REBORN_PID:-}" ]; then
    kill "$REBORN_PID" 2>/dev/null || true
  fi
  if [ -n "${TUNNEL_PID:-}" ]; then
    kill "$TUNNEL_PID" 2>/dev/null || true
  fi
  if [ -n "${TUNNEL_LOG:-}" ] && [ -f "$TUNNEL_LOG" ]; then
    rm -f "$TUNNEL_LOG"
  fi
  exit 0
}
trap cleanup SIGINT SIGTERM

# ─────────────────────────────────────────────────────────────────────
# Tunnel mode (default) — start Reborn locally + expose via tunnel
# ─────────────────────────────────────────────────────────────────────
if [ "$MODE" = "tunnel" ]; then
  # Start Reborn in the background.
  echo "==> Starting ironclaw-reborn on http://$REBORN_HOST:$REBORN_PORT"
  "${CARGO[@]}" serve --confirm-host-access --host "$REBORN_HOST" --port "$REBORN_PORT" &
  REBORN_PID=$!

  # Give the binary a moment to bind.
  sleep 2

  # Detect tunnel tool.
  TUNNEL_CMD=""
  if command -v cloudflared &>/dev/null; then
    TUNNEL_CMD="cloudflared"
  elif command -v ngrok &>/dev/null; then
    TUNNEL_CMD="ngrok"
  elif command -v bore &>/dev/null; then
    TUNNEL_CMD="bore"
  fi

  if [ -z "$TUNNEL_CMD" ]; then
    echo ""
    echo "error: no tunnel tool found. Install one of:" >&2
    echo "  cloudflared   brew install cloudflared    (recommended, no account needed)" >&2
    echo "  ngrok         brew install ngrok           (requires free account)" >&2
    echo "  bore          cargo install bore-cli       (simple, no account)" >&2
    echo "" >&2
    cleanup
    exit 1
  fi

  echo -n "==> Starting tunnel via $TUNNEL_CMD..."

  TUNNEL_LOG="$(mktemp)"
  TUNNEL_URL=""

  case "$TUNNEL_CMD" in
    cloudflared)
      cloudflared tunnel --url "http://$REBORN_HOST:$REBORN_PORT" > "$TUNNEL_LOG" 2>&1 &
      TUNNEL_PID=$!
      for i in {1..30}; do
        TUNNEL_URL="$(grep -oE 'https://[a-zA-Z0-9_.-]+\.trycloudflare\.com' "$TUNNEL_LOG" | tail -1)"
        [ -n "$TUNNEL_URL" ] && break
        echo -n "."
        sleep 1
      done
      echo ""
      ;;
    ngrok)
      ngrok http "$REBORN_HOST:$REBORN_PORT" --log=stdout > "$TUNNEL_LOG" 2>&1 &
      TUNNEL_PID=$!
      for i in {1..15}; do
        TUNNEL_URL="$(grep -oE '"public_url":"https://[^"]+' "$TUNNEL_LOG" | head -1 | sed 's/"public_url":"//')"
        [ -n "$TUNNEL_URL" ] && break
        echo -n "."
        sleep 1
      done
      echo ""
      ;;
    bore)
      bore local "$REBORN_PORT" --to bore.pub > "$TUNNEL_LOG" 2>&1 &
      TUNNEL_PID=$!
      for i in {1..10}; do
        TUNNEL_URL="$(grep -oE 'https?://[a-zA-Z0-9.:-]+' "$TUNNEL_LOG" | head -1)"
        [ -n "$TUNNEL_URL" ] && break
        echo -n "."
        sleep 1
      done
      echo ""
      if [ -n "$TUNNEL_URL" ]; then
        TUNNEL_URL="http://$TUNNEL_URL"
      fi
      ;;
  esac

  if [ -z "$TUNNEL_URL" ]; then
    echo ""
    echo "warning: could not detect tunnel URL (last lines of tunnel log):" >&2
    tail -5 "$TUNNEL_LOG" >&2
    TUNNEL_URL="(see tunnel log: $TUNNEL_LOG)"
  fi

  cat << BANNER

══════════════════════════════════════════════════════════════════
 IronClaw Reborn — MODE: TUNNEL
══════════════════════════════════════════════════════════════════
 Connect the deployed UI at app.ironclaw.nearbuilders.org to your
 local Reborn via the tunnel.

  Tunnel URL : $TUNNEL_URL
  Token      : $IRONCLAW_REBORN_WEBUI_TOKEN

  ┌─ Step 1 ─────────────────────────────────────────────────┐
  │  Open Settings → IronClaw at:                            │
  │    https://app.ironclaw.nearbuilders.org/settings/ironclaw │
  └──────────────────────────────────────────────────────────┘

  ┌─ Step 2 ─────────────────────────────────────────────────┐
  │  In the form, paste:                                     │
  │    Tunnel URL:  $TUNNEL_URL                              │
  │    Token:       $IRONCLAW_REBORN_WEBUI_TOKEN             │
  └──────────────────────────────────────────────────────────┘

  ┌─ Step 3 ─────────────────────────────────────────────────┐
  │  Click Save. The sidebar shows (●) Connected when ready. │
  └──────────────────────────────────────────────────────────┘

  API        : http://$REBORN_HOST:$REBORN_PORT
  Reborn home: $IRONCLAW_REBORN_HOME

  Press Ctrl+C to stop

BANNER

  # Wait for either process to exit.
  wait
fi

# ─────────────────────────────────────────────────────────────────────
# Local mode — start Reborn + everything-dev dev stack on localhost
# ─────────────────────────────────────────────────────────────────────
if [ "$MODE" = "local" ]; then
  EVDEV_DIR="$REPO_ROOT/app/ironclaw.nearbuilders.org"

  if [ ! -d "$EVDEV_DIR" ]; then
    echo "error: everything-dev project not found at $EVDEV_DIR" >&2
    exit 1
  fi

  if [ ! -f "$EVDEV_DIR/.env" ]; then
    echo "error: $EVDEV_DIR/.env not found." >&2
    echo "       Run 'cp .env.example .env' in that directory and configure it." >&2
    exit 1
  fi

  if [ ! -f "$EVDEV_DIR/node_modules/.bin/bos" ]; then
    echo "error: everything-dev dependencies not installed." >&2
    echo "       Run 'bun install' in $EVDEV_DIR" >&2
    exit 1
  fi

  # Tell the ironclaw plugin where to find Reborn.
  export IRONCLAW_BASE_URL="http://$REBORN_HOST:$REBORN_PORT"
  export IRONCLAW_API_TOKEN="$IRONCLAW_REBORN_WEBUI_TOKEN"
  export IRONCLAW_REBORN_CORS_ORIGINS="http://localhost:3000"

  echo "==> Starting ironclaw-reborn on http://$REBORN_HOST:$REBORN_PORT"
  "${CARGO[@]}" serve --confirm-host-access --host "$REBORN_HOST" --port "$REBORN_PORT" &
  REBORN_PID=$!

  cat << BANNER

══════════════════════════════════════════════════════════════════
 IronClaw Reborn — MODE: LOCAL
══════════════════════════════════════════════════════════════════
 The ironclaw plugin auto-discovers Reborn via IRONCLAW_BASE_URL.
 No settings configuration needed — just open the UI.

  ┌─ Open ───────────────────────────────────────────────────┐
  │                                                          │
  │    http://localhost:3000                                  │
  │                                                          │
  │  The sidebar shows (●) Connected when ready.             │
  └──────────────────────────────────────────────────────────┘

  API        : http://$REBORN_HOST:$REBORN_PORT
  Token      : $IRONCLAW_REBORN_WEBUI_TOKEN
  Reborn home: $IRONCLAW_REBORN_HOME

  Press Ctrl+C to stop

BANNER

  echo "==> Starting everything-dev dev stack..."
  cd "$EVDEV_DIR"
  bun run dev || true
  cleanup
fi
