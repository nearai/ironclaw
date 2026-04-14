"""Test the Admin API (user CRUD, secrets) against a live deployment.

Usage:
    python tests/scripts/test_admin_api.py
"""

import os
import uuid

import requests

BASE_URL = os.environ["BASE_URL"]
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]

session = requests.Session()
session.headers.update({
    "Authorization": f"Bearer {ADMIN_TOKEN}",
    "Content-Type": "application/json",
})

passed = 0
failed = 0
test_email = f"test-{uuid.uuid4().hex[:8]}@example.com"


def check(name: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}")
        if detail:
            print(f"    {detail[:500]}")
        failed += 1


# -----------------------------------------------------------
# 1. Create user
# -----------------------------------------------------------
print("--- 1. Create user ---")
r = session.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Test User",
    "email": test_email,
    "role": "member",
})
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
try:
    data = r.json()
except Exception:
    print(f"  FATAL: Non-JSON response: {r.text}")
    exit(1)
check("has id", "id" in data, str(data))
check("has token", "token" in data, str(data))
check("status active", data.get("status") == "active")

user_id = data.get("id", "")
user_token = data.get("token", "")
if not user_id:
    print(f"  FATAL: No user ID in response: {data}")
    exit(1)
print(f"  User ID: {user_id}")
print(f"  Token:   {user_token[:16]}...")
print()

# -----------------------------------------------------------
# 2. List users
# -----------------------------------------------------------
print("--- 2. List users ---")
r = session.get(f"{BASE_URL}/api/admin/users")
check("status 200", r.status_code == 200, f"got {r.status_code}")
data = r.json()
ids = [u["id"] for u in data.get("users", [])]
check("contains new user", user_id in ids, f"user IDs: {ids}")
print()

# -----------------------------------------------------------
# 3. Get user detail
# -----------------------------------------------------------
print("--- 3. Get user detail ---")
r = session.get(f"{BASE_URL}/api/admin/users/{user_id}")
check("status 200", r.status_code == 200, f"got {r.status_code}")
data = r.json()
check("display_name", data.get("display_name") == "Test User")
check("email", data.get("email") == test_email)
print()

# -----------------------------------------------------------
# 4. Update user
# -----------------------------------------------------------
print("--- 4. Update user ---")
r = session.patch(f"{BASE_URL}/api/admin/users/{user_id}", json={
    "display_name": "Updated User",
    "metadata": {"abound_user_id": "abound-ref-123"},
})
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json()
check("updated name", data.get("display_name") == "Updated User")
check("metadata", data.get("metadata", {}).get("abound_user_id") == "abound-ref-123")
print()

# -----------------------------------------------------------
# 5. Suspend user
# -----------------------------------------------------------
print("--- 5. Suspend user ---")
r = session.post(f"{BASE_URL}/api/admin/users/{user_id}/suspend")
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json()
check("suspended", data.get("status") == "suspended")
print()

# -----------------------------------------------------------
# 6. Activate user
# -----------------------------------------------------------
print("--- 6. Activate user ---")
r = session.post(f"{BASE_URL}/api/admin/users/{user_id}/activate")
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json()
check("active", data.get("status") == "active")
print()

# -----------------------------------------------------------
# 7. Inject secret
# -----------------------------------------------------------
print("--- 7. Inject secret ---")
r = session.put(f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_external_token", json={
    "value": "fake-abound-token-for-demo",
    "provider": "abound",
    "expires_in_days": 90,
})
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json() if r.status_code == 200 else {}
check("secret name", data.get("name") == "abound_external_token")
print()

# -----------------------------------------------------------
# 8. List secrets
# -----------------------------------------------------------
print("--- 8. List secrets ---")
r = session.get(f"{BASE_URL}/api/admin/users/{user_id}/secrets")
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json() if r.status_code == 200 else {}
secret_names = [s["name"] for s in data.get("secrets", [])]
check("secret listed", "abound_external_token" in secret_names, f"secrets: {secret_names}")
print()

# -----------------------------------------------------------
# 9. Delete secret
# -----------------------------------------------------------
print("--- 9. Delete secret ---")
r = session.delete(f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_external_token")
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json() if r.status_code == 200 else {}
check("deleted", data.get("deleted") is True)
print()

# -----------------------------------------------------------
# 10. Delete user
# -----------------------------------------------------------
print("--- 10. Delete user ---")
r = session.delete(f"{BASE_URL}/api/admin/users/{user_id}")
check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text}")
data = r.json()
check("deleted", data.get("deleted") is True)
print()

# -----------------------------------------------------------
# 11. Verify deleted
# -----------------------------------------------------------
print("--- 11. Verify deleted ---")
r = session.get(f"{BASE_URL}/api/admin/users/{user_id}")
check("404 not found", r.status_code == 404, f"got {r.status_code}")
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
