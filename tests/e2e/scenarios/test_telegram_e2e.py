"""Full-process Telegram E2E tests.

Boot IronClaw → activate Telegram via setup API → POST webhook updates
→ verify sendMessage round-trip through mock LLM to fake Telegram API.
"""

import asyncio
import json
import os
import re
import time

import httpx

from helpers import api_post, auth_headers

# Bot token used throughout these tests.
BOT_TOKEN = "111222333:FAKE_E2E_TOKEN"
# Owner user id used in subsequent Telegram messages.
OWNER_USER_ID = 42
PAIRED_USER_ID = 77
# Fixed webhook secret supplied during setup so all tests can use it
# without extracting it from the server.
WEBHOOK_SECRET = "e2e-test-webhook-secret-for-telegram"
PAIRING_CODE_RE = re.compile(r"approve telegram ([A-Z0-9]+)|`([A-Z0-9]+)`")


# ── helpers ──────────────────────────────────────────────────────────────


async def reset_fake_tg(fake_tg_url: str):
    async with httpx.AsyncClient() as c:
        await c.post(f"{fake_tg_url}/__mock/reset")


async def install_telegram(base_url: str):
    """Install the bundled Telegram WASM channel if not already installed."""
    r = await api_post(
        base_url,
        "/api/extensions/install",
        json={"name": "telegram", "kind": "wasm_channel"},
        timeout=180,
    )
    # 200 = freshly installed, 409 = already installed — both are fine.
    assert r.status_code in (200, 409), (
        f"Telegram install failed ({r.status_code}): {r.text}"
    )


def _patch_capabilities_for_testing(channels_dir: str):
    """Patch the installed capabilities file for E2E testing.

    1. Remove validation_endpoint (points at real api.telegram.org, unreachable
       in tests and blocked by SSRF protection).
    2. Ensure ``telegram_webhook_secret`` is declared in ``required_secrets``
       with ``auto_generate`` so the server generates one during setup.
       Downloaded release artifacts may lag behind the local source and omit
       this entry, which would leave the webhook router without a secret.
    """
    cap_path = os.path.join(channels_dir, "telegram.capabilities.json")
    assert os.path.exists(cap_path), (
        f"Capabilities file not found at {cap_path}; "
        f"files in dir: {os.listdir(channels_dir)}"
    )
    with open(cap_path, "r") as f:
        caps = json.load(f)

    # 1. Remove validation_endpoint
    if "setup" in caps and "validation_endpoint" in caps["setup"]:
        del caps["setup"]["validation_endpoint"]

    # 2. Ensure telegram_webhook_secret is in required_secrets with auto_generate
    setup = caps.setdefault("setup", {})
    required = setup.setdefault("required_secrets", [])
    has_webhook_secret = any(
        s.get("name") == "telegram_webhook_secret" for s in required
    )
    if not has_webhook_secret:
        required.append({
            "name": "telegram_webhook_secret",
            "prompt": "Webhook secret (auto-generated for tests)",
            "optional": True,
            "auto_generate": {"length": 64},
        })

    # 3. Ensure webhook section declares secret_name and secret_header.
    # Poll interval remains subject to the production minimum enforced by the
    # WASM host capabilities, so tests should allow for a real long-poll tick.
    channel = caps.setdefault("capabilities", {}).setdefault("channel", {})
    webhook = channel.setdefault("webhook", {})
    webhook.setdefault("secret_name", "telegram_webhook_secret")
    webhook.setdefault("secret_header", "X-Telegram-Bot-Api-Secret-Token")

    with open(cap_path, "w") as f:
        json.dump(caps, f, indent=2)


