#!/usr/bin/env bash
# Test the Responses API (/v1/responses) against a live deployment.
# Creates a test user, sends requests with the user token, then cleans up.
#
# Usage:
#   export BASE_URL=https://ironclaw-production-e3b1.up.railway.app
#   export ADMIN_TOKEN=your-admin-token
#   bash tests/scripts/test_responses_api.sh

set -euo pipefail

: "${BASE_URL:?Set BASE_URL to the Railway deployment URL}"
: "${ADMIN_TOKEN:?Set ADMIN_TOKEN to the admin bearer token}"

PASS=0
FAIL=0
USER_ID=""

check() {
  local name="$1" expected="$2" actual="$3"
  if echo "$actual" | grep -q "$expected"; then
    echo "  PASS: $name"
    PASS=$((PASS + 1))
  else
    echo "  FAIL: $name (expected '$expected' in response)"
    echo "    Got: $(echo "$actual" | head -c 500)"
    FAIL=$((FAIL + 1))
  fi
}

cleanup() {
  if [ -n "$USER_ID" ]; then
    echo ""
    echo "--- Cleanup: deleting test user ---"
    curl -s -X DELETE "${BASE_URL}/api/admin/users/${USER_ID}" \
      -H "Authorization: Bearer $ADMIN_TOKEN" \
      -H "Content-Type: application/json" > /dev/null 2>&1 || true
    echo "  Done"
  fi
}
trap cleanup EXIT

admin_auth=(-H "Authorization: Bearer $ADMIN_TOKEN" -H "Content-Type: application/json")

echo "=== Responses API Tests ==="
echo "Target: $BASE_URL"
echo ""

# -------------------------------------------------------
# Setup: Create a test user
# -------------------------------------------------------
echo "--- Setup: Create test user ---"
CREATE_RESP=$(curl -s -X POST "${BASE_URL}/api/admin/users" \
  "${admin_auth[@]}" \
  -d '{"display_name": "Responses Test User", "email": "resp-test@example.com", "role": "member"}')

