"""Test the Responses API (/v1/responses) against a live deployment.

Creates a test user, sends requests with the OpenAI client, then cleans up.

Usage:
    pip install openai requests
    python tests/scripts/test_responses_api.py
"""

import atexit
import os
import uuid

import requests
from openai import OpenAI

BASE_URL = os.environ["BASE_URL"]
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]

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
# Setup: Create test user
# -----------------------------------------------------------
print("=== Responses API Tests ===")
print(f"Target: {BASE_URL}\n")

print("--- Setup: Create test user ---")
r = admin.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Responses Test User",
    "email": f"resp-{uuid.uuid4().hex[:8]}@example.com",
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
print()

# -----------------------------------------------------------
# Initialize OpenAI client pointing at IronClaw
# -----------------------------------------------------------
client = OpenAI(
    api_key=user_token,
    base_url=f"{BASE_URL}/v1",
)

# -----------------------------------------------------------
# 1. Non-streaming response (OpenAI client)
# -----------------------------------------------------------
print("--- 1. Non-streaming response (OpenAI client) ---")
try:
    response = client.responses.create(
        model="default",
        input="Say hello in exactly 3 words",
    )
    check("has id", hasattr(response, "id") and response.id.startswith("resp_"),
          f"id={getattr(response, 'id', None)}")
    check("has output", len(response.output) > 0, f"output={response.output}")
    check("status completed", response.status == "completed",
          f"status={response.status}")

    response_id = response.id
    print(f"  Response ID: {response_id}")

    # Show the text
    for item in response.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    print(f"  Agent said: {content.text[:200]}")
except Exception as e:
    check("request succeeded", False, str(e))
    response_id = None
print()

# -----------------------------------------------------------
# 2. Non-streaming with messages input
# -----------------------------------------------------------
print("--- 2. Non-streaming (messages input) ---")
try:
    response2 = client.responses.create(
        model="default",
        input=[{"role": "user", "content": "What is 2+2? Reply with just the number."}],
    )
    check("has output", len(response2.output) > 0)
    check("status completed", response2.status == "completed",
          f"status={response2.status}")

    for item in response2.output:
        if item.type == "message":
            for content in item.content:
                if content.type == "output_text":
                    print(f"  Agent said: {content.text[:200]}")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 3. Continue conversation
# -----------------------------------------------------------
if response_id:
    print("--- 3. Continue conversation (previous_response_id) ---")
    try:
        response3 = client.responses.create(
            model="default",
            input="Now say goodbye in 3 words",
            previous_response_id=response_id,
        )
        check("has output", len(response3.output) > 0)
        check("continues thread", response3.id != response_id)

        for item in response3.output:
            if item.type == "message":
                for content in item.content:
                    if content.type == "output_text":
                        print(f"  Agent said: {content.text[:200]}")
    except Exception as e:
        check("request succeeded", False, str(e))
    print()

# -----------------------------------------------------------
# 4. Get response by ID
# -----------------------------------------------------------
if response_id:
    print("--- 4. Get response by ID ---")
    try:
        retrieved = client.responses.retrieve(response_id)
        check("same id", retrieved.id == response_id,
              f"expected={response_id}, got={retrieved.id}")
        check("has output", len(retrieved.output) > 0)
    except Exception as e:
        check("request succeeded", False, str(e))
    print()

# -----------------------------------------------------------
# 5. Streaming response (OpenAI client)
# -----------------------------------------------------------
print("--- 5. Streaming response ---")
try:
    stream = client.responses.create(
        model="default",
        input="Count from 1 to 3",
        stream=True,
    )
    events = []
    full_text = ""
    for event in stream:
        events.append(event.type)
        if event.type == "response.output_text.delta":
            full_text += event.delta

    check("received events", len(events) > 0, f"got {len(events)} events")
    check("has response.created", "response.created" in events, f"events: {events[:10]}")
    check("has response.completed", "response.completed" in events, f"events: {events[-5:]}")
    check("has text deltas", len(full_text) > 0, f"text={full_text[:200]}")
    print(f"  Events: {len(events)} total")
    print(f"  Text: {full_text[:200]}")
