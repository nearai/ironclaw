#!/bin/sh
set -eu

is_truthy() {
  case "${1:-}" in
    1|true|TRUE|yes|YES) return 0 ;;
    *) return 1 ;;
  esac
}

railway_runtime_detected() {
  [ -n "${RAILWAY_ENVIRONMENT:-}" ] \
    || [ -n "${RAILWAY_PROJECT_ID:-}" ] \
    || [ -n "${RAILWAY_SERVICE_ID:-}" ]
}

railway_volume_mount=""
if [ -n "${RAILWAY_VOLUME_MOUNT_PATH:-}" ]; then
  railway_volume_mount="${RAILWAY_VOLUME_MOUNT_PATH%/}"
  if [ -z "$railway_volume_mount" ]; then
    railway_volume_mount="/"
  fi
fi

if [ -n "${IRONCLAW_REBORN_HOME:-}" ]; then
  IRONCLAW_REBORN_HOME="${IRONCLAW_REBORN_HOME%/}"
elif [ -n "$railway_volume_mount" ]; then
  case "$railway_volume_mount" in
    */ironclaw-reborn) IRONCLAW_REBORN_HOME="$railway_volume_mount" ;;
    *) IRONCLAW_REBORN_HOME="$railway_volume_mount/ironclaw-reborn" ;;
  esac
else
  IRONCLAW_REBORN_HOME="/data/ironclaw-reborn"
fi
export IRONCLAW_REBORN_HOME
if [ -n "${IRONCLAW_REBORN_DEFAULT_CONFIG:-}" ]; then
  default_config="$IRONCLAW_REBORN_DEFAULT_CONFIG"
else
  case "${IRONCLAW_REBORN_PROFILE:-}" in
    production|migration-dry-run)
      default_config="/opt/ironclaw/reborn/config.production.toml"
      ;;
    hosted-single-tenant)
      default_config="/opt/ironclaw/reborn/config.hosted-single-tenant.toml"
      ;;
    hosted-single-tenant-volume)
      default_config="/opt/ironclaw/reborn/config.hosted-single-tenant-volume.toml"
      ;;
    *)
      default_config="/opt/ironclaw/reborn/config.toml"
      ;;
  esac
fi
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

# One-time volume migration: `config.toml` is now the single source of
# truth for `[llm.default]`, written only by an explicit act (onboard,
# `config set`/`models set-provider`, or the WebUI settings page) — never
# implicitly baked into a shipped default config (see this repo's
# `docker/reborn/config.toml` comment). Before this change, EVERY shipped
# profile config (`config.toml`, `config.hosted-single-tenant.toml`,
# `config.hosted-single-tenant-volume.toml`, `config.production.toml`)
# baked in the identical `[llm.default]` stub below, and the block above
# only installs a default config when `$config_path` doesn't exist yet — so
# a pre-existing Railway volume from before this change still carries that
# stale baked-in stub verbatim and would otherwise never pick up the new
# "no implicit slot" behavior. This check strips the section ONLY when it
# is an EXACT, byte-for-byte match of the known old stub (header + exactly
# these three fields, immediately followed by a blank line, a new `[section]`
# header, or EOF) — an operator who has since edited `[llm.default]` in any
# way (different model, added fields, a deliberately-kept `nearai` pin,
# etc.) is left completely untouched, matching the entrypoint's existing
# narrowly-gated legacy-Slack-field migration just below. A backup of the
# pre-migration file is written alongside as `config.toml.pre-llm-migration`
# (once — never overwritten by a later boot) before any change is made.
if [ -f "$config_path" ]; then
  llm_stub_migration_needed="$(awk '
    BEGIN { state = 0; found = 0 }
    /^\[llm\.default\][[:space:]]*$/ { state = 1; next }
    state == 1 {
      if ($0 == "provider_id = \"nearai\"") { state = 2; next }
      state = 0
    }
    state == 2 {
      if ($0 == "model = \"deepseek-ai/DeepSeek-V4-Flash\"") { state = 3; next }
      state = 0
    }
    state == 3 {
      if ($0 == "api_key_env = \"NEARAI_API_KEY\"") { state = 4; next }
      state = 0
    }
    state == 4 {
      if ($0 == "" || $0 ~ /^\[/) { found = 1 }
      state = 0
    }
    END {
      if (state == 4) { found = 1 }
      print found
    }
  ' "$config_path")"
  if [ "$llm_stub_migration_needed" = "1" ]; then
    backup_path="${config_path}.pre-llm-migration"
    if [ ! -f "$backup_path" ]; then
      cp "$config_path" "$backup_path"
    fi
    tmp_config="${config_path}.tmp.$$"
    trap 'rm -f "$tmp_config"' EXIT HUP INT TERM
    awk '
      BEGIN { skip = 0 }
      /^\[llm\.default\][[:space:]]*$/ { skip = 4; next }
      skip > 0 { skip--; next }
      { print }
    ' "$config_path" > "$tmp_config"
    mv "$tmp_config" "$config_path"
    trap - EXIT HUP INT TERM
    echo "Migrated a stale baked-in [llm.default] stub out of $config_path (backup: $backup_path); LLM environment variables now drive runtime resolution directly. See docker/reborn/config.toml's comment." >&2
  fi
fi

if ! is_truthy "${IRONCLAW_REBORN_SLACK_ENABLED:-}" \
  && awk '
    /^[[:space:]]*\[/ {
      in_slack = ($0 ~ /^[[:space:]]*\[slack\][[:space:]]*$/)
    }
    in_slack && /^[[:space:]]*enabled[[:space:]]*=[[:space:]]*false[[:space:]]*$/ {
      disabled = 1
    }
    in_slack && /^[[:space:]]*(signing_secret_env|bot_token_env)[[:space:]]*=/ {
      legacy = 1
    }
    END { exit !(disabled && legacy) }
  ' "$config_path"
then
  tmp_config="${config_path}.tmp.$$"
  trap 'rm -f "$tmp_config"' EXIT HUP INT TERM
  awk '
    /^[[:space:]]*\[/ {
      in_slack = ($0 ~ /^[[:space:]]*\[slack\][[:space:]]*$/)
    }
    in_slack && /^[[:space:]]*(signing_secret_env|bot_token_env)[[:space:]]*=/ {
      next
    }
    { print }
  ' "$config_path" > "$tmp_config"
  mv "$tmp_config" "$config_path"
  trap - EXIT HUP INT TERM
  echo "Removed disabled legacy Slack setup fields from $config_path." >&2
fi

effective_profile="${IRONCLAW_REBORN_PROFILE:-}"
if [ -z "$effective_profile" ]; then
  effective_profile="$(sed -n 's/^[[:space:]]*profile[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$config_path" | sed -n '1p')"
fi
if [ -z "$effective_profile" ]; then
  effective_profile="local-dev"
fi

case "$effective_profile" in
  production|migration-dry-run)
    if ! grep -q '^[[:space:]]*\[storage\][[:space:]]*$' "$config_path" \
      || ! grep -q '^[[:space:]]*\[policy\][[:space:]]*$' "$config_path"
    then
      echo "IRONCLAW_REBORN_PROFILE=$effective_profile requires $config_path to contain [storage] and [policy]." >&2
      echo "The existing config looks like a stale local-dev seed; remove it to let the entrypoint install $default_config, or migrate it manually." >&2
      exit 1
    fi
    ;;
  hosted-single-tenant)
    if ! grep -q '^[[:space:]]*\[storage\][[:space:]]*$' "$config_path"
    then
      echo "IRONCLAW_REBORN_PROFILE=$effective_profile requires $config_path to contain [storage]." >&2
      echo "The existing config looks like a stale local-dev seed; remove it to let the entrypoint install $default_config, or migrate it manually." >&2
      exit 1
    fi
    ;;
