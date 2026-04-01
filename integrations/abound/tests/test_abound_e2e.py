"""E2E tests for Abound API integration through IronClaw's Responses API.

Creates a test user, injects Abound dev credentials, then sends prompts
that should trigger the agent to call Abound's API via the http tool.

Usage:
    python tests/scripts/test_abound_e2e.py
"""

import atexit
import os
import time
import uuid

import requests
from openai import OpenAI

# IronClaw deployment
BASE_URL = os.environ.get("BASE_URL", "https://ironclaw-production-e3b1.up.railway.app")
ADMIN_TOKEN = os.environ.get("ADMIN_TOKEN", "39a5644953ff8edf2df5c56fcfc7027e3392000381d5d8157552c1a51bee4cca")

# Abound dev credentials (injected via admin API, not hardcoded in agent)
ABOUND_BEARER_TOKEN = os.environ.get(
    "ABOUND_BEARER_TOKEN",
    "eyJhbGciOiJIUzM4NCJ9.eyJleHAiOjE3Nzc2MzM3MjUsImN1c3RvbWVyLWlkIjoiYTk0MTkyNTAtZWRlNy00MWEwLWE0MjItN2Y0ZTZmNDMzNjVmLTE3Njg0NjgzODUzMTgifQ.g3SDYkF2ns4GI3eo-l2f1OI23QN6gtoKNdUrNZfiVLvyOROEIivZ7pkp_NQDylZ4",
)
ABOUND_API_KEY = os.environ.get("ABOUND_API_KEY", "a105acd4-74f6-46b6-b429-c2b764462b99")

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

# Inject bearer token
r = admin.put(
    f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_external_token",
    json={"value": ABOUND_BEARER_TOKEN, "provider": "abound"},
)
check("inject bearer token", r.status_code == 200, f"got {r.status_code}: {r.text[:200]}")

# Inject API key
r = admin.put(
    f"{BASE_URL}/api/admin/users/{user_id}/secrets/abound_api_key",
    json={"value": ABOUND_API_KEY, "provider": "abound"},
)
check("inject api key", r.status_code == 200, f"got {r.status_code}: {r.text[:200]}")

# Wait for workspace bootstrap and auth cache to settle
print("\n  Waiting 5s for workspace bootstrap...")
time.sleep(5)
print()

# Verify the token works with a simple request first
print("--- Verify: Simple hello ---")
r = requests.post(
    f"{BASE_URL}/v1/responses",
    headers={"Authorization": f"Bearer {user_token}", "Content-Type": "application/json"},
    json={"input": "Say hi in 3 words", "stream": False},
    timeout=120,
)
print(f"  Status: {r.status_code}")
if r.status_code == 200:
    d = r.json()
    print(f"  Response status: {d.get('status')}")
    if d.get("output"):
        for item in d["output"]:
            if item.get("type") == "message":
                for c in item.get("content", []):
                    if c.get("type") == "output_text":
                        print(f"  Agent: {c['text'][:100]}")
else:
    print(f"  Error: {r.text[:300]}")
print()

# OpenAI client for the test user
client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")

# -----------------------------------------------------------
# 1. Get Account Info via agent
# -----------------------------------------------------------
print("--- 1. Get Account Info via agent ---")
try:
    response = client.responses.create(
        model="default",
        input="Use the http tool to call the Abound API and get my account information. "
              "GET https://devneobank.timesclub.co/times/bank/remittance/agent/account/info "
              "with header device-type: WEB. Show me the results.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    agent_text += content.text

    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # Check if agent got real account data (not mock)
    has_account_data = any(term in agent_text.lower() for term in [
        "user_id", "acc_", "limit", "recipient", "funding", "ach",
    ])
    check("references account data", has_account_data,
          "agent response doesn't mention account data")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 2. Get Exchange Rate via agent
# -----------------------------------------------------------
print("--- 2. Get Exchange Rate via agent ---")
try:
    response = client.responses.create(
        model="default",
        input="Use the http tool to get the current USD to INR exchange rate from Abound. "
              "GET https://devneobank.timesclub.co/times/bank/remittance/agent/exchange-rate?from_currency=USD&to_currency=INR "
              "with header device-type: WEB. Show me both the current and effective rates.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    agent_text += content.text

    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    has_rate_data = any(term in agent_text.lower() for term in [
        "exchange", "rate", "usd", "inr", "effective",
    ])
    check("references exchange rate", has_rate_data,
          "agent response doesn't mention exchange rates")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 3. Create Notification via agent
# -----------------------------------------------------------
print("--- 3. Create Notification via agent ---")
try:
    response = client.responses.create(
        model="default",
        input="Use the http tool to create a notification on Abound. "
              "POST https://dev.timesclub.co/times/users/agent/create-notification "
              "with header device-type: WEB and Content-Type: application/json. "
              "Body: {\"message_id\": \"agent_test_001\", \"action_type\": \"notification\", "
              "\"meta_data\": {\"score\": 75, \"rate\": 85.42}}. "
              "Show me the result.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    agent_text += content.text

    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    has_notification_data = any(term in agent_text.lower() for term in [
        "accepted", "notification", "message_id", "success",
    ])
    check("references notification result", has_notification_data,
          "agent response doesn't mention notification")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 4. Natural language prompt (no explicit URL)
# -----------------------------------------------------------
print("--- 4. Natural language: 'What is my account balance?' ---")
try:
    response = client.responses.create(
        model="default",
        input="What is my Abound account information? Show me my transfer limits, recipients, and funding sources.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    agent_text += content.text

    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 5. Natural language: exchange rate
# -----------------------------------------------------------
print("--- 5. Natural language: 'What is the USD to INR rate?' ---")
try:
    response = client.responses.create(
        model="default",
        input="What's the current USD to INR exchange rate? Is it a good time to send money to India?",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = ""
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    agent_text += content.text

    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
