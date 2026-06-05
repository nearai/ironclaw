#!/bin/sh
set -eu

default_config="${IRONCLAW_REBORN_DEFAULT_CONFIG:-/opt/ironclaw/reborn/config.toml}"

if [ ! -f "$IRONCLAW_REBORN_HOME/config.toml" ]; then
  mkdir -p "$IRONCLAW_REBORN_HOME"
  cp "$default_config" "$IRONCLAW_REBORN_HOME/config.toml"
fi

if [ "$#" -gt 0 ]; then
  exec ironclaw-reborn "$@"
fi

host="${IRONCLAW_REBORN_SERVE_HOST:-0.0.0.0}"
port="${PORT:-${IRONCLAW_REBORN_SERVE_PORT:-3000}}"

set -- serve --host "$host" --port "$port"

case "${IRONCLAW_REBORN_CONFIRM_HOST_ACCESS:-}" in
  1|true|TRUE|yes|YES)
    set -- "$@" --confirm-host-access
    ;;
esac

exec ironclaw-reborn "$@"