esac

if railway_runtime_detected \
  && ! is_truthy "${IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY:-}"
then
  case "$effective_profile" in
    local-dev|local-dev-yolo|hosted-single-tenant|hosted-single-tenant-volume)
      if [ -z "$railway_volume_mount" ]; then
        echo "Railway deployment using profile=$effective_profile requires a persistent volume for IRONCLAW_REBORN_HOME=$IRONCLAW_REBORN_HOME." >&2
        echo "Attach a Railway volume mounted at /data (or set IRONCLAW_REBORN_HOME under RAILWAY_VOLUME_MOUNT_PATH)." >&2
        echo "Set IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true only for disposable test deployments." >&2
        exit 1
      fi
      case "$IRONCLAW_REBORN_HOME" in
        "$railway_volume_mount"|"$railway_volume_mount"/*) ;;
        *)
          echo "Railway deployment using profile=$effective_profile requires IRONCLAW_REBORN_HOME=$IRONCLAW_REBORN_HOME to be under RAILWAY_VOLUME_MOUNT_PATH=$railway_volume_mount." >&2
          echo "Unset IRONCLAW_REBORN_HOME to use $railway_volume_mount/ironclaw-reborn, or set IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true only for disposable tests." >&2
          exit 1
          ;;
      esac
      ;;
  esac
fi

host="${IRONCLAW_REBORN_SERVE_HOST:-127.0.0.1}"
port="${PORT:-${IRONCLAW_REBORN_SERVE_PORT:-3000}}"

resolve_env_placeholder_arg() {
  case "$1" in
    '$IRONCLAW_REBORN_SERVE_HOST'|'${IRONCLAW_REBORN_SERVE_HOST}')
      printf '%s\n' "$host"
      ;;
    '$PORT'|'${PORT}'|'$IRONCLAW_REBORN_SERVE_PORT'|'${IRONCLAW_REBORN_SERVE_PORT}')
      printf '%s\n' "$port"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

if [ "$#" -gt 0 ]; then
  original_arg_count="$#"
  while [ "$original_arg_count" -gt 0 ]; do
    arg="$(resolve_env_placeholder_arg "$1")"
    shift
    original_arg_count=$((original_arg_count - 1))
    set -- "$@" "$arg"
  done
  exec ironclaw "$@"
fi

set -- serve --host "$host" --port "$port"

if is_truthy "${IRONCLAW_REBORN_CONFIRM_HOST_ACCESS:-}"; then
  set -- "$@" --confirm-host-access
fi

exec ironclaw "$@"
