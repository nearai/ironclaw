# /// script
# dependencies = ["requests", "openai"]
# ///
"""Interactive chat with a test Abound user. Deletes user on exit.

Usage:
    export BASE_URL=... ADMIN_TOKEN=... ABOUND_BEARER_TOKEN=... ABOUND_API_KEY=... MASSIVE_API_KEY=...
    uv run python integrations/abound/tests/chat.py
"""

import atexit
import os
import time

import requests
from openai import OpenAI

BASE_URL = os.environ["BASE_URL"]
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]

admin = requests.Session()
admin.headers.update({
    "Authorization": f"Bearer {ADMIN_TOKEN}",
    "Content-Type": "application/json",
})

# Create user
r = admin.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Interactive Test",
    "role": "member",
})
if r.status_code != 200:
    print(f"FATAL: {r.status_code} {r.text[:200]}")
    exit(1)

data = r.json()
user_id = data["id"]
user_token = data["token"]
print(f"User: {user_id}")

# Inject credentials
for name, env_key in [
    ("abound_read_token", "ABOUND_BEARER_TOKEN"),
    ("abound_write_token", "ABOUND_WRITE_TOKEN"),
    ("abound_api_key", "ABOUND_API_KEY"),
    ("massive_api_key", "MASSIVE_API_KEY"),
]:
    val = os.environ.get(env_key)
    if val:
        admin.put(
            f"{BASE_URL}/api/admin/users/{user_id}/secrets/{name}",
            json={"value": val, "provider": name.split("_")[0]},
        )
        print(f"  Injected {name}")

print("Waiting for workspace bootstrap...")
time.sleep(5)


def cleanup():
    print(f"\nDeleting user {user_id}...")
    admin.delete(f"{BASE_URL}/api/admin/users/{user_id}")
    print("Done")


atexit.register(cleanup)

client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")
prev_id = None

print("\n=== Abound Chat (ctrl-c to exit) ===\n")
while True:
    try:
        msg = input("You: ").strip()
    except (KeyboardInterrupt, EOFError):
        break
    if not msg:
        continue

    try:
        t0 = time.time()
        response = client.responses.create(
            model="default",
            input=msg,
            previous_response_id=prev_id,
            timeout=180,
        )
        elapsed = time.time() - t0
        prev_id = response.id

        for item in response.output:
            if item.type == "message":
                for c in item.content:
                    if c.type == "output_text":
                        print(f"\nAgent: {c.text}\n")
            elif item.type == "function_call":
                print(f"  [tool: {item.name}]")
            elif item.type == "function_call_output":
                print(f"  [result: {item.output[:100]}...]")

        print(f"  [{elapsed:.1f}s]")

        if response.status == "failed":
            print(f"  [status: failed — {getattr(response, 'error', 'unknown')}]")

    except Exception as e:
        print(f"  Error: {e}")
