#!/usr/bin/env bash
#
# run_xyzorg.sh — one-shot: clean slate -> boot a real ironclaw -> run the
# standalone xyzorg capability-policy validator against it.
#
#   1. rm the local-dev DB (fresh directory: no users, no policy deltas)
#   2. cargo run -p ironclaw_reborn_cli --features webui-v2-beta,capability-policy
#        -- serve --port 3000   (with IRONCLAW_REBORN_CAPABILITY_POLICY=1)
#   3. python tests/e2e/test_reborn_capability_policy_xyzorg.py
#
# The owner is `director`, set ONLY via env (matoken / IRONCLAW_REBORN_WEBUI_USER_ID).
# This script reads those from the repo .env if not already exported.
#
# Env overrides:
#   IRONCLAW_REBORN_HOME   reborn home (default /tmp/ironclaw-reborn-home-tui)
#   PORT                   serve port (default 3000)
#   KEEP_SERVE=1           leave serve running after the test (default: stop it)
#
# Requires: a Python with the e2e extras (cd tests/e2e && pip install -e .).
# Set PYTHON=/path/to/venv/python to use a specific interpreter.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PORT="${PORT:-3000}"
BASE_URL="http://127.0.0.1:${PORT}"
REBORN_HOME="${IRONCLAW_REBORN_HOME:-/tmp/ironclaw-reborn-home-tui}"
DB="${REBORN_HOME}/local-dev/reborn-local-dev.db"
PYTHON="${PYTHON:-python3}"

# Resolve the owner identity ONCE (env override, else the FIRST exact-key match
# in .env) and pin it explicitly for BOTH serve and the validator. .env may carry
# a duplicate IRONCLAW_REBORN_WEBUI_USER_ID; pinning here makes the run
# deterministic regardless of which one the dotenv loader would pick.
env_first() {  # $1 = key -> first exact-key value from .env, or empty
  [ -f "${REPO_ROOT}/.env" ] || return 0
  sed -nE "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*//p" "${REPO_ROOT}/.env" \
    | head -n1 | sed -E 's/^"(.*)"$/\1/; s/[[:space:]]+$//'
}
OWNER_TOKEN="${IRONCLAW_REBORN_WEBUI_TOKEN:-$(env_first IRONCLAW_REBORN_WEBUI_TOKEN)}"
OWNER_USER_ID="${IRONCLAW_REBORN_WEBUI_USER_ID:-$(env_first IRONCLAW_REBORN_WEBUI_USER_ID)}"
OWNER_USER_ID="${OWNER_USER_ID:-director}"
if [ -z "${OWNER_TOKEN}" ]; then
  echo "ERROR: no owner bearer; set IRONCLAW_REBORN_WEBUI_TOKEN or put it in .env" >&2
  exit 2
fi
# Enabling an SSO provider (for the S6 mocked-SSO checks) makes
# IRONCLAW_REBORN_WEBUI_TOKEN double as the stateless SSO session-signing HMAC
# key, which the serve requires to be >=32 bytes. If the resolved owner token is
# shorter (e.g. a short local-dev token), pin a test signing key for THIS run so
# the serve boots and the forged-SSO bearers verify against the same key. serve +
# validator + forge all use this same value, so the owner identity stays aligned.
if [ "${#OWNER_TOKEN}" -lt 32 ]; then
  echo "  (owner token <32B; pinning a test SSO session-signing key for this run)"
  OWNER_TOKEN="ironclaw-reborn-xyzorg-e2e-sso-session-signing-key"
fi

echo "repo:        ${REPO_ROOT}"
echo "reborn home: ${REBORN_HOME}"
echo "serve url:   ${BASE_URL}"
echo "owner:       ${OWNER_USER_ID} (env bearer)"

# 1) Clean slate: drop the local-dev DB (users + policy deltas live here).
echo "[1/3] Dropping local-dev DB: ${DB}"
rm -f "${DB}" "${DB}-wal" "${DB}-shm"

# 2) Boot a real serve with the policy build + enforcement on. Owner identity is
#    pinned explicitly (overrides any .env value) so serve and the validator agree.
echo "[2/3] Starting serve (capability-policy)..."
SERVE_LOG="$(mktemp -t xyzorg-serve.XXXXXX.log)"
(
  cd "${REPO_ROOT}" || exit 1
  IRONCLAW_REBORN_HOME="${REBORN_HOME}" \
  IRONCLAW_REBORN_CAPABILITY_POLICY=1 \
  IRONCLAW_REBORN_WEBUI_TOKEN="${OWNER_TOKEN}" \
  IRONCLAW_REBORN_WEBUI_USER_ID="${OWNER_USER_ID}" \
  RUST_LOG="${RUST_LOG:-ironclaw_reborn=debug}" \
  IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID="${IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID:-dummy-sso-client-id}" \
  IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET="${IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET:-dummy-sso-client-secret}" \
  IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS="${IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS:-xyzorg.com,example.com}" \
  IRONCLAW_REBORN_WEBUI_SSO_EMAIL_KEYED="${IRONCLAW_REBORN_WEBUI_SSO_EMAIL_KEYED:-1}" \
    cargo run -p ironclaw_reborn_cli \
      --features webui-v2-beta,capability-policy \
      -- serve --host 127.0.0.1 --port "${PORT}"
) >"${SERVE_LOG}" 2>&1 &
SERVE_PID=$!

cleanup() {
  if [ "${KEEP_SERVE:-0}" != "1" ] && kill -0 "${SERVE_PID}" 2>/dev/null; then
    echo "Stopping serve (pid ${SERVE_PID})..."
    kill -INT "${SERVE_PID}" 2>/dev/null
    wait "${SERVE_PID}" 2>/dev/null
  else
    [ "${KEEP_SERVE:-0}" = "1" ] && echo "Leaving serve running (pid ${SERVE_PID}); log: ${SERVE_LOG}"
  fi
}
trap cleanup EXIT

# Wait for health (first build can take a few minutes).
echo "    waiting for ${BASE_URL}/api/health (up to 600s; building if needed)..."
ready=0
for _ in $(seq 1 600); do
  if curl -sS -m2 -o /dev/null -w '' "${BASE_URL}/api/health" 2>/dev/null; then
    ready=1; break
  fi
  if ! kill -0 "${SERVE_PID}" 2>/dev/null; then
    echo "ERROR: serve exited before becoming ready. Last log lines:" >&2
    tail -30 "${SERVE_LOG}" >&2
    exit 2
  fi
  sleep 1
done
if [ "${ready}" != "1" ]; then
  echo "ERROR: serve did not become ready in time. Last log lines:" >&2
  tail -30 "${SERVE_LOG}" >&2
  exit 2
fi
echo "    serve is up."

# 3) Run the standalone validator against the live instance (same pinned owner).
echo "[3/3] Running validator..."
BASE_URL="${BASE_URL}" \
IRONCLAW_REBORN_WEBUI_TOKEN="${OWNER_TOKEN}" \
IRONCLAW_REBORN_WEBUI_USER_ID="${OWNER_USER_ID}" \
SERVE_LOG="${SERVE_LOG}" \
  "${PYTHON}" "${REPO_ROOT}/tests/e2e/test_reborn_capability_policy_xyzorg.py"
status=$?

echo "validator exit: ${status}  (serve log: ${SERVE_LOG})"
exit "${status}"
