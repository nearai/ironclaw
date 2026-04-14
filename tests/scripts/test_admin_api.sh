#!/usr/bin/env bash
# Test the Admin API (user CRUD, secrets) against a live deployment.
#
# Usage:
#   export BASE_URL=https://ironclaw-production-e3b1.up.railway.app
#   export ADMIN_TOKEN=your-admin-token
#   bash tests/scripts/test_admin_api.sh

set -euo pipefail

: "${BASE_URL:?Set BASE_URL to the Railway deployment URL}"
: "${ADMIN_TOKEN:?Set ADMIN_TOKEN to the admin bearer token}"

PASS=0
FAIL=0

check() {
  local name="$1" expected="$2" actual="$3"
  if echo "$actual" | grep -q "$expected"; then
    echo "  PASS: $name"
    PASS=$((PASS + 1))
  else
    echo "  FAIL: $name (expected '$expected' in response)"
    echo "    Got: $actual"
    FAIL=$((FAIL + 1))
  fi
}

auth=(-H "Authorization: Bearer $ADMIN_TOKEN" -H "Content-Type: application/json")

echo "=== Admin API Tests ==="
echo "Target: $BASE_URL"
echo ""

# -------------------------------------------------------
# 1. Create a test user
# -------------------------------------------------------
echo "--- 1. Create user ---"
CREATE_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/api/admin/users" \
  "${auth[@]}" \
  -d '{"display_name": "Test User", "email": "test-demo@example.com", "role": "member"}')
CREATE_BODY=$(echo "$CREATE_RESP" | head -n -1)
CREATE_STATUS=$(echo "$CREATE_RESP" | tail -1)

check "status 200" "200" "$CREATE_STATUS"
check "has id" '"id"' "$CREATE_BODY"
check "has token" '"token"' "$CREATE_BODY"
check "status active" '"status":"active"' "$CREATE_BODY"

USER_ID=$(echo "$CREATE_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])" 2>/dev/null || echo "")
USER_TOKEN=$(echo "$CREATE_BODY" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null || echo "")

if [ -z "$USER_ID" ]; then
  echo "  FATAL: Could not extract user ID. Response: $CREATE_BODY"
  exit 1
fi
echo "  User ID: $USER_ID"
echo "  Token:   ${USER_TOKEN:0:16}..."
echo ""

# -------------------------------------------------------
# 2. List users
# -------------------------------------------------------
echo "--- 2. List users ---"
LIST_RESP=$(curl -s -w "\n%{http_code}" -X GET "${BASE_URL}/api/admin/users" "${auth[@]}")
LIST_BODY=$(echo "$LIST_RESP" | head -n -1)
LIST_STATUS=$(echo "$LIST_RESP" | tail -1)

check "status 200" "200" "$LIST_STATUS"
check "contains new user" "$USER_ID" "$LIST_BODY"
echo ""

# -------------------------------------------------------
# 3. Get user detail
# -------------------------------------------------------
echo "--- 3. Get user detail ---"
DETAIL_RESP=$(curl -s -w "\n%{http_code}" -X GET "${BASE_URL}/api/admin/users/${USER_ID}" "${auth[@]}")
DETAIL_BODY=$(echo "$DETAIL_RESP" | head -n -1)
DETAIL_STATUS=$(echo "$DETAIL_RESP" | tail -1)

check "status 200" "200" "$DETAIL_STATUS"
check "display_name" "Test User" "$DETAIL_BODY"
check "email" "test-demo@example.com" "$DETAIL_BODY"
echo ""

# -------------------------------------------------------
# 4. Update user
# -------------------------------------------------------
echo "--- 4. Update user ---"
UPDATE_RESP=$(curl -s -w "\n%{http_code}" -X PATCH "${BASE_URL}/api/admin/users/${USER_ID}" \
  "${auth[@]}" \
  -d '{"display_name": "Updated User", "metadata": {"abound_user_id": "abound-ref-123"}}')
UPDATE_BODY=$(echo "$UPDATE_RESP" | head -n -1)
UPDATE_STATUS=$(echo "$UPDATE_RESP" | tail -1)

check "status 200" "200" "$UPDATE_STATUS"
check "updated name" "Updated User" "$UPDATE_BODY"
check "metadata" "abound-ref-123" "$UPDATE_BODY"
echo ""

