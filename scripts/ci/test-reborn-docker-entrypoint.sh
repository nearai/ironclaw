#!/usr/bin/env bash
# Self-tests for docker/reborn/entrypoint.sh startup and narrow config migrations.
#
# #7115: the migration was gated on `IRONCLAW_REBORN_SLACK_ENABLED` not being
# truthy. That variable lost its last Rust reader in #6116, while the operator
# docs still told people to set it to `true` — so following the documented setup
# turned the migration off and left the retired `signing_secret_env` /
# `bot_token_env` keys in `config.toml`, which is exactly what makes `serve` fail
# closed. The container would not boot *because* the operator followed the docs.
#
# Driven through the real entrypoint, not through a copy of the awk block: the
# defect was in the `if` condition wrapping that block, so a test that invoked
# the block directly would have passed on the broken script.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENTRYPOINT="${ROOT}/docker/reborn/entrypoint.sh"

if [ ! -x "$ENTRYPOINT" ] && [ ! -f "$ENTRYPOINT" ]; then
  echo "FAIL: entrypoint not found at $ENTRYPOINT" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# `exec ironclaw "$@"` terminates the script; a stub keeps the run in-process
# and records that boot was actually reached rather than aborted early.
mkdir -p "${WORK}/bin"
cat > "${WORK}/bin/ironclaw" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" > "${IRONCLAW_STUB_ARGV_PATH}"
printf '%s\n' "${IRONCLAW_REBORN_STORAGE_MIGRATION:-}" > "${IRONCLAW_STUB_MIGRATION_PATH}"
exit 0
STUB
chmod +x "${WORK}/bin/ironclaw"

failures=0

# Runs the entrypoint over a seeded config and echoes the resulting file.
# $1 = case name, $2 = seeded config body, remaining args = extra `KEY=VALUE` env.
run_entrypoint() {
  local name="$1"
  local body="$2"
  shift 2

  local home="${WORK}/${name}"
  mkdir -p "$home"
  printf '%s' "$body" > "${home}/config.toml"

  (
    export PATH="${WORK}/bin:${PATH}"
    export IRONCLAW_REBORN_HOME="$home"
    # Never copied — the seeded config already exists — but the entrypoint
    # validates the path prefix before it decides that.
    export IRONCLAW_REBORN_DEFAULT_CONFIG=/opt/ironclaw/reborn/config.toml
    export IRONCLAW_STUB_ARGV_PATH="${home}/argv"
    export IRONCLAW_STUB_MIGRATION_PATH="${home}/migration-policy"
    # Keep the Railway persistence guard out of the way.
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID RAILWAY_VOLUME_MOUNT_PATH
    unset IRONCLAW_REBORN_SERVE_HOST IRONCLAW_REBORN_SERVE_PORT PORT
    unset IRONCLAW_REBORN_STORAGE_MIGRATION
    for assignment in "$@"; do
      export "${assignment?}"
    done
    sh "$ENTRYPOINT" >/dev/null 2>&1
  )

  if [ ! -f "${home}/argv" ]; then
    echo "FAIL[${name}]: entrypoint never reached the ironclaw exec" >&2
    # Every caller invokes this function inside a command substitution, so the
    # body runs in a subshell and a `failures=$((failures + 1))` here would
    # mutate a copy the parent never sees (the run stayed green with this
    # check firing — sabotage-verified). Exit instead: under the parent's
    # `set -e` the failed substitution aborts the whole script non-zero.
    exit 1
  fi
  cat "${home}/config.toml"
}

expect_absent() {
  local name="$1" needle="$2" haystack="$3"
  if printf '%s' "$haystack" | grep -q "$needle"; then
    echo "FAIL[${name}]: expected '${needle}' to be migrated away, but it survived:" >&2
    printf '%s\n' "$haystack" >&2
    failures=$((failures + 1))
  fi
}

expect_present() {
  local name="$1" needle="$2" haystack="$3"
  if ! printf '%s' "$haystack" | grep -q "$needle"; then
    echo "FAIL[${name}]: expected '${needle}' to survive, but it was removed:" >&2
    printf '%s\n' "$haystack" >&2
    failures=$((failures + 1))
  fi
}

expect_literal_present() {
  local name="$1" needle="$2" haystack="$3"
  if ! printf '%s' "$haystack" | grep -Fq -- "$needle"; then
    echo "FAIL[${name}]: expected literal '${needle}' to be present:" >&2
    printf '%s\n' "$haystack" >&2
    failures=$((failures + 1))
  fi
}

LEGACY_DISABLED='[slack]
enabled = false
signing_secret_env = "SLACK_SIGNING_SECRET"
bot_token_env = "SLACK_BOT_TOKEN"

