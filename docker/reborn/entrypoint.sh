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
  # `${VAR%/}` turns a bare "/" into an empty string, which used to reach
  # `mkdir -p "" /workspace` and die with a confusing diagnostic. Reject the
  # filesystem root outright rather than restoring it: the root pass chowns
  # this path to the runtime uid, so honoring "/" would hand uid 1000
  # ownership of the container's entire root filesystem.
  if [ -z "$IRONCLAW_REBORN_HOME" ]; then
    echo "IRONCLAW_REBORN_HOME must not be the filesystem root (/); it is chowned to the unprivileged runtime user at startup." >&2
    exit 1
  fi
elif [ -n "$railway_volume_mount" ]; then
  case "$railway_volume_mount" in
    */ironclaw-reborn) IRONCLAW_REBORN_HOME="$railway_volume_mount" ;;
    *) IRONCLAW_REBORN_HOME="$railway_volume_mount/ironclaw-reborn" ;;
  esac
else
  IRONCLAW_REBORN_HOME="/data/ironclaw-reborn"
fi
export IRONCLAW_REBORN_HOME

if [ -n "${IRONCLAW_REBORN_WORKSPACE_ROOT:-}" ]; then
  IRONCLAW_REBORN_WORKSPACE_ROOT="${IRONCLAW_REBORN_WORKSPACE_ROOT%/}"
  # Same reasoning as IRONCLAW_REBORN_HOME above. Without this the empty value
  # reached `readlink -m ""`, which exits non-zero and, under `set -eu`, killed
  # the boot with no diagnostic at all.
  if [ -z "$IRONCLAW_REBORN_WORKSPACE_ROOT" ]; then
    echo "IRONCLAW_REBORN_WORKSPACE_ROOT must not be the filesystem root (/)." >&2
    exit 1
  fi
else
  IRONCLAW_REBORN_WORKSPACE_ROOT="$IRONCLAW_REBORN_HOME/workspace"
fi
export IRONCLAW_REBORN_WORKSPACE_ROOT

