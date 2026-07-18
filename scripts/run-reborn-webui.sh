#!/usr/bin/env bash
# Launch IronClaw Reborn with the WebChat v2 web UI for local testing.
#
# Handles the setup footguns from docs/reborn-binary.md for you:
#   - keeps the Reborn home OUTSIDE the repo (serve uses the cwd as the
#     local-dev workspace root and rejects overlap with it);
#   - configures the model route via `models set-provider`;
#   - generates the WebUI bearer token and sets the WebUI user to the home's
#     `[identity].default_owner` (falling back to `reborn-cli`, config init's
#     default) so serve's owner check doesn't refuse to start.
#
# Usage:
#   scripts/run-reborn-webui.sh                 # NEAR AI defaults
#   PROVIDER=openai scripts/run-reborn-webui.sh
#   PROVIDER=anthropic MODEL=claude-sonnet-4-20250514 scripts/run-reborn-webui.sh
#
# Before running, export your provider's API key, e.g.:
#   export NEARAI_API_KEY=...      # or OPENAI_API_KEY / ANTHROPIC_API_KEY
#
# Overridable via environment:
#   PROVIDER      provider id        (default: nearai)
#   MODEL         model id           (default: provider catalog default)
#   REBORN_HOST   listen host        (default: 127.0.0.1)
#   REBORN_PORT   listen port        (default: 3000)
#   IRONCLAW_REBORN_HOME             (default: $HOME/.ironclaw-reborn-demo)
#   IRONCLAW_REBORN_WEBUI_USER_ID    (default: home's [identity].default_owner)
#   IRONCLAW_REBORN_WEBUI_TOKEN      (default: generated and printed)
#
# REBORN_HOST/REBORN_PORT are deliberately prefixed: a bare HOST would collide
# with zsh's auto-set $HOST (the machine hostname), which could bind serve to a
# non-loopback interface and expose the bearer token over plain HTTP.

set -euo pipefail

PROVIDER="${PROVIDER:-nearai}"
MODEL="${MODEL:-}"
REBORN_HOST="${REBORN_HOST:-127.0.0.1}"
REBORN_PORT="${REBORN_PORT:-3000}"

# This launcher prints a login URL for a browser, so a fixed port is required.
# `serve --port 0` (kernel-picks-a-free-port) is for test harnesses only and
# would print an unusable http://REBORN_HOST:0/ here.
if [ "$REBORN_PORT" = "0" ]; then
  echo "error: REBORN_PORT=0 (kernel-assigned port) isn't usable for browser onboarding." >&2
  echo "       Set a fixed REBORN_PORT, or run the test-harness form directly:" >&2
  echo "       cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta \\" >&2
  echo "         --bin ironclaw -- serve --port 0" >&2
  exit 1
fi

# Run cargo from the workspace root regardless of where the script is invoked.
REPO_ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

FRONTEND_DIR="$REPO_ROOT/crates/ironclaw_webui/frontend"
if ! command -v pnpm >/dev/null 2>&1; then
  if command -v corepack >/dev/null 2>&1; then
    corepack enable pnpm
  fi
fi
if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: pnpm is required to build WebUI v2 assets." >&2
  echo "       Install Node 22 from .nvmrc and enable pnpm with: corepack enable pnpm" >&2
  exit 1
fi
echo "==> Building WebUI v2 frontend assets"
(
  cd "$FRONTEND_DIR"
  pnpm install --frozen-lockfile
  pnpm build
)

export IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME:-$HOME/.ironclaw-reborn-demo}"

# Reject a home inside the repo, which would trip the workspace/skill-root
# overlap validation in serve. Canonicalize both paths first (resolving `..`
# and symlinks, like serve does) so e.g. `../reborn-home` isn't mis-flagged.
# Resolve via the parent dir so we don't have to create the home to normalize
# it; if the parent doesn't exist yet, skip this friendly check and let serve's
# own validation handle it.
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

# Generate a WebUI bearer token if the caller didn't supply one.
if [ -z "${IRONCLAW_REBORN_WEBUI_TOKEN:-}" ]; then
  export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
fi

CARGO=(cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta --bin ironclaw --)

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

cat <<EOF

==> Starting WebChat v2 on http://$REBORN_HOST:$REBORN_PORT/
    login token : $IRONCLAW_REBORN_WEBUI_TOKEN
    login user  : $IRONCLAW_REBORN_WEBUI_USER_ID
    reborn home : $IRONCLAW_REBORN_HOME

EOF

exec "${CARGO[@]}" serve --host "$REBORN_HOST" --port "$REBORN_PORT"
