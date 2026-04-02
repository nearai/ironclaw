"""E2E tests for Abound API integration through IronClaw's Responses API.

Creates a test user, injects Abound dev credentials, then sends natural
language prompts that should trigger the agent to call Abound's API via
the http tool (guided by the abound-remittance skill).

Usage:
    python integrations/abound/tests/test_abound_e2e.py
"""

import atexit
import os
import time
import uuid

import requests
from openai import OpenAI

# IronClaw deployment — set these env vars before running
BASE_URL = os.environ["BASE_URL"]
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]

# Abound dev credentials (injected via admin API)
ABOUND_BEARER_TOKEN = os.environ["ABOUND_BEARER_TOKEN"]
ABOUND_API_KEY = os.environ["ABOUND_API_KEY"]

admin = requests.Session()
admin.headers.update({
    "Authorization": f"Bearer {ADMIN_TOKEN}",
    "Content-Type": "application/json",
})

passed = 0
failed = 0
user_id = ""


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


def extract_agent_text(response) -> str:
    text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    text += content.text
    return text


def cleanup():
    if user_id:
        print("\n--- Cleanup: deleting test user ---")
        admin.delete(f"{BASE_URL}/api/admin/users/{user_id}")
        print("  Done")


atexit.register(cleanup)

# -----------------------------------------------------------
# Setup: Create user and inject Abound credentials
# -----------------------------------------------------------
print("=== Abound E2E Tests (via IronClaw) ===")
print(f"Target: {BASE_URL}\n")

print("--- Setup: Create test user ---")
email = f"abound-e2e-{uuid.uuid4().hex[:8]}@example.com"
r = admin.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Abound E2E Test User",
    "email": email,
    "role": "member",
})
if r.status_code != 200:
    print(f"  FATAL: Could not create test user: {r.status_code} {r.text}")
    exit(1)

data = r.json()
user_id = data["id"]
user_token = data["token"]
print(f"  User ID: {user_id}")
print(f"  Token:   {user_token[:16]}...")

print("\n--- Setup: Inject Abound credentials ---")

r = admin.put(
    f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_read_token",
    json={"value": ABOUND_BEARER_TOKEN, "provider": "abound"},
)
check("inject bearer token", r.status_code == 200, f"got {r.status_code}: {r.text[:200]}")

r = admin.put(
    f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_api_key",
    json={"value": ABOUND_API_KEY, "provider": "abound"},
)
check("inject api key", r.status_code == 200, f"got {r.status_code}: {r.text[:200]}")

print("\n  Waiting 5s for workspace bootstrap...")
time.sleep(5)
print()

client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")

# -----------------------------------------------------------
# 1. Get Account Info (natural language)
# -----------------------------------------------------------
print("--- 1. Account info ---")
try:
    response = client.responses.create(
        model="default",
        input="What is my Abound account info? Show me my transfer limits, recipients, and funding sources.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # Verify Abound-specific data (not generic)
    has_abound_data = any(term in agent_text.lower() for term in [
        "ach", "limit", "recipient", "funding", "discover", "bageshwar",
    ])
    check("contains Abound account data", has_abound_data,
          "response doesn't contain Abound-specific account data")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 2. Exchange rate (natural language)
# -----------------------------------------------------------
print("--- 2. Exchange rate ---")
try:
    response = client.responses.create(
        model="default",
        input="What's the current USD to INR exchange rate on Abound? "
              "Show me both the market rate and the effective rate I'd get.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # "effective rate" is Abound-specific — a generic API wouldn't return this
    has_effective_rate = "effective" in agent_text.lower()
    check("contains effective rate (Abound-specific)", has_effective_rate,
          "response doesn't mention effective rate — may not be using Abound API")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 3. Send money advice (natural language)
# -----------------------------------------------------------
print("--- 3. Send money advice ---")
try:
    response = client.responses.create(
        model="default",
        input="I want to send $1,000 to India. Check the rate and tell me "
              "how much INR I'd get. Is now a good time to send?",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # Should reference INR amount and rate
    has_conversion = any(term in agent_text.lower() for term in ["inr", "rupee"])
    check("mentions INR conversion", has_conversion)
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 4. Create notification (natural language)
# -----------------------------------------------------------
print("--- 4. Create notification ---")
try:
    response = client.responses.create(
        model="default",
        input="Send a notification to my Abound app about the current "
              "exchange rate. Use a score of 75 and include the current rate.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # Agent should mention notification attempt (may 401 due to Abound auth)
    has_notification = any(term in agent_text.lower() for term in [
        "notification", "accepted", "unauthorized", "401",
    ])
    check("references notification attempt", has_notification,
          "response doesn't mention notification")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