[storage]
'

LEGACY_ENABLED='[slack]
enabled = true
signing_secret_env = "SLACK_SIGNING_SECRET"
bot_token_env = "SLACK_BOT_TOKEN"

[storage]
'

# 1. Baseline: a disabled `[slack]` with legacy fields is migrated.
out="$(run_entrypoint plain "$LEGACY_DISABLED")"
expect_absent plain 'signing_secret_env' "$out"
expect_absent plain 'bot_token_env' "$out"
expect_present plain 'enabled = false' "$out"
local_argv="$(cat "${WORK}/plain/argv")"
expect_literal_present plain 'serve --host 127.0.0.1 --port 3000' "$local_argv"

out="$(run_entrypoint explicit_host "$LEGACY_DISABLED" \
  "IRONCLAW_REBORN_SERVE_HOST=127.0.0.1")"
explicit_argv="$(cat "${WORK}/explicit_host/argv")"
expect_literal_present explicit_host 'serve --host 127.0.0.1 --port 3000' "$explicit_argv"

# 2. The regression itself. Every truthy spelling the entrypoint's `is_truthy`
#    accepts used to suppress the migration; the docs taught `=true`.
for truthy in 1 true TRUE yes YES; do
  out="$(run_entrypoint "truthy_${truthy}" "$LEGACY_DISABLED" "IRONCLAW_REBORN_SLACK_ENABLED=${truthy}")"
  expect_absent "truthy_${truthy}" 'signing_secret_env' "$out"
  expect_absent "truthy_${truthy}" 'bot_token_env' "$out"
done

# 3. The deliberate carve-out: `enabled = true` plus legacy fields is left
#    untouched so `serve` fails loudly instead of a live config being rewritten
#    underneath the operator. Asserted so the choice cannot be reverted silently.
out="$(run_entrypoint enabled_true "$LEGACY_ENABLED")"
expect_present enabled_true 'signing_secret_env' "$out"
expect_present enabled_true 'bot_token_env' "$out"

# 4. Railway uses the same entrypoint, receives a network-reachable host, and
#    forwards the operator's storage-migration policy override to `ironclaw serve`.
out="$(run_entrypoint railway_volume/ironclaw-reborn "$LEGACY_DISABLED" \
  "IRONCLAW_REBORN_HOME=" \
  "RAILWAY_ENVIRONMENT=production" \
  "RAILWAY_VOLUME_MOUNT_PATH=${WORK}/railway_volume" \
  "IRONCLAW_REBORN_STORAGE_MIGRATION=manual")"
railway_argv="$(cat "${WORK}/railway_volume/ironclaw-reborn/argv")"
railway_policy="$(cat "${WORK}/railway_volume/ironclaw-reborn/migration-policy")"
expect_literal_present railway_startup 'serve --host 0.0.0.0 --port 3000' "$railway_argv"
expect_present railway_startup '^manual$' "$railway_policy"

# An image-level host value would look like an operator override to the
# entrypoint and silently defeat its container-reachable default.
if awk '
  function inspect_instruction() {
    if (instruction ~ /^[[:space:]]*ENV[[:space:]]+/ \
        && instruction ~ /(^|[[:space:]])IRONCLAW_REBORN_SERVE_HOST([=[:space:]]|$)/) {
      found = 1
    }
  }
  {
    line = $0
    continued = sub(/\\[[:space:]]*$/, "", line)
    instruction = instruction " " line
    if (!continued) {
      inspect_instruction()
      instruction = ""
    }
  }
  END {
    if (instruction != "") {
      inspect_instruction()
    }
    exit !found
  }
' "${ROOT}/Dockerfile"; then
  echo "FAIL: Dockerfile must not bake IRONCLAW_REBORN_SERVE_HOST; the entrypoint derives it per runtime" >&2
  failures=$((failures + 1))
fi

# 5. The dead variable has no reader left anywhere in the tree.
if grep -rn 'IRONCLAW_REBORN_SLACK_ENABLED' \
  --include='*.rs' --include='*.sh' --include='*.toml' \
  "${ROOT}/crates" "${ROOT}/docker" "${ROOT}/scripts" 2>/dev/null \
  | grep -v 'test-reborn-docker-entrypoint.sh' \
  | grep -v '^.*entrypoint.sh:.*# ' ; then
  echo "FAIL: IRONCLAW_REBORN_SLACK_ENABLED regained a reader; it has had none since #6116" >&2
  failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
  echo "${failures} entrypoint self-test failure(s)" >&2
  exit 1
fi

echo "docker/reborn/entrypoint.sh self-tests passed"
