# /// script
# dependencies = ["requests", "openai"]
# ///
"""Tests for user-visible message contracts.

Two distinct contracts:

1. **Transaction status query** — when the user asks about an existing
   transaction by ID, the agent must direct them to the Remittance tab
   in the Abound app, NOT suggest email or customer support.

2. **Send notification silence** — when `abound_send_wire(action="send")`
   succeeds, the tool emits *no* `function_call_output` item (the harness
   suppresses empty results so the Responses API surfaces nothing for the
   call), and the agent's reply text for that turn is empty (no paraphrased
   confirmation, no "Notification sent…" echo, no follow-up question). The
   UI handles the user-facing confirmation.

Required env vars:
    BASE_URL            IronClaw deployment URL (e.g. http://localhost:3000)
    ADMIN_TOKEN         Admin bearer token
    ABOUND_READ_TOKEN
    ABOUND_WRITE_TOKEN
    ABOUND_API_KEY
    MASSIVE_API_KEY     (optional, enables forex analysis in initiate)

Usage:
    BASE_URL=http://localhost:3000 ADMIN_TOKEN=... \\
        uv run integrations/abound/tests/test_abound_messages.py
"""

import atexit
import os
import sys
import time
import uuid

import requests
from openai import OpenAI

BASE_URL = os.environ["BASE_URL"].rstrip("/")
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]
ABOUND_READ_TOKEN = os.environ["ABOUND_READ_TOKEN"]
ABOUND_WRITE_TOKEN = os.environ["ABOUND_WRITE_TOKEN"]
ABOUND_API_KEY = os.environ["ABOUND_API_KEY"]
MASSIVE_API_KEY = os.environ.get("MASSIVE_API_KEY", "")
TEMPERATURE = float(os.environ.get("TEMPERATURE", "0.1"))

admin = requests.Session()
admin.headers.update({
    "Authorization": f"Bearer {ADMIN_TOKEN}",
    "Content-Type": "application/json",
})

passed = 0
failed = 0
user_id = ""


def cleanup():
    if user_id:
        admin.delete(f"{BASE_URL}/api/admin/users/{user_id}")
        print(f"\n  Deleted test user {user_id}")


atexit.register(cleanup)


def chk(name: str, condition: bool, detail: str = "") -> bool:
    global passed, failed
    if condition:
        print(f"  PASS: {name}")
        passed += 1
    else:
        print(f"  FAIL: {name}")
        if detail:
            for line in detail.splitlines()[:8]:
                print(f"    {line}")
        failed += 1
    return condition


def collect_text(resp) -> str:
    return " ".join(
        c.text
        for item in resp.output if item.type == "message"
        for c in item.content if c.type == "output_text"
    )


def collect_tool_outputs(resp) -> list[str]:
    """Return the `output` of every function_call_output item in the response."""
    return [
        item.output
        for item in resp.output
        if item.type == "function_call_output" and item.output
    ]


def find_send_call(resp) -> tuple[str, str | None] | None:
    """Locate `abound_send_wire(action="send")` and any matching tool output.

    Returns (call_id, output_string_or_None) where output_string is None
    when no `function_call_output` item exists for the call — the expected
    state under the silent-success contract. Returns None when no
    `abound_send_wire(send)` call is in the response at all.
    """
    import json as _json
    send_call_ids: list[str] = []
    for item in resp.output:
        if item.type != "function_call":
            continue
        if item.name not in ("abound_send_wire", "abound_send_wire(send)"):
            continue
        action = ""
        if item.name.endswith("(send)"):
            action = "send"
        else:
            try:
                args = _json.loads(item.arguments or "{}")
                action = args.get("action") or ""
            except Exception:
                action = ""
        if action == "send":
            send_call_ids.append(item.call_id)

    if not send_call_ids:
        return None

    for item in resp.output:
        if item.type == "function_call_output" and item.call_id in send_call_ids:
            return (item.call_id, item.output or "")
    return (send_call_ids[0], None)


# ---------------------------------------------------------------------------
# Setup: create a fresh test user with secrets injected.
# ---------------------------------------------------------------------------
print("=== Abound user-message contract tests ===")
print(f"Target: {BASE_URL}")
t_total = time.monotonic()

r = admin.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Messages Test",
    "email": f"messages-{uuid.uuid4().hex[:8]}@example.com",
    "role": "member",
})
if r.status_code != 200:
    print(f"FATAL: create user {r.status_code} {r.text}")
    raise SystemExit(1)

data = r.json()
user_id = data["id"]
user_token = data["token"]
print(f"Created user {user_id}")

