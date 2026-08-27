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
printf '%s\n' "${IRONCLAW_REBORN_WORKSPACE_ROOT}" > "${IRONCLAW_STUB_WORKSPACE_PATH}"
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
# Appends: the root pass issues a second chown for the workspace root only when
# that path is inside a directory the entrypoint manages, so the tests need to
# see every invocation, not just the last one.
printf '%s\n' "$*" >> "${IRONCLAW_STUB_CHOWN_PATH}"
STUB
cat > "${WORK}/root-bin/mkdir" <<'STUB'
#!/bin/sh
# IRONCLAW_STUB_UNWRITABLE_PARENT simulates a root-owned, non-writable parent
# directory: once the (stubbed) privilege drop has happened -- IRONCLAW_STUB_UID
# no longer "0" -- creating a not-yet-existing path under it fails closed, the
# same EACCES a real unprivileged `mkdir -p` would hit. A path already created
# during the earlier root pass is unaffected (mkdir -p on an existing directory
# is always a no-op), which is exactly what distinguishes the fixed entrypoint
# (creates it while still root) from the broken one (only tries later).
#
# status tracks whether any real `mkdir` invocation below failed (e.g. a
# blank path from an unfixed `IRONCLAW_REBORN_HOME=/` normalization bug) so
# this stub's own exit code matches real `mkdir`'s: a failure on one operand
# does not stop it from attempting the rest, but the overall call still
# reports non-zero. Without this, a failing path processed before the last
# loop iteration is silently swallowed -- the stub's exit status would be
# whatever the final (successful) iteration returned, and a regression test
# asserting on `set -eu` aborting the caller would pass for the wrong reason.
status=0
for path in "$@"; do
  case "$path" in
    -*) continue ;;
    /workspace) continue ;;
  esac
  if [ "${IRONCLAW_STUB_UID:-0}" != "0" ] && [ -n "${IRONCLAW_STUB_UNWRITABLE_PARENT:-}" ] \
    && [ ! -d "$path" ]
  then
    case "$path" in
      "${IRONCLAW_STUB_UNWRITABLE_PARENT}"|"${IRONCLAW_STUB_UNWRITABLE_PARENT}"/*)
        echo "mkdir: cannot create directory '$path': Permission denied" >&2
        exit 1
        ;;
    esac
  fi
  /bin/mkdir -p "$path" || status=1
done
exit "$status"
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
    export IRONCLAW_STUB_WORKSPACE_PATH="${home}/workspace-root"
    # Keep the Railway persistence guard out of the way.
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID RAILWAY_VOLUME_MOUNT_PATH
    unset IRONCLAW_REBORN_WORKSPACE_ROOT
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

assert_eq() {
  local name="$1" expected="$2" actual="$3" what="$4"
  if [ "$actual" != "$expected" ]; then
    echo "FAIL[${name}]: ${what}: expected '${expected}', got '${actual}'" >&2
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
assert_eq plain "${WORK}/plain/workspace" "$(cat "${WORK}/plain/workspace-root")" \
  'workspace root did not default beneath IRONCLAW_REBORN_HOME'

out="$(run_entrypoint custom_workspace "$LEGACY_DISABLED" "IRONCLAW_REBORN_WORKSPACE_ROOT=${WORK}/durable-workspace")"
assert_eq custom_workspace "${WORK}/durable-workspace" \
  "$(cat "${WORK}/custom_workspace/workspace-root")" \
  'explicit durable workspace root was not preserved'

# A trailing slash is normalized away so the Railway containment comparison and
# the Rust-side root agree on one spelling.
out="$(run_entrypoint trailing_slash "$LEGACY_DISABLED" "IRONCLAW_REBORN_WORKSPACE_ROOT=${WORK}/durable-workspace/")"
assert_eq trailing_slash "${WORK}/durable-workspace" \
  "$(cat "${WORK}/trailing_slash/workspace-root")" \
  'trailing slash was not stripped from the workspace root'

# The Railway containment guard compares resolved paths, not spelling. A
# workspace root that is a symlink *under* the mount but points outside it
# passes a lexical prefix test while the runtime writes to the ephemeral
# target — a deployment that boots and silently loses project files.
railway_mount="${WORK}/railway-mount"
railway_home="${railway_mount}/ironclaw-reborn"
mkdir -p "$railway_home" "${WORK}/railway-ephemeral" "${railway_mount}/real-workspace"
printf '%s' "$LEGACY_DISABLED" > "${railway_home}/config.toml"
ln -sfn "${WORK}/railway-ephemeral" "${railway_mount}/escaping-workspace"

run_railway_entrypoint() {
  (
    export PATH="${WORK}/bin:${PATH}"
    export IRONCLAW_REBORN_HOME="$railway_home"
    export IRONCLAW_REBORN_DEFAULT_CONFIG=/opt/ironclaw/reborn/config.toml
    export IRONCLAW_STUB_ARGV_PATH="${railway_home}/argv"
    export IRONCLAW_STUB_WORKSPACE_PATH="${railway_home}/workspace-root"
    export RAILWAY_ENVIRONMENT=production
    export RAILWAY_VOLUME_MOUNT_PATH="$railway_mount"
    export IRONCLAW_REBORN_WORKSPACE_ROOT="$1"
    unset IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY
    sh "$ENTRYPOINT" >/dev/null 2>"${railway_home}/railway.err"
  )
}

rm -f "${railway_home}/argv"
if run_railway_entrypoint "${railway_mount}/escaping-workspace"; then
  echo "FAIL[railway_escape]: a workspace root resolving outside the volume was accepted" >&2
  failures=$((failures + 1))
elif ! grep -q 'IRONCLAW_REBORN_WORKSPACE_ROOT' "${railway_home}/railway.err"; then
  echo "FAIL[railway_escape]: expected a workspace-root containment diagnostic" >&2
  cat "${railway_home}/railway.err" >&2
  failures=$((failures + 1))
fi

# Positive control: a real directory under the mount still boots. This also
# pins that both sides are canonicalized — on a host where the mount path
# itself traverses a symlink, resolving only one side would reject every root.
rm -f "${railway_home}/argv"
if ! run_railway_entrypoint "${railway_mount}/real-workspace"; then
  echo "FAIL[railway_contained]: a workspace root under the volume was rejected" >&2
  cat "${railway_home}/railway.err" >&2
  failures=$((failures + 1))
fi

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
if ! grep -qx "ironclaw:ironclaw ${WORK}/ssh_root /workspace" "$chown_record" \
  || ! grep -qx "ironclaw:ironclaw ${WORK}/ssh_root/workspace" "$chown_record"; then
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

# 6. An explicit IRONCLAW_REBORN_WORKSPACE_ROOT override must be created and
#    chowned during the root pass, before the privilege drop. Left until the
#    later unprivileged `mkdir -p` (bottom of the script), a workspace root
#    whose parent is a fresh, root-owned volume mount aborts the container at
#    boot under `set -eu`. IRONCLAW_STUB_UNWRITABLE_PARENT (consumed by the
#    root-bin/mkdir stub above) simulates exactly that: it fails any
#    not-yet-existing path under it once IRONCLAW_STUB_UID has flipped away
#    from "0", i.e. once the (stubbed) privilege drop has happened. The fixed
#    entrypoint creates the workspace root while still root, so the later
#    unprivileged call is a no-op against an already-existing directory.
workspace_root_privdrop_home="${WORK}/workspace-root-privdrop"
mkdir -p "$workspace_root_privdrop_home"
printf '%s' "$LEGACY_DISABLED" > "${workspace_root_privdrop_home}/config.toml"
locked_parent="${WORK}/locked-volume-mount"
locked_workspace_root="${locked_parent}/project-workspace"
workspace_root_chown_record="${workspace_root_privdrop_home}/chown-argv"

run_workspace_root_privdrop() {
  (
    export PATH="${WORK}/root-bin:${WORK}/bin:${PATH}"
    export IRONCLAW_REBORN_HOME="$workspace_root_privdrop_home"
    export IRONCLAW_REBORN_DEFAULT_CONFIG=/opt/ironclaw/reborn/config.toml
    export IRONCLAW_STUB_ARGV_PATH="${workspace_root_privdrop_home}/argv"
    export IRONCLAW_STUB_WORKSPACE_PATH="${workspace_root_privdrop_home}/workspace-root"
    export IRONCLAW_STUB_CHOWN_PATH="$workspace_root_chown_record"
    export IRONCLAW_STUB_UID=0
    export IRONCLAW_STUB_UNWRITABLE_PARENT="$locked_parent"
    export IRONCLAW_REBORN_WORKSPACE_ROOT="$locked_workspace_root"
    # The root pass pre-creates the workspace root only inside a directory it
    # manages -- $IRONCLAW_REBORN_HOME or the Railway volume mount -- so that a
    # raw override can never have an arbitrary path chowned to the runtime uid
    # before the containment check (which runs after the privilege drop) can
    # reject it. This case's locked parent IS that volume mount, which is the
    # scenario the fix exists for.
    export RAILWAY_VOLUME_MOUNT_PATH="$locked_parent"
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID
    sh "$ENTRYPOINT" >/dev/null 2>"${workspace_root_privdrop_home}/entrypoint.err"
  )
}

rm -f "${workspace_root_privdrop_home}/argv"
if ! run_workspace_root_privdrop; then
  echo "FAIL[workspace_root_privdrop]: entrypoint aborted instead of creating the workspace root while still root" >&2
  cat "${workspace_root_privdrop_home}/entrypoint.err" >&2
  failures=$((failures + 1))
elif [ ! -d "$locked_workspace_root" ]; then
  echo "FAIL[workspace_root_privdrop]: workspace root was never created: $locked_workspace_root" >&2
  failures=$((failures + 1))
elif ! grep -q "$locked_workspace_root" "$workspace_root_chown_record"; then
  echo "FAIL[workspace_root_privdrop]: workspace root was not chowned during the root pass" >&2
  cat "$workspace_root_chown_record" >&2
  failures=$((failures + 1))
fi

# 6b. The root pass must NEVER chown a workspace root that lies outside the
#     directories this entrypoint manages. The Railway containment check that
#     validates an operator-supplied override runs only after the privilege
#     drop, so a root-run `chown` of the raw value would hand the runtime uid
#     ownership of an arbitrary path -- `IRONCLAW_REBORN_WORKSPACE_ROOT=/etc`
#     chowning /etc to ironclaw before anything rejects it. The `..` spelling
#     below also pins that the comparison is canonicalized, not lexical.
outside_root_home="${WORK}/outside-root"
mkdir -p "$outside_root_home"
printf '%s' "$LEGACY_DISABLED" > "${outside_root_home}/config.toml"
outside_mount="${WORK}/outside-mount"
mkdir -p "$outside_mount"
outside_chown_record="${outside_root_home}/chown-argv"

run_outside_workspace_root() {
  (
    export PATH="${WORK}/root-bin:${WORK}/bin:${PATH}"
    export IRONCLAW_REBORN_HOME="$outside_root_home"
    export IRONCLAW_REBORN_DEFAULT_CONFIG=/opt/ironclaw/reborn/config.toml
    export IRONCLAW_STUB_ARGV_PATH="${outside_root_home}/argv"
    export IRONCLAW_STUB_WORKSPACE_PATH="${outside_root_home}/workspace-root"
    export IRONCLAW_STUB_CHOWN_PATH="$outside_chown_record"
    export IRONCLAW_STUB_UID=0
    export RAILWAY_VOLUME_MOUNT_PATH="$outside_mount"
    export IRONCLAW_REBORN_WORKSPACE_ROOT="$1"
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID
    sh "$ENTRYPOINT" >/dev/null 2>"${outside_root_home}/entrypoint.err"
  )
}

for outside_case in "${WORK}/not-managed-at-all" "${outside_mount}/../not-managed-via-dotdot"; do
  rm -f "$outside_chown_record"
  run_outside_workspace_root "$outside_case" || true
  if [ -f "$outside_chown_record" ] && grep -q "not-managed" "$outside_chown_record"; then
    echo "FAIL[workspace_root_outside_managed]: root pass chowned an unvalidated workspace root: $outside_case" >&2
    cat "$outside_chown_record" >&2
    failures=$((failures + 1))
  fi
done

# 7. The Railway containment guard must canonicalize paths even when an
#    intermediate component does not exist yet. GNU `readlink -f` requires
#    every component but the last to already exist and otherwise fails
#    outright -- falling back to the raw, unresolved spelling, which then
#    lexically (but wrongly) matches the mount-prefix check even though the
#    path actually resolves outside the mount via `..`. `missing-intermediate`
#    below is deliberately never created.
railway_escape_missing_root="${railway_mount}/missing-intermediate/../../tmp-outside-mount"

rm -f "${railway_home}/argv"
if run_railway_entrypoint "$railway_escape_missing_root"; then
  echo "FAIL[railway_escape_missing_intermediate]: a workspace root with a non-existent intermediate component and a '..' escape was accepted" >&2
  failures=$((failures + 1))
elif ! grep -q 'IRONCLAW_REBORN_WORKSPACE_ROOT' "${railway_home}/railway.err"; then
  echo "FAIL[railway_escape_missing_intermediate]: expected a workspace-root containment diagnostic" >&2
  cat "${railway_home}/railway.err" >&2
  failures=$((failures + 1))
fi

# 8/9. A bare "/" for either root path must be refused with a diagnostic.
#      `${VAR%/}` turns the single-character input "/" into "", which used to
#      reach `mkdir -p "" /workspace` (a confusing "cannot create directory ''"
#      failure) for IRONCLAW_REBORN_HOME, and `readlink -m ""` for
#      IRONCLAW_REBORN_WORKSPACE_ROOT -- the latter exits non-zero with NO
#      output at all, so `set -eu` killed the boot silently, the worse half of
#      the bug.
#
#      These are refused rather than normalized back to "/". The root pass
#      chowns IRONCLAW_REBORN_HOME to the unprivileged runtime user, so
#      honoring "/" would hand uid 1000 ownership of the container's entire
#      root filesystem -- strictly worse than the crash it replaced. Nothing
#      legitimately runs with either path set to the filesystem root, so the
#      safe reading of "/" is "operator error, stop and say so".
slash_root_home="${WORK}/slash-root"
mkdir -p "$slash_root_home"
printf '%s' "$LEGACY_DISABLED" > "${slash_root_home}/config.toml"
slash_root_chown_record="${slash_root_home}/chown-argv"

run_slash_root() { # $1 = variable name to set to "/"
  (
    export PATH="${WORK}/root-bin:${WORK}/bin:${PATH}"
    export IRONCLAW_REBORN_HOME="$slash_root_home"
    export IRONCLAW_REBORN_DEFAULT_CONFIG=/opt/ironclaw/reborn/config.toml
    export IRONCLAW_STUB_ARGV_PATH="${slash_root_home}/argv"
    export IRONCLAW_STUB_WORKSPACE_PATH="${slash_root_home}/workspace-root"
    export IRONCLAW_STUB_UID=0
    export IRONCLAW_STUB_CHOWN_PATH="$slash_root_chown_record"
    export IRONCLAW_STUB_GOSU_PATH="${slash_root_home}/gosu-argv"
    unset IRONCLAW_REBORN_WORKSPACE_ROOT
    unset RAILWAY_ENVIRONMENT RAILWAY_PROJECT_ID RAILWAY_SERVICE_ID RAILWAY_VOLUME_MOUNT_PATH
    export "$1=/"
    sh "$ENTRYPOINT" >/dev/null 2>"${slash_root_home}/entrypoint.err"
  )
}

for slash_var in IRONCLAW_REBORN_HOME IRONCLAW_REBORN_WORKSPACE_ROOT; do
  rm -f "${slash_root_home}/argv" "$slash_root_chown_record"
  touch "$slash_root_chown_record"
  if run_slash_root "$slash_var"; then
    echo "FAIL[slash_root]: ${slash_var}=/ was accepted instead of refused" >&2
    failures=$((failures + 1))
  elif ! grep -q "${slash_var} must not be the filesystem root" "${slash_root_home}/entrypoint.err"; then
    # Catches BOTH original failure modes: the blank-operand mkdir error, and
    # the silent `readlink -m ""` death that printed nothing whatsoever.
    echo "FAIL[slash_root]: ${slash_var}=/ did not produce the expected diagnostic" >&2
    cat "${slash_root_home}/entrypoint.err" >&2
    failures=$((failures + 1))
  fi
  # Nothing may be chowned on the way to that refusal -- least of all "/".
  if [ -s "$slash_root_chown_record" ]; then
    echo "FAIL[slash_root]: ${slash_var}=/ reached a chown before being refused" >&2
    cat "$slash_root_chown_record" >&2
    failures=$((failures + 1))
  fi
done

# 10. The dead variable has no reader left anywhere in the tree.
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