USER_ID=$(echo "$CREATE_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null || echo "")
USER_TOKEN=$(echo "$CREATE_RESP" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || echo "")

if [ -z "$USER_ID" ] || [ -z "$USER_TOKEN" ]; then
  echo "  FATAL: Could not create test user. Response: $CREATE_RESP"
  exit 1
fi
echo "  User ID: $USER_ID"
echo "  Token:   ${USER_TOKEN:0:16}..."
echo ""

user_auth=(-H "Authorization: Bearer $USER_TOKEN" -H "Content-Type: application/json")

# -------------------------------------------------------
# 1. Non-streaming response (simple text input)
# -------------------------------------------------------
echo "--- 1. Non-streaming response (text input) ---"
RESP1=$(curl -s -w "\n%{http_code}" --max-time 120 -X POST "${BASE_URL}/v1/responses" \
  "${user_auth[@]}" \
  -d '{"input": "Say hello in exactly 3 words", "stream": false}')
RESP1_BODY=$(echo "$RESP1" | head -n -1)
RESP1_STATUS=$(echo "$RESP1" | tail -1)

check "status 200" "200" "$RESP1_STATUS"
check "has response id" '"id":"resp_' "$RESP1_BODY"
check "status completed or failed" '"status"' "$RESP1_BODY"
check "has output" '"output"' "$RESP1_BODY"

RESPONSE_ID=$(echo "$RESP1_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null || echo "")
echo "  Response ID: ${RESPONSE_ID:-'(not found)'}"
echo ""

# -------------------------------------------------------
# 2. Non-streaming response (messages input)
# -------------------------------------------------------
echo "--- 2. Non-streaming response (messages input) ---"
RESP2=$(curl -s -w "\n%{http_code}" --max-time 120 -X POST "${BASE_URL}/v1/responses" \
  "${user_auth[@]}" \
  -d '{"input": [{"role": "user", "content": "What is 2+2? Reply with just the number."}], "stream": false}')
RESP2_BODY=$(echo "$RESP2" | head -n -1)
RESP2_STATUS=$(echo "$RESP2" | tail -1)

check "status 200" "200" "$RESP2_STATUS"
check "has output" '"output"' "$RESP2_BODY"
echo ""

# -------------------------------------------------------
# 3. Continue conversation (previous_response_id)
# -------------------------------------------------------
if [ -n "$RESPONSE_ID" ]; then
  echo "--- 3. Continue conversation ---"
  RESP3=$(curl -s -w "\n%{http_code}" --max-time 120 -X POST "${BASE_URL}/v1/responses" \
    "${user_auth[@]}" \
    -d "{\"input\": \"Now say goodbye in 3 words\", \"stream\": false, \"previous_response_id\": \"$RESPONSE_ID\"}")
  RESP3_BODY=$(echo "$RESP3" | head -n -1)
  RESP3_STATUS=$(echo "$RESP3" | tail -1)

  check "status 200" "200" "$RESP3_STATUS"
  check "has response" '"output"' "$RESP3_BODY"
  echo ""
else
  echo "--- 3. Continue conversation (SKIPPED - no response ID) ---"
  echo ""
fi

# -------------------------------------------------------
# 4. Get response by ID
# -------------------------------------------------------
if [ -n "$RESPONSE_ID" ]; then
  echo "--- 4. Get response by ID ---"
  GET_RESP=$(curl -s -w "\n%{http_code}" -X GET "${BASE_URL}/v1/responses/${RESPONSE_ID}" \
    "${user_auth[@]}")
  GET_BODY=$(echo "$GET_RESP" | head -n -1)
  GET_STATUS=$(echo "$GET_RESP" | tail -1)

  check "status 200" "200" "$GET_STATUS"
  check "same id" "$RESPONSE_ID" "$GET_BODY"
  check "has output" '"output"' "$GET_BODY"
  echo ""
else
  echo "--- 4. Get response by ID (SKIPPED - no response ID) ---"
  echo ""
fi

# -------------------------------------------------------
# 5. Streaming response
# -------------------------------------------------------
echo "--- 5. Streaming response ---"
echo "  (Collecting SSE events for up to 30s...)"
STREAM_OUT=$(curl -s --max-time 30 -N -X POST "${BASE_URL}/v1/responses" \
  "${user_auth[@]}" \
  -d '{"input": "Count from 1 to 3", "stream": true}' 2>/dev/null || true)

if [ -n "$STREAM_OUT" ]; then
  check "has SSE events" "event:" "$STREAM_OUT"
  check "has response.created" "response.created" "$STREAM_OUT"

  # Count events
  EVENT_COUNT=$(echo "$STREAM_OUT" | grep -c "^event:" || true)
  echo "  Received $EVENT_COUNT SSE events"

  # Show first few events
  echo "  First events:"
  echo "$STREAM_OUT" | grep "^event:" | head -5 | sed 's/^/    /'
else
  echo "  FAIL: No streaming output received"
  FAIL=$((FAIL + 1))
fi
echo ""

# -------------------------------------------------------
# 6. Error cases
# -------------------------------------------------------
echo "--- 6. Error: no auth ---"
NO_AUTH=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/v1/responses" \
  -H "Content-Type: application/json" \
  -d '{"input": "hello"}')
NO_AUTH_STATUS=$(echo "$NO_AUTH" | tail -1)
check "401 unauthorized" "401" "$NO_AUTH_STATUS"
echo ""

echo "--- 7. Error: empty input ---"
EMPTY=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/v1/responses" \
  "${user_auth[@]}" \
  -d '{"input": ""}')
EMPTY_STATUS=$(echo "$EMPTY" | tail -1)
check "400 bad request" "400" "$EMPTY_STATUS"
echo ""

echo "--- 8. Error: bad response ID ---"
BAD_ID=$(curl -s -w "\n%{http_code}" -X GET "${BASE_URL}/v1/responses/bad_id" \
  "${user_auth[@]}")
BAD_ID_STATUS=$(echo "$BAD_ID" | tail -1)
check "400 bad id" "400" "$BAD_ID_STATUS"
echo ""

# -------------------------------------------------------
# Summary
# -------------------------------------------------------
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
