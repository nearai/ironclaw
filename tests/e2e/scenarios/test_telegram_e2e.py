"""Full-process Telegram E2E tests.

Boot IronClaw → activate Telegram via setup API → POST webhook updates
→ verify sendMessage round-trip through mock LLM to fake Telegram API.
"""

import asyncio
import json
import os
import time

import httpx

from helpers import api_post, auth_headers

# Bot token used throughout these tests.
BOT_TOKEN = "111222333:FAKE_E2E_TOKEN"
# Owner user id used in the verification message and subsequent webhooks.
OWNER_USER_ID = 42
# Fixed webhook secret supplied during setup so all tests can use it
# without extracting it from the server.
WEBHOOK_SECRET = "e2e-test-webhook-secret-for-telegram"


# ── helpers ──────────────────────────────────────────────────────────────


async def reset_fake_tg(fake_tg_url: str):
    async with httpx.AsyncClient() as c:
        await c.post(f"{fake_tg_url}/__mock/reset")


async def queue_verification_message(fake_tg_url: str, code: str):
    """Queue a /start message that matches the IronClaw verification code."""
    async with httpx.AsyncClient() as c:
        await c.post(
            f"{fake_tg_url}/__mock/queue_update",
            json={
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

    # 3. Ensure webhook section declares secret_name and secret_header
    channel = caps.setdefault("capabilities", {}).setdefault("channel", {})
    webhook = channel.setdefault("webhook", {})
    webhook.setdefault("secret_name", "telegram_webhook_secret")
    webhook.setdefault("secret_header", "X-Telegram-Bot-Api-Secret-Token")

    with open(cap_path, "w") as f:
        json.dump(caps, f, indent=2)


async def activate_telegram(
    base_url: str, fake_tg_url: str, channels_dir: str
) -> None:
    """Install (if needed) and run the two-step Telegram setup flow."""
    await reset_fake_tg(fake_tg_url)
    await install_telegram(base_url)

    # Patch capabilities for testing (remove validation_endpoint, ensure
    # webhook secret is declared in required_secrets).
    _patch_capabilities_for_testing(channels_dir)

    # Step 1: submit bot token AND a known webhook secret.
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
    assert body1.get("success"), f"First setup call failed: {body1}"
    verification = body1.get("verification")
    assert verification, f"No verification challenge returned: {body1}"
    code = verification["code"]

    # Queue the verification message on the fake Telegram API so the
    # second setup call finds it immediately via getUpdates.
    await queue_verification_message(fake_tg_url, code)

    # Step 2: trigger the polling call — this blocks until verification
    async with httpx.AsyncClient() as c:
        r2 = await c.post(
            f"{base_url}/api/extensions/telegram/setup",
            headers=auth_headers(),
            json={"secrets": {}, "fields": {}},
            timeout=60,
        )
    r2.raise_for_status()
    body2 = r2.json()
    assert body2.get("activated"), f"Second setup call did not activate: {body2}"


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


# ── tests ────────────────────────────────────────────────────────────────


async def test_telegram_setup_and_dm_roundtrip(telegram_e2e_server):
    """Full DM round-trip: setup → webhook → mock LLM → sendMessage."""
    base_url = telegram_e2e_server["base_url"]
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]
    channels_dir = telegram_e2e_server["channels_dir"]

    # Reset fake API and activate the Telegram channel
    await activate_telegram(base_url, fake_tg_url, channels_dir)

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
    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=30)
    reply_text = messages[-1].get("text", "")
    assert reply_text, f"Empty reply text. All sent messages: {messages}"
    assert messages[-1]["chat_id"] == OWNER_USER_ID


async def test_telegram_edited_message_roundtrip(telegram_e2e_server):
    """Edited-message webhook triggers a new agent reply."""
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]

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
    http_url = telegram_e2e_server["http_url"]
    fake_tg_url = telegram_e2e_server["fake_tg_url"]

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
    http_url = telegram_e2e_server["http_url"]

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