except Exception as e:
    check("streaming succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 6. Streaming with raw requests (fallback test)
# -----------------------------------------------------------
print("--- 6. Streaming with raw requests ---")
r = requests.post(
    f"{BASE_URL}/v1/responses",
    headers={
        "Authorization": f"Bearer {user_token}",
        "Content-Type": "application/json",
    },
    json={"input": "Say hi", "stream": True},
    stream=True,
    timeout=30,
)
check("status 200", r.status_code == 200, f"got {r.status_code}")
event_count = 0
for line in r.iter_lines(decode_unicode=True):
    if line and line.startswith("event:"):
        event_count += 1
        if event_count <= 3:
            print(f"  {line}")
check("received SSE events", event_count > 0, f"got {event_count} events")
print(f"  Total events: {event_count}")
print()

# -----------------------------------------------------------
# 7. Error: no auth
# -----------------------------------------------------------
print("--- 7. Error: no auth ---")
r = requests.post(f"{BASE_URL}/v1/responses",
                   json={"input": "hello"},
                   headers={"Content-Type": "application/json"})
check("401 unauthorized", r.status_code == 401, f"got {r.status_code}")
print()

# -----------------------------------------------------------
# 8. Error: empty input
# -----------------------------------------------------------
print("--- 8. Error: empty input ---")
r = requests.post(f"{BASE_URL}/v1/responses",
                   json={"input": ""},
                   headers={
                       "Authorization": f"Bearer {user_token}",
                       "Content-Type": "application/json",
                   })
check("400 bad request", r.status_code == 400, f"got {r.status_code}")
print()

# -----------------------------------------------------------
# 9. Context injection (notification approval)
# -----------------------------------------------------------
print("--- 9. Context injection (notification approval) ---")
try:
    # Simulate: Abound sends notification approval with context
    r = requests.post(
        f"{BASE_URL}/v1/responses",
        headers={
            "Authorization": f"Bearer {user_token}",
            "Content-Type": "application/json",
        },
        json={
            "input": "Go ahead with the transfer",
            "x_context": {
                "notification_response": {
                    "notification_id": "msg_456",
                    "action": "approved",
                    "original_signal": "convert_now",
                    "score": 72,
                    "rate": 85.42,
                    "amount_usd": 1000,
                }
            },
            "stream": False,
        },
        timeout=120,
    )
    check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text[:300]}")
    data = r.json()
    check("status completed", data.get("status") == "completed",
          f"status={data.get('status')}")
    check("has output", len(data.get("output", [])) > 0)

    # The agent should see the context and mention the notification/approval
    for item in data.get("output", []):
        if item.get("type") == "message":
            for content in item.get("content", []):
                if content.get("type") == "output_text":
                    text = content["text"]
                    print(f"  Agent said: {text[:300]}")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# 10. Context injection (notification rejection)
# -----------------------------------------------------------
print("--- 10. Context injection (notification rejection) ---")
try:
    r = requests.post(
        f"{BASE_URL}/v1/responses",
        headers={
            "Authorization": f"Bearer {user_token}",
            "Content-Type": "application/json",
        },
        json={
            "input": "I changed my mind, cancel it",
            "x_context": {
                "notification_response": {
                    "notification_id": "msg_789",
                    "action": "rejected",
                    "original_signal": "convert_now",
                    "score": 65,
                    "rate": 85.20,
                    "amount_usd": 2000,
                }
            },
            "stream": False,
        },
        timeout=120,
    )
    check("status 200", r.status_code == 200, f"got {r.status_code}: {r.text[:300]}")
    data = r.json()
    check("status completed", data.get("status") == "completed",
          f"status={data.get('status')}")
    check("has output", len(data.get("output", [])) > 0)

    for item in data.get("output", []):
        if item.get("type") == "message":
            for content in item.get("content", []):
                if content.get("type") == "output_text":
                    text = content["text"]
                    print(f"  Agent said: {text[:300]}")
except Exception as e:
    check("request succeeded", False, str(e))
print()

# -----------------------------------------------------------
# Summary
# -----------------------------------------------------------
print(f"=== Results: {passed} passed, {failed} failed ===")
exit(0 if failed == 0 else 1)
