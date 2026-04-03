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

print("\n--- Setup: Verify Massive API key is seeded (global default user) ---")
r = admin.get(f"{BASE_URL}/api/admin/users/default/secrets")
if r.status_code == 200:
    secret_names = [s["name"] for s in r.json().get("secrets", [])]
    if "massive_api_key" not in secret_names:
        massive_key = os.environ.get("MASSIVE_API_KEY")
        if massive_key:
            r2 = admin.put(
                f"{BASE_URL}/api/admin/users/default/secrets/massive_api_key",
                json={"value": massive_key, "provider": "massive"},
            )
            check("massive_api_key seeded on default", r2.status_code == 200,
                  f"got {r2.status_code}: {r2.text[:200]}")
        else:
            check("massive_api_key seeded on default", False,
                  f"not found and MASSIVE_API_KEY env var not set — seed with: curl -X PUT {BASE_URL}/api/admin/users/default/secrets/massive_api_key -d '{{\"value\": \"<key>\"}}'")
    else:
        check("massive_api_key seeded on default", True)
else:
    check("secrets list accessible", False, f"got {r.status_code}: {r.text[:200]}")

# Wait for workspace bootstrap and auth cache to settle
print("\n  Waiting 5s for workspace bootstrap...")
time.sleep(5)
print()

client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")

# -----------------------------------------------------------
# 1. Smart remittance: analyze_transfer
# -----------------------------------------------------------
print("--- 1. Smart remittance: analyze_transfer ---")
try:
    response = client.responses.create(
        model="default",
        input="Should I send $500 to India now or wait? Analyze the current USD/INR rate and timing.",
        timeout=180,
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    has_analysis = any(term in agent_text.lower() for term in [
        "wait", "now", "rate", "inr", "volatility", "rsi", "hit rate", "recommend",
    ])
    check("contains timing analysis", has_analysis,
          "agent response doesn't contain transfer timing analysis")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 2. Smart remittance: validate_transfer_target
# -----------------------------------------------------------
print("--- 2. Smart remittance: validate_transfer_target ---")
try:
    response = client.responses.create(
        model="default",
        input="What's the probability of USD/INR hitting 95 in the next 30 days?",
        timeout=180,
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    has_probability = any(term in agent_text.lower() for term in [
        "probability", "hit", "%", "horizon", "days", "chance",
    ])
    check("contains probability analysis", has_probability,
          "agent response doesn't contain probability/horizon data")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 3. Get Account Info (natural language)
# -----------------------------------------------------------
print("--- 3. Account info ---")
try:
    response = client.responses.create(
        model="default",
        input="What is my Abound account info? Show me my transfer limits, recipients, and funding sources.",
    )
    check("status completed", response.status == "completed", f"status={response.status}")
    check("has output", len(response.output) > 0)

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    # Verify the http tool was actually called
    check("called http tool", has_tool_call(response, "http"),
          f"output types: {[item.type for item in response.output]}")

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
# 4. Exchange rate (natural language)
# -----------------------------------------------------------
print("--- 4. Exchange rate ---")
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

    check("called http tool", has_tool_call(response, "http"),
          f"output types: {[item.type for item in response.output]}")

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
# 5. Send money advice (natural language)
# -----------------------------------------------------------
print("--- 5. Send money advice ---")
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

    # Should reference INR/rate or ask about transfer details (recipient, reason)
    has_transfer_context = any(term in agent_text.lower() for term in [
        "inr", "rupee", "rate", "recipient", "transfer", "wire", "send",
    ])
    check("mentions transfer context", has_transfer_context)
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 6. Create notification (natural language)
# -----------------------------------------------------------
print("--- 6. Create notification ---")
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
# 5. Streaming: exchange rate
# -----------------------------------------------------------
print("--- 5. Streaming: exchange rate ---")
try:
    stream = client.responses.create(
        model="default",
        input="What's the current USD to INR rate on Abound?",
        stream=True,
    )
    events = []
    full_text = ""
    for event in stream:
        events.append(event.type)
        if event.type == "response.output_text.delta":
            full_text += event.delta

    check("has response.created", "response.created" in events,
          f"events: {events[:5]}")
    check("has response.completed", "response.completed" in events,
          f"events: {events[-5:]}")
    check("has text deltas", len(full_text) > 0, f"text={full_text[:100]}")
    check("mentions rate in stream", any(t in full_text.lower() for t in ["rate", "inr", "exchange"]),
          "streamed text doesn't mention rate")
    print(f"  Events: {len(events)} total")
    print(f"  Text: {full_text[:300]}")
except Exception as e:
    check("streaming succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 6. Choice set: payment reason
# -----------------------------------------------------------
print("--- 6. Choice set: payment reason ---")
try:
    response = client.responses.create(
        model="default",
        input="I want to send $500 to India. What payment reasons can I choose from?",
    )
    check("status completed", response.status == "completed", f"status={response.status}")

    agent_text = extract_agent_text(response)
    print(f"  Agent response ({len(agent_text)} chars): {agent_text[:400]}")

    has_choice_set = "[[choice_set]]" in agent_text and "[[/choice_set]]" in agent_text
    check("contains choice_set markers", has_choice_set,
          "no [[choice_set]]...[[/choice_set]] block found")

    if has_choice_set:
        import json as json_mod
        start = agent_text.index("[[choice_set]]") + len("[[choice_set]]")
        end = agent_text.index("[[/choice_set]]")
        raw = agent_text[start:end].strip()
        try:
            cs = json_mod.loads(raw)
            check("valid JSON", True)
            check("has type", cs.get("type") == "choice_set")
            check("has items", len(cs.get("items", [])) >= 2,
                  f"items: {len(cs.get('items', []))}")
            for item in cs.get("items", []):
                check(f"item '{item.get('id','')}' has prompt",
                      bool(item.get("prompt")))
        except Exception as e:
            check("valid JSON", False, str(e))
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 7. Streaming: multiple deltas (not one big chunk)
# -----------------------------------------------------------
print("--- 7. Streaming: verify incremental deltas ---")
try:
    stream = client.responses.create(
        model="default",
        input="Write a short paragraph about exchange rates.",
        stream=True,
    )
    delta_count = 0
    delta_sizes = []
    full_text = ""
    for event in stream:
        if event.type == "response.output_text.delta":
            delta_count += 1
            delta_sizes.append(len(event.delta))
            full_text += event.delta

    check("has multiple deltas", delta_count > 1,
          f"got {delta_count} delta(s) — expected >1 for token-level streaming")
    check("has text content", len(full_text) > 0, f"text length: {len(full_text)}")

    if delta_count > 1:
        avg_size = sum(delta_sizes) / len(delta_sizes)
        check("avg delta smaller than full text", avg_size < len(full_text),
              f"avg delta: {avg_size:.0f} chars, full text: {len(full_text)} chars")

    print(f"  Deltas: {delta_count}")
    print(f"  Delta sizes: {delta_sizes[:10]}{'...' if len(delta_sizes) > 10 else ''}")
    print(f"  Full text: {len(full_text)} chars")
    print(f"  Text: {full_text[:200]}")
except Exception as e:
    check("streaming succeeded", False, str(e))
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
