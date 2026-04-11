#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IRONCLAW_BIN="${IRONCLAW_BIN:-$ROOT_DIR/target/debug/ironclaw}"
MOCK_LLM_SCRIPT="${MOCK_LLM_SCRIPT:-$ROOT_DIR/tests/e2e/mock_llm.py}"
SMOKE_PROMPT="${SMOKE_PROMPT:-hello from standby smoke}"
STANDBY_TOKEN="${STANDBY_TOKEN:-standby-token}"

find_free_port() {
  python3 - <<'PY'
import socket

s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_for_http_200() {
  local url=$1
  local attempts="${2:-40}"

  for _ in $(seq 1 "$attempts"); do
    if curl -sf "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done

  return 1
}

http_status() {
  local output_file=$1
  shift
  curl -sS -o "$output_file" -w "%{http_code}" "$@"
}

json_get() {
  local file=$1
  local expr=$2
  python3 - "$file" "$expr" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1]))
expr = sys.argv[2]

if expr == "thread_id":
    print(payload["id"])
elif expr == "history_response":
    turns = payload.get("turns", [])
    print(turns[0].get("response", "") if turns else "")
elif expr == "history_state":
    turns = payload.get("turns", [])
    print(turns[0].get("state", "") if turns else "")
else:
    raise SystemExit(f"unsupported expr: {expr}")
PY
}

fail() {
  echo "[standby-smoke] $*" >&2
  exit 1
}

if [[ ! -x "$IRONCLAW_BIN" ]]; then
  fail "ironclaw binary not found or not executable: $IRONCLAW_BIN"
fi

if [[ ! -f "$MOCK_LLM_SCRIPT" ]]; then
  fail "mock llm script not found: $MOCK_LLM_SCRIPT"
fi

WORK_DIR="$(mktemp -d /tmp/ironclaw-standby-smoke-XXXXXX)"
HOME_DIR="$WORK_DIR/home"
MOCK_LOG="$WORK_DIR/mock-llm.log"
STANDBY_LOG="$WORK_DIR/standby.log"
CONFIGURE_JSON="$WORK_DIR/configure.json"
THREAD_JSON="$WORK_DIR/thread.json"
SEND_JSON="$WORK_DIR/send.json"
HISTORY_JSON="$WORK_DIR/history.json"
mkdir -p "$HOME_DIR"
SUCCESS=0

MOCK_LLM_PORT="${MOCK_LLM_PORT:-$(find_free_port)}"
GATEWAY_PORT="${GATEWAY_PORT:-$(find_free_port)}"
WEBHOOK_SERVER_PORT="${WEBHOOK_SERVER_PORT:-$(find_free_port)}"
ORCHESTRATOR_PORT="${ORCHESTRATOR_PORT:-$(find_free_port)}"

cleanup() {
  if [[ -n "${STANDBY_PID:-}" ]]; then
    kill "$STANDBY_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "${MOCK_PID:-}" ]]; then
    kill "$MOCK_PID" >/dev/null 2>&1 || true
  fi
  wait "${STANDBY_PID:-}" >/dev/null 2>&1 || true
  wait "${MOCK_PID:-}" >/dev/null 2>&1 || true
  if [[ "$SUCCESS" == "1" ]]; then
    rm -rf "$WORK_DIR"
  else
    echo "[standby-smoke] logs preserved at $WORK_DIR" >&2
  fi
}
trap cleanup EXIT

python3 "$MOCK_LLM_SCRIPT" --port "$MOCK_LLM_PORT" >"$MOCK_LOG" 2>&1 &
MOCK_PID=$!
wait_for_http_200 "http://127.0.0.1:${MOCK_LLM_PORT}/v1/models" 40 || {
  cat "$MOCK_LOG" >&2 || true
  fail "mock llm did not become ready"
}

cat >"$CONFIGURE_JSON" <<JSON
{
  "agentId": "00000000-0000-0000-0000-000000000000",
  "llm": {
    "backend": "openai",
    "model": "gpt-4.1",
    "apiKey": "dummy-key",
    "baseUrl": "http://127.0.0.1:${MOCK_LLM_PORT}/v1"
  },
  "mcpServers": [],
  "channels": [],
  "persona": {
    "soul": "helpful",
    "parameters": {
      "identity": "standby smoke",
      "instructions": "answer briefly"
    },
    "skills": []
  }
}
JSON