async def activate_telegram(
    base_url: str, http_url: str, fake_tg_url: str, channels_dir: str
) -> None:
    """Install (if needed) and run the Telegram setup flow."""
    await reset_fake_tg(fake_tg_url)
    await install_telegram(base_url)

    # Patch capabilities for testing (remove validation_endpoint, ensure
    # webhook secret is declared in required_secrets).
    _patch_capabilities_for_testing(channels_dir)

    # Submit bot token AND a known webhook secret.
    # Supplying the secret explicitly (instead of relying on auto-generation)
    # lets the tests use a known value for subsequent webhook POSTs.
    async with httpx.AsyncClient() as c:
        r1 = await c.post(
            f"{base_url}/api/extensions/telegram/setup",
            headers=auth_headers(),
            json={
                "secrets": {
                    "telegram_bot_token": BOT_TOKEN,
                    "telegram_webhook_secret": WEBHOOK_SECRET,
                },
                "fields": {},
            },
            timeout=30,
        )
    r1.raise_for_status()
    body1 = r1.json()
    assert body1.get("success"), f"Setup call failed: {body1}"
    if verification := body1.get("verification"):
        code = verification.get("code")
        assert code, f"Telegram verification response missing code: {body1}"
        await queue_fake_telegram_update(
            fake_tg_url,
            {
                "message": {
                    "message_id": 1,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": f"/start {code}",
                },
            },
        )
        async with httpx.AsyncClient() as c:
            r2 = await c.post(
                f"{base_url}/api/extensions/telegram/setup",
                headers=auth_headers(),
                json={
                    "secrets": {
                        "telegram_bot_token": BOT_TOKEN,
                        "telegram_webhook_secret": WEBHOOK_SECRET,
                    },
                    "fields": {},
                },
                timeout=30,
            )
        r2.raise_for_status()
        body1 = r2.json()
        assert body1.get("success"), f"Telegram verification setup retry failed: {body1}"

    assert body1.get("activated"), f"Setup call did not activate Telegram: {body1}"

    # Verification can emit setup confirmation DMs; clear them so subsequent
    # assertions only see scenario-specific traffic.
    await reset_fake_tg(fake_tg_url)

    # Complete the pairing flow so OWNER_USER_ID can chat normally in the
    # subsequent round-trip assertions.
    pairing_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 1,
            "message": {
                "message_id": 1,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert pairing_resp.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    code = extract_pairing_code(messages)
    if code:
        await approve_pairing(base_url, code)
    await reset_fake_tg(fake_tg_url)


def extract_pairing_code(messages: list[dict]) -> str | None:
    """Extract a pairing code from Telegram pairing reply text."""
    for message in reversed(messages):
        text = message.get("text", "")
        match = PAIRING_CODE_RE.search(text)
        if match:
            return match.group(1) or match.group(2)
    return None


async def approve_pairing(base_url: str, code: str) -> None:
    """Approve a pairing code through the web API."""
    async with httpx.AsyncClient() as c:
        response = await c.post(
            f"{base_url}/api/pairing/telegram/approve",
            headers=auth_headers(),
            json={"code": code},
            timeout=10,
        )
    response.raise_for_status()
    body = response.json()
    assert body.get("success"), f"Pairing approval failed: {body}"


async def pair_telegram_user(
    base_url: str,
    http_url: str,
    fake_tg_url: str,
    *,
    user_id: int,
    first_name: str,
) -> None:
    """Pair an arbitrary Telegram user to the current IronClaw owner scope."""
    pairing_resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": int(time.time() * 1000) % 2_147_483_647,
            "message": {
                "message_id": 1,
                "from": {
                    "id": user_id,
                    "is_bot": False,
                    "first_name": first_name,
                },
                "chat": {"id": user_id, "type": "private"},
                "date": int(time.time()),
                "text": "hello from paired user",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert pairing_resp.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    code = extract_pairing_code(messages)
    assert code, f"Expected pairing code in Telegram reply, got: {messages}"
    await approve_pairing(base_url, code)
    await reset_fake_tg(fake_tg_url)


async def create_owner_routine_via_chat(base_url: str, routine_name: str) -> None:
    """Create an owner-scoped routine through the gateway chat API."""
    thread_r = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    assert thread_r.status_code == 200
    thread_id = thread_r.json()["id"]

    send_r = await api_post(
        base_url,
        "/api/chat/send",
        json={
            "content": f"create lightweight owner routine {routine_name}",
            "thread_id": thread_id,
        },
        timeout=30,
    )
    assert send_r.status_code in (200, 202), (
        f"Routine create send failed ({send_r.status_code}): {send_r.text}"
    )

    deadline = time.monotonic() + 30
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            history = await client.get(
                f"{base_url}/api/chat/history",
                params={"thread_id": thread_id},
                headers=auth_headers(),
                timeout=15,
            )
            history.raise_for_status()
            data = history.json()

            pending = data.get("pending_gate") or data.get("pending_approval")
            if pending:
                approval = await client.post(
                    f"{base_url}/api/chat/approval",
                    json={
                        "request_id": pending["request_id"],
                        "action": "approve",
                        "thread_id": thread_id,
                    },
                    headers=auth_headers(),
                    timeout=15,
                )
                assert approval.status_code == 202, (
                    f"Routine approval failed ({approval.status_code}): {approval.text}"
                )

            turns = data.get("turns", [])
            if turns and turns[-1].get("response"):
                response = turns[-1]["response"]
                assert routine_name in response, (
                    f"Routine create response missing routine name: {response}"
                )
                return

            await asyncio.sleep(0.5)

    raise TimeoutError(
        f"Routine '{routine_name}' did not complete via chat within 30 seconds"
    )


async def wait_for_routine(base_url: str, routine_name: str, *, timeout: float = 30) -> dict:
    """Poll the routines API until the named routine appears."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            response = await client.get(
                f"{base_url}/api/routines",
                headers=auth_headers(),
                timeout=10,
            )
            response.raise_for_status()
            for routine in response.json().get("routines", []):
                if routine.get("name") == routine_name:
                    return routine
            await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Routine '{routine_name}' did not appear within {timeout} seconds"
    )


async def queue_fake_telegram_update(fake_tg_url: str, update: dict) -> None:
    """Queue an update for fake Telegram polling mode APIs like getUpdates."""
    async with httpx.AsyncClient() as c:
        response = await c.post(
            f"{fake_tg_url}/__mock/queue_update",
            json=update,
            timeout=5,
        )
    response.raise_for_status()


async def post_telegram_webhook(
    http_url: str,
    update: dict,
    *,
    secret: str | None = None,
) -> httpx.Response:
    """POST a Telegram-shaped update to IronClaw's webhook endpoint."""
    headers = {"Content-Type": "application/json"}
    if secret is not None:
        headers["X-Telegram-Bot-Api-Secret-Token"] = secret
    async with httpx.AsyncClient() as c:
        return await c.post(
            f"{http_url}/webhook/telegram",
            json=update,
            headers=headers,
            timeout=10,
        )


async def wait_for_sent_messages(
    fake_tg_url: str,
    *,
    min_count: int = 1,
    timeout: float = 30,
) -> list[dict]:
    """Poll the fake Telegram API until at least min_count sendMessage calls appear."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as c:
        while time.monotonic() < deadline:
            r = await c.get(f"{fake_tg_url}/__mock/sent_messages", timeout=5)
            messages = r.json().get("messages", [])
            if len(messages) >= min_count:
                return messages
            await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Expected at least {min_count} sent messages within {timeout}s"
    )


async def get_api_calls(fake_tg_url: str) -> list[dict]:
    """Fetch all recorded API calls from the fake Telegram server."""
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_tg_url}/__mock/api_calls", timeout=5)
        return r.json().get("calls", [])


async def set_reject_markdown(fake_tg_url: str, reject: bool):
    """Toggle the markdown rejection flag on the fake Telegram server."""
    async with httpx.AsyncClient() as c:
        await c.post(
            f"{fake_tg_url}/__mock/set_reject_markdown",
            json={"reject": reject},
            timeout=5,
        )


async def set_rate_limit(fake_tg_url: str, count: int):
    """Set the number of sendMessage calls that should return 429."""
    async with httpx.AsyncClient() as c:
        await c.post(
            f"{fake_tg_url}/__mock/set_rate_limit",
            json={"count": count},
            timeout=5,
        )


async def set_fail_downloads(fake_tg_url: str, fail: bool):
    """Toggle download failure mode on the fake Telegram server."""
    async with httpx.AsyncClient() as c:
        await c.post(
            f"{fake_tg_url}/__mock/set_fail_downloads",
            json={"fail": fail},
            timeout=5,
        )


async def wait_for_api_call(
    fake_tg_url: str,
    method: str,
    *,
    timeout: float = 15,
) -> list[dict]:
    """Poll until at least one API call with the given method appears."""
    deadline = time.monotonic() + timeout
    async with httpx.AsyncClient() as c:
        while time.monotonic() < deadline:
            r = await c.get(f"{fake_tg_url}/__mock/api_calls", timeout=5)
            calls = r.json().get("calls", [])
            matching = [call for call in calls if call["method"] == method]
            if matching:
                return matching
            await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Expected at least one '{method}' API call within {timeout}s"
    )


# ── tests ────────────────────────────────────────────────────────────────


async def test_telegram_setup_and_dm_roundtrip(telegram_e2e_server):
    """Full DM round-trip: setup → webhook → mock LLM → sendMessage."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    # Reset fake API and activate the Telegram channel
    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    # Clear fake API state to only capture round-trip messages
    await reset_fake_tg(fake_tg_url)

    # POST a DM webhook update as the verified owner
    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 100,
            "message": {
                "message_id": 10,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200, f"Webhook returned {resp.status_code}: {resp.text}"

    # Wait for the bot to send a reply via the fake Telegram API.
    # The mock LLM matches "hello" → "Hello! How can I help you today?"
    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    reply_text = messages[-1].get("text", "")
    assert reply_text, f"Empty reply text. All sent messages: {messages}"
    assert messages[-1]["chat_id"] == OWNER_USER_ID


async def test_paired_telegram_user_lists_owner_routines(
    telegram_e2e_server_with_routines,
):
    """A paired Telegram guest should see routines created in owner scope."""
    base_url = telegram_e2e_server_with_routines["base_url"]
    http_url = telegram_e2e_server_with_routines["http_url"]
    fake_tg_url = telegram_e2e_server_with_routines["fake_tg_url"]
    channels_dir = telegram_e2e_server_with_routines["channels_dir"]
    routine_name = f"telegram-owner-{int(time.time())}"

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)
    await create_owner_routine_via_chat(base_url, routine_name)
    await wait_for_routine(base_url, routine_name)
    await reset_fake_tg(fake_tg_url)

    await pair_telegram_user(
        base_url,
        http_url,
        fake_tg_url,
        user_id=PAIRED_USER_ID,
        first_name="Paired Tester",
    )

    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": int(time.time() * 1000) % 2_147_483_647,
            "message": {
                "message_id": 99,
                "from": {
                    "id": PAIRED_USER_ID,
                    "is_bot": False,
                    "first_name": "Paired Tester",
                },
                "chat": {"id": PAIRED_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "list owner routines",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200

    try:
        messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    except TimeoutError as exc:
        api_calls = await get_api_calls(fake_tg_url)
        async with httpx.AsyncClient() as client:
            threads_response = await client.get(
                f"{base_url}/api/chat/threads",
                headers=auth_headers(),
                timeout=10,
            )
            threads_response.raise_for_status()
            threads = threads_response.json().get("threads", [])
            telegram_threads = [t for t in threads if t.get("channel") == "telegram"]
            latest_telegram_thread = telegram_threads[0] if telegram_threads else None
            latest_history = None
            if latest_telegram_thread:
                history_response = await client.get(
                    f"{base_url}/api/chat/history",
                    params={"thread_id": latest_telegram_thread["id"]},
                    headers=auth_headers(),
                    timeout=10,
                )
                history_response.raise_for_status()
                latest_history = history_response.json()
        raise AssertionError(
            "Expected paired Telegram user to receive a reply after requesting "
            f"owner routines; fake Telegram API calls were: {api_calls}; "
            f"latest telegram thread: {latest_telegram_thread}; "
            f"latest telegram history: {latest_history}"
        ) from exc
    reply_text = "\n".join(m.get("text", "") for m in messages if m.get("chat_id") == PAIRED_USER_ID)
    assert routine_name in reply_text, (
        f"Expected paired Telegram user to see owner routine '{routine_name}', "
        f"got replies: {messages}"
    )


async def test_telegram_edited_message_roundtrip(telegram_e2e_server):
    """Edited-message webhook triggers a new agent reply."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)

    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 200,
            "edited_message": {
                "message_id": 20,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "edit_date": int(time.time()),
                "text": "2 + 2",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200

    # Mock LLM matches "2+2" → "The answer is 4."
    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
    assert any("4" in m.get("text", "") for m in messages), (
        f"Expected '4' in replies: {messages}"
    )


async def test_telegram_unauthorized_user_rejected(telegram_e2e_server):
    """A webhook from a non-owner user should not produce a sendMessage reply."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)

    # Send a message from a different user ID (not the owner)
    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 300,
            "message": {
                "message_id": 30,
                "from": {
                    "id": 99999,
                    "is_bot": False,
                    "first_name": "Stranger",
                },
                "chat": {"id": 99999, "type": "private"},
                "date": int(time.time()),
                "text": "hello from stranger",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    # The webhook is accepted at the transport level but the WASM channel
    # should not route it to the agent (dm_policy = pairing).
    assert resp.status_code == 200

    # Give it a moment, then verify no reply was sent to the stranger.
    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_tg_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    stranger_replies = [m for m in messages if m.get("chat_id") == 99999]
    # The channel may send a pairing prompt, but should NOT send an LLM reply.
    for m in stranger_replies:
        text = m.get("text", "").lower()
        assert "how can i help" not in text, (
            f"Unauthorized user received an LLM reply: {m}"
        )


async def test_telegram_invalid_webhook_secret_rejected(telegram_e2e_server):
    """Webhook with wrong secret header is rejected."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 400,
            "message": {
                "message_id": 40,
                "from": {"id": OWNER_USER_ID, "is_bot": False, "first_name": "E2E"},
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "should be rejected",
            },
        },
        secret="wrong-secret",
    )
    assert resp.status_code in (401, 403), (
        f"Expected 401/403, got {resp.status_code}: {resp.text}"
    )


async def test_telegram_group_mention_filtering(telegram_e2e_server):
    """Group messages without a bot mention are ignored; mentioned messages get a reply."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    # Part 1: group message WITHOUT bot mention → no reply
    await reset_fake_tg(fake_tg_url)

    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 500,
            "message": {
                "message_id": 50,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": -1001, "type": "group", "title": "Test Group"},
                "date": int(time.time()),
                "text": "hello everyone",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200

    # Wait a few seconds and verify no reply was sent to the group
    await asyncio.sleep(3)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_tg_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    group_replies = [m for m in messages if m.get("chat_id") == -1001]
    assert len(group_replies) == 0, (
        f"Expected no replies to group without mention, got: {group_replies}"
    )

    # Part 2: group message WITH bot mention → reply expected
    await reset_fake_tg(fake_tg_url)

    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 501,
            "message": {
                "message_id": 51,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": -1001, "type": "group", "title": "Test Group"},
                "date": int(time.time()),
                "text": "@e2e_test_bot hello",
                "entities": [
                    {
                        "offset": 0,
                        "length": len("@e2e_test_bot"),
                        "type": "mention",
                    }
                ],
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
    group_replies = [m for m in messages if m.get("chat_id") == -1001]
    assert len(group_replies) >= 1, (
        f"Expected at least one reply to group with mention, got: {messages}"
    )


async def test_telegram_long_message_chunking(telegram_e2e_server):
    """Long LLM responses are split into multiple Telegram messages (<=4096 chars each)."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)

    # "long response" triggers the ~7400-char canned response in mock_llm.py
    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 600,
            "message": {
                "message_id": 60,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "long response",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp.status_code == 200

    # Wait for at least 2 chunks
    messages = await wait_for_sent_messages(fake_tg_url, min_count=2, timeout=30)

    # Verify each chunk is within Telegram's 4096 char limit
    for i, msg in enumerate(messages):
        text = msg.get("text", "")
        assert len(text) <= 4096, (
            f"Chunk {i} exceeds 4096 chars: {len(text)} chars"
        )

    # Verify all chunks target the correct chat_id
    for msg in messages:
        assert msg["chat_id"] == OWNER_USER_ID

    # Verify total text across all chunks exceeds the single-message limit
    total_text = "".join(m.get("text", "") for m in messages)
    assert len(total_text) > 4096, (
        f"Total text ({len(total_text)} chars) should exceed 4096"
    )


async def test_telegram_polling_mode_roundtrip(isolated_telegram_e2e_server):
    """Updates queued via the mock API are picked up by the polling loop."""
    base_url = isolated_telegram_e2e_server["base_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    await activate_telegram(
        base_url,
        isolated_telegram_e2e_server["http_url"],
        fake_tg_url,
        channels_dir,
    )

    await reset_fake_tg(fake_tg_url)

    # Queue an update via the mock control endpoint (simulates polling mode)
    async with httpx.AsyncClient() as c:
        await c.post(
            f"{fake_tg_url}/__mock/queue_update",
            json={
                "update_id": 700,
                "message": {
                    "message_id": 70,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "hello",
                },
            },
            timeout=5,
        )

    # Wait for the host polling loop to pick up the update and reply
    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    assert len(messages) >= 1, f"Expected at least one reply, got: {messages}"
    assert messages[-1]["chat_id"] == OWNER_USER_ID

    # Receiving a reply for a queued update proves the polling path is active.


async def test_telegram_markdown_fallback(telegram_e2e_server):
    """When Telegram rejects Markdown formatting, the bot retries as plain text."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)
    await set_reject_markdown(fake_tg_url, True)

    try:
        resp = await post_telegram_webhook(
            http_url,
            {
                "update_id": 800,
                "message": {
                    "message_id": 80,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "hello",
                },
            },
            secret=WEBHOOK_SECRET,
        )
        assert resp.status_code == 200

        # Wait for the plain-text retry to succeed (appears in sent_messages)
        messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
        assert len(messages) >= 1, f"Expected at least one message, got: {messages}"

        # Verify the retry sequence in api_calls:
        # First sendMessage has parse_mode (rejected with 400),
        # second sendMessage has no parse_mode (success)
        api_calls = await get_api_calls(fake_tg_url)
        send_calls = [c for c in api_calls if c["method"] == "sendMessage"]
        assert len(send_calls) >= 2, (
            f"Expected at least 2 sendMessage calls (rejected + retry), "
            f"got {len(send_calls)}: {send_calls}"
        )

        # First call should have parse_mode (was rejected)
        assert "parse_mode" in send_calls[0].get("body", {}), (
            f"First sendMessage should have parse_mode: {send_calls[0]}"
        )
        # Second call should NOT have parse_mode (plain-text fallback)
        assert "parse_mode" not in send_calls[1].get("body", {}), (
            f"Retry sendMessage should not have parse_mode: {send_calls[1]}"
        )
    finally:
        await set_reject_markdown(fake_tg_url, False)


async def test_telegram_missing_webhook_secret_rejected(telegram_e2e_server):
    """Webhook with no secret header at all is rejected with 401."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    # POST without any secret header (secret=None means no header is sent)
    resp = await post_telegram_webhook(
        http_url,
        {
            "update_id": 900,
            "message": {
                "message_id": 90,
                "from": {"id": OWNER_USER_ID, "is_bot": False, "first_name": "E2E"},
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "should be rejected",
            },
        },
        secret=None,
    )
    assert resp.status_code in (401, 403), (
        f"Expected 401/403 for missing secret, got {resp.status_code}: {resp.text}"
    )


async def test_telegram_rate_limit_resilience(telegram_e2e_server):
    """Bot survives Telegram 429 rate limiting and can send after it clears."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)

    # Make the next 20 sendMessage calls return 429. Using a high count
    # so the test is resilient to changes in the WASM channel's retry
    # strategy (e.g., Markdown attempt + plain-text fallback + future retries).
    await set_rate_limit(fake_tg_url, 20)

    try:
        # Send a webhook — the bot will process it but sendMessage will get 429
        resp = await post_telegram_webhook(
            http_url,
            {
                "update_id": 1000,
                "message": {
                    "message_id": 100,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "hello",
                },
            },
            secret=WEBHOOK_SECRET,
        )
        assert resp.status_code == 200

        # Wait for the bot to attempt sendMessage (it will be rejected with 429)
        send_calls = await wait_for_api_call(fake_tg_url, "sendMessage", timeout=15)
        assert len(send_calls) >= 1, f"Expected sendMessage attempt, got: {send_calls}"

        # Verify no messages were actually delivered (all got 429)
        async with httpx.AsyncClient() as c:
            r = await c.get(f"{fake_tg_url}/__mock/sent_messages", timeout=5)
        messages = r.json().get("messages", [])
        assert len(messages) == 0, (
            f"Expected no delivered messages during rate limit, got: {messages}"
        )

        # Now clear rate limiting and send another message
        await set_rate_limit(fake_tg_url, 0)
        await reset_fake_tg(fake_tg_url)

        resp2 = await post_telegram_webhook(
            http_url,
            {
                "update_id": 1001,
                "message": {
                    "message_id": 101,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "hello",
                },
            },
            secret=WEBHOOK_SECRET,
        )
        assert resp2.status_code == 200

        # Verify the bot recovered and sent a reply
        messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
        assert len(messages) >= 1, (
            f"Expected bot to recover after rate limit, got: {messages}"
        )
        assert messages[-1]["chat_id"] == OWNER_USER_ID
    finally:
        await set_rate_limit(fake_tg_url, 0)


async def test_telegram_document_download_failure_graceful(telegram_e2e_server):
    """Bot still replies to message text when document download fails."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)
    await set_fail_downloads(fake_tg_url, True)

    try:
        # Send a webhook with a document attachment AND text content
        resp = await post_telegram_webhook(
            http_url,
            {
                "update_id": 1100,
                "message": {
                    "message_id": 110,
                    "from": {
                        "id": OWNER_USER_ID,
                        "is_bot": False,
                        "first_name": "E2E Tester",
                    },
                    "chat": {"id": OWNER_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "hello",
                    "document": {
                        "file_id": "test_doc_fail_123",
                        "file_unique_id": "unique_test_doc_fail_123",
                        "file_name": "report.pdf",
                        "mime_type": "application/pdf",
                        "file_size": 2048,
                    },
                },
            },
            secret=WEBHOOK_SECRET,
        )
        assert resp.status_code == 200

        # Verify the bot still replies to the text content despite download failure
        messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
        assert len(messages) >= 1, (
            f"Expected bot to reply despite download failure, got: {messages}"
        )
        assert messages[-1]["chat_id"] == OWNER_USER_ID

        # Verify getFile was attempted (and failed)
        api_calls = await get_api_calls(fake_tg_url)
        get_file_calls = [c for c in api_calls if c["method"] == "getFile"]
        assert len(get_file_calls) >= 1, (
            f"Expected getFile attempt, got: {[c['method'] for c in api_calls]}"
        )
    finally:
        await set_fail_downloads(fake_tg_url, False)


async def test_telegram_malformed_payload_resilience(telegram_e2e_server):
    """Malformed JSON webhook is accepted gracefully; bot continues working after."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    await activate_telegram(base_url, http_url, fake_tg_url, channels_dir)

    await reset_fake_tg(fake_tg_url)

    # Send a completely malformed payload (not valid Telegram update JSON)
    headers = {
        "Content-Type": "application/json",
        "X-Telegram-Bot-Api-Secret-Token": WEBHOOK_SECRET,
    }
    async with httpx.AsyncClient() as c:
        resp = await c.post(
            f"{http_url}/webhook/telegram",
            content=b'{"not_a_valid_update": true}',
            headers=headers,
            timeout=10,
        )
    # The WASM channel returns 200 for malformed payloads to prevent
    # Telegram from retrying the same broken update forever.
    assert resp.status_code == 200, (
        f"Expected 200 for malformed payload, got {resp.status_code}: {resp.text}"
    )

    # Wait a moment, verify no replies were sent
    await asyncio.sleep(2)
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{fake_tg_url}/__mock/sent_messages", timeout=5)
    messages = r.json().get("messages", [])
    assert len(messages) == 0, (
        f"Expected no replies for malformed payload, got: {messages}"
    )

    # Verify the bot still works after receiving the bad payload
    await reset_fake_tg(fake_tg_url)

    resp2 = await post_telegram_webhook(
        http_url,
        {
            "update_id": 1200,
            "message": {
                "message_id": 120,
                "from": {
                    "id": OWNER_USER_ID,
                    "is_bot": False,
                    "first_name": "E2E Tester",
                },
                "chat": {"id": OWNER_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    assert resp2.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
    assert len(messages) >= 1, (
        f"Expected bot to work after malformed payload, got: {messages}"
    )