for name, value, provider in [
    ("abound_read_token", ABOUND_READ_TOKEN, "abound"),
    ("abound_write_token", ABOUND_WRITE_TOKEN, "abound"),
    ("abound_api_key", ABOUND_API_KEY, "abound"),
    *([("massive_api_key", MASSIVE_API_KEY, "massive")] if MASSIVE_API_KEY else []),
]:
    r = admin.put(
        f"{BASE_URL}/api/admin/users/{user_id}/secrets/{name}",
        json={"value": value, "provider": provider},
    )
    if r.status_code != 200:
        print(f"FATAL: inject {name}: {r.status_code} {r.text[:200]}")
        raise SystemExit(1)

print(f"  [setup] sleeping 5s for workspace bootstrap...", flush=True)
time.sleep(5)
print(f"  [setup] done in {time.monotonic() - t_total:.2f}s", flush=True)
client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")


def call(prompt: str, prev_id: str | None = None):
    kwargs = {"model": "default", "input": prompt, "timeout": 180,
              "temperature": TEMPERATURE}
    if prev_id:
        kwargs["previous_response_id"] = prev_id
    t0 = time.monotonic()
    print(f"  [call] sending: {prompt[:80]!r}{' (+prev)' if prev_id else ''}",
          flush=True)
    resp = client.responses.create(**kwargs)
    elapsed = time.monotonic() - t0
    n_func = sum(1 for i in resp.output if i.type == "function_call")
    n_msg = sum(1 for i in resp.output if i.type == "message")
    n_out = sum(1 for i in resp.output if i.type == "function_call_output")
    print(f"  [call] {elapsed:.2f}s  status={resp.status} "
          f"func_calls={n_func} func_outs={n_out} msgs={n_msg} id={resp.id}",
          flush=True)
    return resp


# ---------------------------------------------------------------------------
# Test 1: Transaction status query → Remittance tab.
# ---------------------------------------------------------------------------
print("\n--- Test 1: transaction status → Remittance tab ---")
t_test1 = time.monotonic()

queries = [
    "what's the status of transaction 2015262375?",
    "did my transfer go through?",
    "is my $50 transfer complete?",
]

for q in queries:
    try:
        resp = call(q)
    except Exception as e:
        chk(f"status query '{q[:40]}...' returned a response", False, str(e))
        continue

    text = collect_text(resp).lower()
    chk(
        f"status query '{q[:40]}...' got non-empty reply",
        bool(text),
        f"resp.status={resp.status}",
    )
    chk(
        f"status query '{q[:40]}...' mentions Remittance tab",
        "remittance" in text,
        f"text[:200]={text[:200]}",
    )
    chk(
        f"status query '{q[:40]}...' does NOT suggest email",
        "email" not in text,
        f"text[:200]={text[:200]}",
    )
    chk(
        f"status query '{q[:40]}...' does NOT suggest customer support",
        "customer support" not in text and "contact support" not in text,
        f"text[:200]={text[:200]}",
    )


# ---------------------------------------------------------------------------
# Test 2: Send tool returns empty string AND agent stays silent.
# ---------------------------------------------------------------------------
# Contract: on action=send success, the tool's function_call_output is
# empty, and the agent's text reply for that same response is also empty.
# The frontend renders the user-facing confirmation; any agent text would
# be duplicate noise.
print(f"\n  [test 1] elapsed {time.monotonic() - t_test1:.2f}s")
print("\n--- Test 2: send tool empty output + agent silence ---")
t_test2 = time.monotonic()

# Two-shot, deterministic: turn 1 front-loads every parameter using
# positional references ("first recipient", "first debit account") so the
# agent can run `initiate` analysis without parameter-collection nudges.
# Turn 2 is a literal "send now" — the only nudge ever needed under the
# silent-success contract.
amount = 17  # within typical Abound limits

# Turn 1: initiate analysis.
resp_initiate = call(
    f"send {amount} for my first recipient in family maintenance "
    f"from the first debit account",
    prev_id=None,
)

# Turn 2: trigger the send.
resp_send = call("send now", prev_id=resp_initiate.id)

# All assertions are made against the turn-2 response.
pair = find_send_call(resp_send)
chk(
    "abound_send_wire(action=send) call present in turn-2 response",
    pair is not None,
    "no function_call with name=abound_send_wire and action=send was found",
)

if pair is not None:
    _, send_output = pair
    chk(
        "no function_call_output emitted for send (silent-success contract)",
        send_output is None,
        f"got function_call_output with output={send_output!r} (expected: no item)",
    )
    agent_text = collect_text(resp_send).strip()
    chk(
        "agent reply text is empty on send success",
        agent_text == "",
        f"agent text was non-empty: {agent_text[:200]!r}",
    )


# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
print(f"\n  [test 2] elapsed {time.monotonic() - t_test2:.2f}s")
print(f"  [total] elapsed {time.monotonic() - t_total:.2f}s")
print(f"\n=== {passed} passed, {failed} failed ===")
sys.exit(0 if failed == 0 else 1)