env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY -u all_proxy -u ALL_PROXY -u no_proxy -u NO_PROXY \
  HOME="$HOME_DIR" \
  IRONCLAW_OWNER_ID=standby-smoke \
  GATEWAY_HOST=127.0.0.1 \
  GATEWAY_PORT="$GATEWAY_PORT" \
  GATEWAY_AUTH_TOKEN="$STANDBY_TOKEN" \
  WEBHOOK_SERVER_PORT="$WEBHOOK_SERVER_PORT" \
  ORCHESTRATOR_PORT="$ORCHESTRATOR_PORT" \
  "$IRONCLAW_BIN" --standby --no-onboard --no-db >"$STANDBY_LOG" 2>&1 &
STANDBY_PID=$!

wait_for_http_200 "http://127.0.0.1:${GATEWAY_PORT}/api/health" 80 || {
  cat "$STANDBY_LOG" >&2 || true
  fail "standby gateway did not become healthy"
}

unauth_status="$(http_status "$WORK_DIR/configure-unauth.txt" \
  -X POST "http://127.0.0.1:${GATEWAY_PORT}/api/configure" \
  -H 'Content-Type: application/json' \
  --data-binary "@$CONFIGURE_JSON")"
[[ "$unauth_status" == "401" ]] || fail "expected unauth configure=401, got ${unauth_status}"

auth_status="$(http_status "$WORK_DIR/configure-auth.txt" \
  -X POST "http://127.0.0.1:${GATEWAY_PORT}/api/configure" \
  -H "Authorization: Bearer ${STANDBY_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data-binary "@$CONFIGURE_JSON")"
[[ "$auth_status" == "200" ]] || {
  cat "$STANDBY_LOG" >&2 || true
  fail "expected auth configure=200, got ${auth_status}"
}

thread_status="$(http_status "$THREAD_JSON" \
  -X POST "http://127.0.0.1:${GATEWAY_PORT}/api/chat/thread/new" \
  -H "Authorization: Bearer ${STANDBY_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data '{}')"
[[ "$thread_status" == "200" ]] || fail "expected thread create=200, got ${thread_status}"

THREAD_ID="$(json_get "$THREAD_JSON" "thread_id")"

send_status="$(http_status "$SEND_JSON" \
  -X POST "http://127.0.0.1:${GATEWAY_PORT}/api/chat/send" \
  -H "Authorization: Bearer ${STANDBY_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data "{\"content\":\"${SMOKE_PROMPT}\",\"thread_id\":\"${THREAD_ID}\"}")"
[[ "$send_status" == "202" ]] || fail "expected send=202, got ${send_status}"

for _ in $(seq 1 20); do
  curl -sS "http://127.0.0.1:${GATEWAY_PORT}/api/chat/history?thread_id=${THREAD_ID}" \
    -H "Authorization: Bearer ${STANDBY_TOKEN}" \
    >"$HISTORY_JSON"
  if [[ "$(json_get "$HISTORY_JSON" "history_state")" == "Completed" ]]; then
    break
  fi
  sleep 0.5
done

history_state="$(json_get "$HISTORY_JSON" "history_state")"
history_response="$(json_get "$HISTORY_JSON" "history_response")"

[[ "$history_state" == "Completed" ]] || {
  cat "$STANDBY_LOG" >&2 || true
  fail "expected history state=Completed, got ${history_state}"
}
[[ -n "$history_response" ]] || fail "expected non-empty assistant response"

SUCCESS=1
echo "[standby-smoke] ok"
echo "[standby-smoke] gateway_port=${GATEWAY_PORT}"
echo "[standby-smoke] webhook_server_port=${WEBHOOK_SERVER_PORT}"
echo "[standby-smoke] orchestrator_port=${ORCHESTRATOR_PORT}"
echo "[standby-smoke] response=${history_response}"