# -------------------------------------------------------
# 5. Suspend user
# -------------------------------------------------------
echo "--- 5. Suspend user ---"
SUSPEND_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/api/admin/users/${USER_ID}/suspend" "${auth[@]}")
SUSPEND_BODY=$(echo "$SUSPEND_RESP" | head -n -1)
SUSPEND_STATUS=$(echo "$SUSPEND_RESP" | tail -1)

check "status 200" "200" "$SUSPEND_STATUS"
check "suspended" '"suspended"' "$SUSPEND_BODY"
echo ""

# -------------------------------------------------------
# 6. Activate user
# -------------------------------------------------------
echo "--- 6. Activate user ---"
ACTIVATE_RESP=$(curl -s -w "\n%{http_code}" -X POST "${BASE_URL}/api/admin/users/${USER_ID}/activate" "${auth[@]}")
ACTIVATE_BODY=$(echo "$ACTIVATE_RESP" | head -n -1)
ACTIVATE_STATUS=$(echo "$ACTIVATE_RESP" | tail -1)

check "status 200" "200" "$ACTIVATE_STATUS"
check "active" '"active"' "$ACTIVATE_BODY"
echo ""

# -------------------------------------------------------
# 7. Inject a secret (external token)
# -------------------------------------------------------
echo "--- 7. Inject secret ---"
SECRET_RESP=$(curl -s -w "\n%{http_code}" -X PUT \
  "${BASE_URL}/api/admin/users/${USER_ID}/secrets/abound_external_token" \
  "${auth[@]}" \
  -d '{"value": "fake-abound-token-for-demo", "provider": "abound", "expires_in_days": 90}')
SECRET_BODY=$(echo "$SECRET_RESP" | head -n -1)
SECRET_STATUS=$(echo "$SECRET_RESP" | tail -1)

check "status 200" "200" "$SECRET_STATUS"
check "secret created" "abound_external_token" "$SECRET_BODY"
echo ""

# -------------------------------------------------------
# 8. List secrets
# -------------------------------------------------------
echo "--- 8. List secrets ---"
LIST_SEC_RESP=$(curl -s -w "\n%{http_code}" -X GET \
  "${BASE_URL}/api/admin/users/${USER_ID}/secrets" "${auth[@]}")
LIST_SEC_BODY=$(echo "$LIST_SEC_RESP" | head -n -1)
LIST_SEC_STATUS=$(echo "$LIST_SEC_RESP" | tail -1)

check "status 200" "200" "$LIST_SEC_STATUS"
check "secret listed" "abound_external_token" "$LIST_SEC_BODY"
check "provider abound" "abound" "$LIST_SEC_BODY"
echo ""

# -------------------------------------------------------
# 9. Delete secret
# -------------------------------------------------------
echo "--- 9. Delete secret ---"
DEL_SEC_RESP=$(curl -s -w "\n%{http_code}" -X DELETE \
  "${BASE_URL}/api/admin/users/${USER_ID}/secrets/abound_external_token" "${auth[@]}")
DEL_SEC_BODY=$(echo "$DEL_SEC_RESP" | head -n -1)
DEL_SEC_STATUS=$(echo "$DEL_SEC_RESP" | tail -1)

check "status 200" "200" "$DEL_SEC_STATUS"
check "deleted" '"deleted":true' "$DEL_SEC_BODY"
echo ""

# -------------------------------------------------------
# 10. Delete user
# -------------------------------------------------------
echo "--- 10. Delete user ---"
DEL_RESP=$(curl -s -w "\n%{http_code}" -X DELETE "${BASE_URL}/api/admin/users/${USER_ID}" "${auth[@]}")
DEL_BODY=$(echo "$DEL_RESP" | head -n -1)
DEL_STATUS=$(echo "$DEL_RESP" | tail -1)

check "status 200" "200" "$DEL_STATUS"
check "deleted" '"deleted":true' "$DEL_BODY"
echo ""

# -------------------------------------------------------
# 11. Verify user is gone
# -------------------------------------------------------
echo "--- 11. Verify deleted ---"
GONE_RESP=$(curl -s -w "\n%{http_code}" -X GET "${BASE_URL}/api/admin/users/${USER_ID}" "${auth[@]}")
GONE_STATUS=$(echo "$GONE_RESP" | tail -1)

check "404 not found" "404" "$GONE_STATUS"
echo ""

# -------------------------------------------------------
# Summary
# -------------------------------------------------------
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