ssh_public_key="${IRONCLAW_REBORN_SSH_PUBLIC_KEY:-}"
if [ "$(id -u)" = "0" ]; then
  # The workspace root is created and chowned here too (not just at
  # $IRONCLAW_REBORN_HOME and /workspace) because an explicit
  # IRONCLAW_REBORN_WORKSPACE_ROOT override can point at a path whose parent
  # is a fresh, root-owned volume mount. Left until the later unprivileged
  # `mkdir -p` (below, after the privilege drop), that call would fail closed
  # with EACCES under `set -eu` and abort the container at boot. This chown
  # is intentionally non-recursive (no `-R`): start-sshd.sh relies on
  # $IRONCLAW_REBORN_HOME/ssh staying root-owned.
  mkdir -p "$IRONCLAW_REBORN_HOME" /workspace
  chown ironclaw:ironclaw "$IRONCLAW_REBORN_HOME" /workspace
  # Repair ownership of pre-existing home CONTENTS, not just the directory.
  # A persistent volume outlives the image, and this image ran as `ironclaw`
  # until in-worker SSH required a root entrypoint -- so a volume can carry
  # files a root-run wrote. One unreadable file is fatal rather than
  # degraded: the provider-registry overlay is fail-closed, so a root-owned
  # `providers.json` crash-loops the container with
  # "failed to read provider registry overlay ...: Permission denied", while
  # a sibling `config.toml` written earlier still loads fine.
  #
  # `ssh` is excluded deliberately: start-sshd.sh refuses to start when its
  # state directory is not root-owned, so recursing into it would trade this
  # crash loop for a broken SSH listener.
  # `-H` follows the start path only: $IRONCLAW_REBORN_HOME may itself be a
  # symlink (find's default physical walk would then match nothing under
  # -mindepth 1 and silently repair nothing), while symlinks encountered
  # *during* the walk are still never followed -- combined with `chown -h`,
  # a symlink planted in the home cannot redirect ownership outside it.
  # Deliberately NOT filtered with `! -user ironclaw`: that predicate resolves
  # the name at find time and aborts the whole boot under `set -e` wherever it
  # does not resolve, trading a rare ownership repair for a guaranteed crash.
  find -H "$IRONCLAW_REBORN_HOME" -mindepth 1 \
    -path "$IRONCLAW_REBORN_HOME/ssh" -prune -o \
    -exec chown -h ironclaw:ironclaw {} +
  # ...but ONLY when the override provably lives inside a directory this
  # entrypoint already manages. The Railway containment check that validates an
  # operator-supplied workspace root runs after the privilege drop (it needs
  # $effective_profile, resolved from the config file further down), so
  # chowning the raw value here would hand the runtime uid ownership of an
  # arbitrary path -- `IRONCLAW_REBORN_WORKSPACE_ROOT=/etc` chowns /etc to
  # ironclaw before anything rejects it. Paths outside are left untouched: the
  # containment check rejects them shortly afterwards, and off-Railway the
  # later unprivileged `mkdir -p` reports the failure as it did before.
  # Compare canonicalized paths so `$IRONCLAW_REBORN_HOME/../../etc` cannot
  # spell its way in.
  canonical_home="$(readlink -m "$IRONCLAW_REBORN_HOME")"
  canonical_ws_root="$(readlink -m "$IRONCLAW_REBORN_WORKSPACE_ROOT")"
  workspace_root_managed=false
  case "$canonical_ws_root" in
    "$canonical_home"|"$canonical_home"/*) workspace_root_managed=true ;;
  esac
  if [ "$workspace_root_managed" = false ] \
    && [ -n "$railway_volume_mount" ] && [ "$railway_volume_mount" != "/" ]; then
    canonical_mount="$(readlink -m "$railway_volume_mount")"
    case "$canonical_ws_root" in
      "$canonical_mount"/*) workspace_root_managed=true ;;
    esac
  fi
  if [ "$workspace_root_managed" = true ]; then
    mkdir -p "$IRONCLAW_REBORN_WORKSPACE_ROOT"
    chown ironclaw:ironclaw "$IRONCLAW_REBORN_WORKSPACE_ROOT"
  fi
  if [ -n "$ssh_public_key" ]; then
    ironclaw-reborn-start-sshd
  fi
  unset IRONCLAW_REBORN_SSH_PUBLIC_KEY
  exec gosu ironclaw "$0" "$@"
fi
if [ -n "$ssh_public_key" ]; then
  echo "direct SSH requires the container entrypoint to start as root" >&2
  exit 1
fi

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
    hosted-single-tenant-volume|hosted-single-tenant-volume-sandboxed|hosted-single-tenant-volume-sandboxed-railway)
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

# Strip the retired `[slack]` setup fields that make `serve` fail closed.
#
# This used to also require `IRONCLAW_REBORN_SLACK_ENABLED` to be non-truthy.
# That variable lost its last Rust reader in #6116, which deleted the
# enablement-gate path outright — this line was the only thing in the repo
# still reading it. Worse, the operator docs instructed setting it to `true`,
# so following the documented setup **disabled the migration** and produced a
# container that would not boot: the mechanism built to prevent exactly that
# failure was switched off by the same instruction (#7115). The awk condition
# below is the whole signal.
#
# Chosen, not incidental: the migration fires only for `enabled = false`. A
# config with `enabled = true` plus legacy fields is left alone and fails
# `serve` closed with a migration pointer, because silently rewriting an
# apparently-live channel config is worse than refusing to start. This matches
# the narrowly-gated `[llm.default]` migration above.
if awk '
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
    local-dev|local-dev-yolo|hosted-single-tenant|hosted-single-tenant-volume|hosted-single-tenant-volume-sandboxed|hosted-single-tenant-volume-sandboxed-railway)
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
      # Compare canonicalized paths, not their original spelling. A symlink
      # beneath the mount whose target is outside it, or a `..` segment such as
      # `/volume/../ephemeral`, passes a purely lexical prefix test while the
      # runtime resolves the real (ephemeral) target — booting a deployment
      # whose project files silently do not persist, which is the exact failure
      # this guard exists to prevent. `readlink -m` canonicalizes symlinks and
      # `.`/`..` segments even when a path (or an intermediate component of it)
      # does not exist yet. `readlink -f` requires every component but the
      # last to already exist and otherwise fails outright, which used to send
      # a not-yet-created path (routine for a fresh volume) down a fallback to
      # its raw, unresolved spelling — and a raw spelling containing `..` can
      # lexically match the containment check below while actually resolving
      # outside the mount, defeating this guard. There is deliberately no
      # fallback to the raw spelling any more: failing closed on a
      # canonicalization error is safe, silently comparing an unresolved path
      # is not.
      if ! canonical_workspace_root="$(readlink -m "$IRONCLAW_REBORN_WORKSPACE_ROOT" 2>/dev/null)"; then
        echo "Failed to resolve IRONCLAW_REBORN_WORKSPACE_ROOT=$IRONCLAW_REBORN_WORKSPACE_ROOT for the Railway containment check." >&2
        exit 1
      fi
      if ! canonical_volume_mount="$(readlink -m "$railway_volume_mount" 2>/dev/null)"; then
        echo "Failed to resolve RAILWAY_VOLUME_MOUNT_PATH=$railway_volume_mount for the Railway containment check." >&2
        exit 1
      fi
      case "$canonical_workspace_root" in
        "$canonical_volume_mount"|"$canonical_volume_mount"/*) ;;
        *)
          echo "Railway deployment using profile=$effective_profile requires IRONCLAW_REBORN_WORKSPACE_ROOT=$IRONCLAW_REBORN_WORKSPACE_ROOT (resolved: $canonical_workspace_root) to be under RAILWAY_VOLUME_MOUNT_PATH=$railway_volume_mount (resolved: $canonical_volume_mount)." >&2
          echo "Unset IRONCLAW_REBORN_WORKSPACE_ROOT to use $IRONCLAW_REBORN_HOME/workspace, or set IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true only for disposable tests." >&2
          exit 1
          ;;
      esac
      ;;
  esac
fi

case "$effective_profile" in
  local-dev|local-dev-yolo|hosted-single-tenant|hosted-single-tenant-volume|hosted-single-tenant-volume-sandboxed|hosted-single-tenant-volume-sandboxed-railway)
    mkdir -p "$IRONCLAW_REBORN_WORKSPACE_ROOT"
    ;;
esac

# Serve-host resolution: an explicit IRONCLAW_REBORN_SERVE_HOST always wins.
# Otherwise, on Railway (and any platform that sets the RAILWAY_* markers) the
# container MUST bind 0.0.0.0 or the platform health check / ingress cannot
# reach it — a loopback bind fails the deploy. Off-Railway (e.g. a local
# `docker run`) keeps the conservative loopback default.
if [ -n "${IRONCLAW_REBORN_SERVE_HOST:-}" ]; then
  host="${IRONCLAW_REBORN_SERVE_HOST}"
elif railway_runtime_detected; then
  host="0.0.0.0"
else
  host="127.0.0.1"
fi
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
