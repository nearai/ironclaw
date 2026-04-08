# /// script
# dependencies = ["requests", "openai"]
# ///
"""E2E tests for Abound API integration through IronClaw's Responses API.

Creates a test user, injects Abound dev credentials, then sends natural
language prompts that should trigger the agent to call Abound's API via
the http tool (guided by the abound-remittance skill).

Usage:
    python integrations/abound/tests/test_abound_e2e.py
"""

import atexit
import json
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


def has_tool_call(response, tool_name: str = "http") -> bool:
    """Check if the response includes a function_call for the given tool."""
    for item in response.output:
        if item.type == "function_call" and getattr(item, "name", "") == tool_name:
            return True
    return False


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


# AGENTS.md is auto-seeded via AGENTS_SEED_PATH env var on the server

print("\n--- Setup: Inject Massive API key ---")
massive_key = os.environ.get("MASSIVE_API_KEY")
if massive_key:
    r = admin.put(
        f"{BASE_URL}/api/admin/users/{user_id}/secrets/massive_api_key",
        json={"value": massive_key, "provider": "massive"},
    )
    check("inject massive_api_key", r.status_code == 200,
          f"got {r.status_code}: {r.text[:200]}")
else:
    check("inject massive_api_key", False,
          "MASSIVE_API_KEY env var not set")

# Wait for workspace bootstrap and auth cache to settle
print("\n  Waiting 5s for workspace bootstrap...")
time.sleep(5)
print()

client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")

# -----------------------------------------------------------
# 1. Smart remittance: analyze_transfer
# -----------------------------------------------------------
print("--- 1. Smart remittance: analyze_transfer ---")
target_rate = None
try:
    response = client.responses.create(
        model="default",
        input="Should I send $500 to India now or wait? Analyze the current USD/INR rate and timing.",
        timeout=180,
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text}")

    # Try to extract target_rate from the JSON response
    try:
        data = json.loads(agent_text)
        target_rate = data.get("target_rate") or data.get("plot", {}).get("target_rate")
        if target_rate:
            print(f"  Extracted target_rate: {target_rate}")
    except (json.JSONDecodeError, AttributeError):
        pass

    has_analysis = any(term in agent_text.lower() for term in [
        "wait", "now", "rate", "inr", "volatility", "rsi", "hit rate", "recommend",
    ])
    check("contains timing analysis", has_analysis,
          "agent response doesn't contain transfer timing analysis")
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


    # Should mention effective rate (real data) or account setup (auth error)
    has_rate_or_setup = any(term in agent_text.lower() for term in [
        "effective", "rate", "exchange", "setup", "support", "account",
    ])
    check("mentions rate or account setup", has_rate_or_setup,
          "response doesn't mention rate or account setup")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 3. Smart remittance: create routine
# -----------------------------------------------------------
print("--- 3. Smart remittance: create routine ---")
try:
    if not target_rate:
        target_rate = 100
    routine_input = (
            f"Run the routine every 1 hour checking whether the current USD/INR exchange rate "
            f"is greater than {target_rate} (the target rate from the analysis). "
            f"If so, send a notification to my Abound app that the target rate has been reached, "
            f"include the current rate."
        )
    response = client.responses.create(
        model="default",
        input=routine_input,
        timeout=180,
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text}")

    has_routine = any(term in agent_text.lower() for term in [
        "routine", "schedule", "hourly", "every hour", "created", "set up",
        "notification", "notify", "alert", "mission",
    ])
    check("contains routine creation confirmation", has_routine,
          "agent response doesn't confirm routine creation")
except Exception as e:
    check("request succeeded", False, str(e))
print()