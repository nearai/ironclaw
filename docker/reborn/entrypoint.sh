#!/bin/sh
set -eu

IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME:-/data/ironclaw-reborn}"
export IRONCLAW_REBORN_HOME
default_config="${IRONCLAW_REBORN_DEFAULT_CONFIG:-/opt/ironclaw/reborn/config.toml}"
config_path="$IRONCLAW_REBORN_HOME/config.toml"

case "$default_config" in
  /opt/ironclaw/*) ;;
  *)
    echo "IRONCLAW_REBORN_DEFAULT_CONFIG must be under /opt/ironclaw: $default_config" >&2
    exit 1
    ;;
esac

case "$default_config" in
  *"/../"*|*"/.."|*"../"*|*"/."|*"/./"*)
    echo "IRONCLAW_REBORN_DEFAULT_CONFIG must not contain relative path segments: $default_config" >&2
    exit 1
    ;;
esac

if [ ! -f "$config_path" ]; then
  mkdir -p "$IRONCLAW_REBORN_HOME"
  tmp_config="${config_path}.tmp.$$"
  trap 'rm -f "$tmp_config"' EXIT HUP INT TERM
  cp "$default_config" "$tmp_config"
  if ! ln "$tmp_config" "$config_path" 2>/dev/null && [ ! -f "$config_path" ]; then
    echo "failed to install default Reborn config at $config_path" >&2
    exit 1
  fi
  rm -f "$tmp_config"
  trap - EXIT HUP INT TERM
fi

if [ "$#" -gt 0 ]; then
  exec ironclaw-reborn "$@"
fi

host="${IRONCLAW_REBORN_SERVE_HOST:-127.0.0.1}"
port="${PORT:-${IRONCLAW_REBORN_SERVE_PORT:-3000}}"

set -- serve --host "$host" --port "$port"

case "${IRONCLAW_REBORN_CONFIRM_HOST_ACCESS:-}" in
  1|true|TRUE|yes|YES)
    set -- "$@" --confirm-host-access
    ;;
esac

exec ironclaw-reborn "$@"
