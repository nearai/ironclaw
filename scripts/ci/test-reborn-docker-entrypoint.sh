#!/usr/bin/env bash
# Self-test for docker/reborn/entrypoint.sh's startup contracts.
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
exit 0
STUB
chmod +x "${WORK}/bin/ironclaw"

# Root startup is exercised without requiring the test process itself to run as
# root. The entrypoint resolves these commands through PATH, so the stubs model
# the one root pass followed by gosu's non-root re-exec while recording ordering
# and environment consumption.
mkdir -p "${WORK}/root-bin"
cat > "${WORK}/root-bin/id" <<'STUB'
#!/bin/sh
test "${1:-}" = "-u"
printf '%s\n' "${IRONCLAW_STUB_UID:-1000}"
STUB
cat > "${WORK}/root-bin/ironclaw-reborn-start-sshd" <<'STUB'
#!/bin/sh
test -d "${IRONCLAW_REBORN_HOME}"
printf '%s\n' "${IRONCLAW_REBORN_SSH_PUBLIC_KEY:-}" > "${IRONCLAW_STUB_SSH_PATH}"
STUB
cat > "${WORK}/root-bin/chown" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" > "${IRONCLAW_STUB_CHOWN_PATH}"
STUB
cat > "${WORK}/root-bin/mkdir" <<'STUB'
#!/bin/sh
for path in "$@"; do
  case "$path" in
    -*) ;;
    /workspace) ;;
    *) /bin/mkdir -p "$path" ;;
  esac
done
STUB
cat > "${WORK}/root-bin/gosu" <<'STUB'
#!/bin/sh
if [ "${1:-}" != "ironclaw" ]; then
  echo "unexpected gosu user: ${1:-}" >&2
  exit 1
fi
if [ -n "${IRONCLAW_REBORN_SSH_PUBLIC_KEY:-}" ]; then
  echo "SSH public key survived the privilege drop" >&2
  exit 1
fi
printf '%s\n' "$*" > "${IRONCLAW_STUB_GOSU_PATH}"
shift
export IRONCLAW_STUB_UID=1000
exec "$@"
STUB
chmod +x "${WORK}/root-bin/id" \
  "${WORK}/root-bin/ironclaw-reborn-start-sshd" \
  "${WORK}/root-bin/chown" \
  "${WORK}/root-bin/mkdir" \
  "${WORK}/root-bin/gosu"

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
    # Keep the Railway persistence guard out of the way.
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID RAILWAY_VOLUME_MOUNT_PATH
    for assignment in "$@"; do
      export "${assignment?}"
    done
    sh "$ENTRYPOINT" >/dev/null 2>"${home}/entrypoint.err"
  )

  if [ ! -f "${home}/argv" ]; then
    echo "FAIL[${name}]: entrypoint never reached the ironclaw exec" >&2
    cat "${home}/entrypoint.err" >&2
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

# 4. Keyed SSH starts during the root pass, consumes the public key, then
#    re-executes the same entrypoint as the unprivileged application user.
ssh_key='ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITestOnlyKey entrypoint-test'
ssh_record="${WORK}/ssh-started"
gosu_record="${WORK}/gosu-argv"
chown_record="${WORK}/chown-argv"
out="$(run_entrypoint ssh_root "$LEGACY_DISABLED" \
  "PATH=${WORK}/root-bin:${WORK}/bin:${PATH}" \
  "IRONCLAW_STUB_UID=0" \
  "IRONCLAW_STUB_SSH_PATH=${ssh_record}" \
  "IRONCLAW_STUB_GOSU_PATH=${gosu_record}" \
  "IRONCLAW_STUB_CHOWN_PATH=${chown_record}" \
  "IRONCLAW_REBORN_SSH_PUBLIC_KEY=${ssh_key}")"
expect_absent ssh_root 'signing_secret_env' "$out"
if [ "$(cat "$ssh_record")" != "$ssh_key" ]; then
  echo "FAIL[ssh_root]: SSH did not start with the configured public key" >&2
  failures=$((failures + 1))
fi
if ! grep -q '^ironclaw .*docker/reborn/entrypoint.sh' "$gosu_record"; then
  echo "FAIL[ssh_root]: entrypoint did not re-exec through gosu: $(cat "$gosu_record")" >&2
  failures=$((failures + 1))
fi
if [ "$(cat "$chown_record")" != "ironclaw:ironclaw ${WORK}/ssh_root /workspace" ]; then
  echo "FAIL[ssh_root]: root pass did not hand runtime paths to ironclaw: $(cat "$chown_record")" >&2
  failures=$((failures + 1))
fi

# 5. A keyed non-root launch cannot silently advertise SSH without starting
#    the daemon. This is also the guard for an operator overriding USER.
nonroot_home="${WORK}/ssh-nonroot"
mkdir -p "$nonroot_home"
if (
  export PATH="${WORK}/root-bin:${WORK}/bin:${PATH}"
  export IRONCLAW_STUB_UID=1000
  export IRONCLAW_REBORN_HOME="$nonroot_home"
  export IRONCLAW_REBORN_SSH_PUBLIC_KEY="$ssh_key"
  sh "$ENTRYPOINT" >"${WORK}/ssh-nonroot.out" 2>"${WORK}/ssh-nonroot.err"
); then
  echo "FAIL[ssh_nonroot]: keyed SSH launch unexpectedly succeeded without root" >&2
  failures=$((failures + 1))
elif ! grep -q 'direct SSH requires the container entrypoint to start as root' \
  "${WORK}/ssh-nonroot.err"
then
  echo "FAIL[ssh_nonroot]: expected fail-closed diagnostic" >&2
  cat "${WORK}/ssh-nonroot.err" >&2
  failures=$((failures + 1))
fi

# 6. The dead variable has no reader left anywhere in the tree.
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
