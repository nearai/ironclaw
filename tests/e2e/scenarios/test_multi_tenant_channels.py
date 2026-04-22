"""Multi-tenant Telegram channel E2E tests.

Two users independently activate the same Telegram WASM channel, each
getting a unique dispatch key and webhook path. Verifies:

1. Both tenants get distinct dispatch keys (webhook paths)
2. Webhook messages route to the correct tenant's replies
3. Cross-tenant webhook delivery (wrong secret) is rejected
4. Thread isolation: no shared thread IDs between tenants
5. Extensions list shows correct per-tenant active status
6. An unactivated user sees Telegram as inactive
"""

import asyncio
import json
import os
import re
import time
from itertools import count

import httpx
import pytest

from helpers import api_get, api_post, auth_headers, create_member_user

# Per-tenant bot tokens (distinct so they're traceable in the fake API)
ALICE_BOT_TOKEN = "111222333:FAKE_ALICE_TOKEN"
BOB_BOT_TOKEN = "444555666:FAKE_BOB_TOKEN"

# Per-tenant webhook secrets
ALICE_WEBHOOK_SECRET = "e2e-alice-webhook-secret"
BOB_WEBHOOK_SECRET = "e2e-bob-webhook-secret"

# Per-tenant Telegram user IDs (used in webhook payloads)
ALICE_TG_USER_ID = 100001
BOB_TG_USER_ID = 200002

PAIRING_CODE_RE = re.compile(r"approve telegram ([A-Z0-9]+)|`([A-Z0-9]+)`")

# Monotonic update IDs that don't collide with other Telegram E2E test modules.
_UPDATE_IDS = count(5000)


def _next_update_id() -> int:
    return next(_UPDATE_IDS)


# ── helpers ──────────────────────────────────────────────────────────────


async def reset_fake_tg(fake_tg_url: str):
    async with httpx.AsyncClient() as c:
        await c.post(f"{fake_tg_url}/__mock/reset")


async def wait_for_sent_messages(
    fake_tg_url: str,
    *,
    min_count: int = 1,
    timeout: float = 30,
) -> list[dict]:
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


async def get_active_dispatch_keys(http_url: str) -> list[str]:
    """Get all active WASM channel dispatch keys from the router health endpoint."""
    async with httpx.AsyncClient() as c:
        r = await c.get(f"{http_url}/wasm-channels/health", timeout=5)
        r.raise_for_status()
        return r.json().get("channels", [])


async def install_telegram(base_url: str):
    r = await api_post(
        base_url,
        "/api/extensions/install",
        json={"name": "telegram", "kind": "wasm_channel"},
        timeout=180,
    )
    assert r.status_code in (200, 409), (
        f"Telegram install failed ({r.status_code}): {r.text}"
    )


def patch_capabilities_for_testing(channels_dir: str):
    """Remove validation_endpoint and ensure webhook secret is declared."""
    cap_path = os.path.join(channels_dir, "telegram.capabilities.json")
    assert os.path.exists(cap_path), (
        f"Capabilities not found: {os.listdir(channels_dir)}"
    )
    with open(cap_path, "r") as f:
        caps = json.load(f)
    if "setup" in caps and "validation_endpoint" in caps["setup"]:
        del caps["setup"]["validation_endpoint"]
    setup = caps.setdefault("setup", {})
    required = setup.setdefault("required_secrets", [])
    if not any(s.get("name") == "telegram_webhook_secret" for s in required):
        required.append({
            "name": "telegram_webhook_secret",
            "prompt": "Webhook secret",
            "optional": True,
            "auto_generate": {"length": 64},
        })
    channel = caps.setdefault("capabilities", {}).setdefault("channel", {})
    webhook = channel.setdefault("webhook", {})
    webhook.setdefault("secret_name", "telegram_webhook_secret")
    webhook.setdefault("secret_header", "X-Telegram-Bot-Api-Secret-Token")
    with open(cap_path, "w") as f:
        json.dump(caps, f, indent=2)


def extract_pairing_code(messages: list[dict]) -> str | None:
    for message in reversed(messages):
        text = message.get("text", "")
        match = PAIRING_CODE_RE.search(text)
        if match:
            return match.group(1) or match.group(2)
    return None


async def post_webhook(
    http_url: str,
    dispatch_key: str,
    update: dict,
    *,
    secret: str | None = None,
) -> httpx.Response:
    """POST a Telegram-shaped update to a specific dispatch key's webhook path."""
    headers = {"Content-Type": "application/json"}
    if secret is not None:
        headers["X-Telegram-Bot-Api-Secret-Token"] = secret
    async with httpx.AsyncClient() as c:
        return await c.post(
            f"{http_url}/webhook/{dispatch_key}",
            json=update,
            headers=headers,
            timeout=10,
        )


async def approve_pairing(base_url: str, code: str, *, token: str) -> None:
    async with httpx.AsyncClient() as c:
        response = await c.post(
            f"{base_url}/api/pairing/telegram/approve",
            headers=auth_headers(token),
            json={"code": code},
            timeout=10,
        )
    response.raise_for_status()
    body = response.json()
    assert body.get("success"), f"Pairing approval failed: {body}"


async def activate_tenant_telegram(
    base_url: str,
    http_url: str,
    fake_tg_url: str,
    *,
    user_token: str,
    bot_token: str,
    webhook_secret: str,
    tg_user_id: int,
    first_name: str,
) -> str:
    """Activate Telegram for a specific tenant. Returns the dispatch key.

    1. Snapshots active dispatch keys before activation
    2. Calls the setup API with per-tenant credentials
    3. Discovers the new dispatch key by diffing the health endpoint
    4. Completes the pairing flow so the Telegram user can chat
    """
    keys_before = set(await get_active_dispatch_keys(http_url))
    await reset_fake_tg(fake_tg_url)

    r = await api_post(
        base_url,
        "/api/extensions/telegram/setup",
        token=user_token,
        json={
            "secrets": {
                "telegram_bot_token": bot_token,
                "telegram_webhook_secret": webhook_secret,
            },
            "fields": {},
        },
        timeout=30,
    )
    r.raise_for_status()
    body = r.json()
    assert body.get("activated"), f"Tenant setup did not activate: {body}"

    # Discover the new dispatch key by polling the health endpoint
    dispatch_key = None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        keys_after = set(await get_active_dispatch_keys(http_url))
        new_keys = keys_after - keys_before
        telegram_keys = [k for k in new_keys if k.startswith("telegram")]
        if telegram_keys:
            dispatch_key = telegram_keys[0]
            break
        await asyncio.sleep(0.5)

    assert dispatch_key is not None, (
        f"Failed to discover dispatch key for {first_name}. "
        f"Keys before: {keys_before}, keys after activation: "
        f"{set(await get_active_dispatch_keys(http_url))}"
    )

    # Complete pairing: send a message, extract pairing code, approve
    await reset_fake_tg(fake_tg_url)
    pairing_resp = await post_webhook(
        http_url,
        dispatch_key,
        {
            "update_id": _next_update_id(),
            "message": {
                "message_id": 1,
                "from": {
                    "id": tg_user_id,
                    "is_bot": False,
                    "first_name": first_name,
                },
                "chat": {"id": tg_user_id, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=webhook_secret,
    )
    assert pairing_resp.status_code == 200

    messages = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    code = extract_pairing_code(messages)
    if code:
        await approve_pairing(base_url, code, token=user_token)

    await reset_fake_tg(fake_tg_url)
    return dispatch_key


# ── tests ────────────────────────────────────────────────────────────────


async def test_multi_tenant_telegram_channel_isolation(isolated_telegram_e2e_server):
    """Two tenants activate Telegram independently; verify full isolation."""
    base_url = isolated_telegram_e2e_server["base_url"]
    http_url = isolated_telegram_e2e_server["http_url"]
    fake_tg_url = isolated_telegram_e2e_server["fake_tg_url"]
    channels_dir = isolated_telegram_e2e_server["channels_dir"]

    # ── Setup: create three users, activate Telegram for two ──

    alice = await create_member_user(base_url, display_name="Alice Tenant")
    bob = await create_member_user(base_url, display_name="Bob Tenant")
    charlie = await create_member_user(base_url, display_name="Charlie Inactive")

    await install_telegram(base_url)
    patch_capabilities_for_testing(channels_dir)

    alice_dk = await activate_tenant_telegram(
        base_url, http_url, fake_tg_url,
        user_token=alice["token"],
        bot_token=ALICE_BOT_TOKEN,
        webhook_secret=ALICE_WEBHOOK_SECRET,
        tg_user_id=ALICE_TG_USER_ID,
        first_name="Alice TG",
    )

    bob_dk = await activate_tenant_telegram(
        base_url, http_url, fake_tg_url,
        user_token=bob["token"],
        bot_token=BOB_BOT_TOKEN,
        webhook_secret=BOB_WEBHOOK_SECRET,
        tg_user_id=BOB_TG_USER_ID,
        first_name="Bob TG",
    )

    # ── Assert 1: dispatch keys are unique and well-formed ──

    assert alice_dk != bob_dk, f"Both tenants got same dispatch key: {alice_dk}"
    assert alice_dk.startswith("telegram:"), (
        f"Unexpected Alice dispatch key format: {alice_dk}"
    )
    assert bob_dk.startswith("telegram:"), (
        f"Unexpected Bob dispatch key format: {bob_dk}"
    )

    # ── Assert 2: Alice's webhook → reply targets Alice's chat_id ──

    await reset_fake_tg(fake_tg_url)
    resp = await post_webhook(
        http_url,
        alice_dk,
        {
            "update_id": _next_update_id(),
            "message": {
                "message_id": 100,
                "from": {
                    "id": ALICE_TG_USER_ID,
                    "is_bot": False,
                    "first_name": "Alice TG",
                },
                "chat": {"id": ALICE_TG_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello from alice",
            },
        },
        secret=ALICE_WEBHOOK_SECRET,
    )
    assert resp.status_code == 200
    alice_msgs = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    assert any(m["chat_id"] == ALICE_TG_USER_ID for m in alice_msgs), (
        f"Expected reply to Alice's chat_id {ALICE_TG_USER_ID}, got: {alice_msgs}"
    )

    # ── Assert 3: Bob's webhook → reply targets Bob's chat_id ──

    await reset_fake_tg(fake_tg_url)
    resp = await post_webhook(
        http_url,
        bob_dk,
        {
            "update_id": _next_update_id(),
            "message": {
                "message_id": 200,
                "from": {
                    "id": BOB_TG_USER_ID,
                    "is_bot": False,
                    "first_name": "Bob TG",
                },
                "chat": {"id": BOB_TG_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "hello from bob",
            },
        },
        secret=BOB_WEBHOOK_SECRET,
    )
    assert resp.status_code == 200
    bob_msgs = await wait_for_sent_messages(fake_tg_url, min_count=1, timeout=60)
    assert any(m["chat_id"] == BOB_TG_USER_ID for m in bob_msgs), (
        f"Expected reply to Bob's chat_id {BOB_TG_USER_ID}, got: {bob_msgs}"
    )

    # ── Assert 4: threads are isolated per tenant ──

    alice_threads_r = await api_get(
        base_url, "/api/chat/threads", token=alice["token"]
    )
    alice_threads_r.raise_for_status()
    alice_threads = alice_threads_r.json().get("threads", [])

    bob_threads_r = await api_get(
        base_url, "/api/chat/threads", token=bob["token"]
    )
    bob_threads_r.raise_for_status()
    bob_threads = bob_threads_r.json().get("threads", [])

    alice_thread_ids = {t["id"] for t in alice_threads}
    bob_thread_ids = {t["id"] for t in bob_threads}
    assert alice_thread_ids.isdisjoint(bob_thread_ids), (
        f"Thread IDs overlap between tenants: "
        f"alice={alice_thread_ids}, bob={bob_thread_ids}"
    )

    # ── Assert 5: cross-tenant secret is rejected ──

    resp = await post_webhook(
        http_url,
        alice_dk,
        {
            "update_id": _next_update_id(),
            "message": {
                "message_id": 300,
                "from": {
                    "id": BOB_TG_USER_ID,
                    "is_bot": False,
                    "first_name": "Bob TG",
                },
                "chat": {"id": BOB_TG_USER_ID, "type": "private"},
                "date": int(time.time()),
                "text": "trying to reach alice with wrong secret",
            },
        },
        secret=BOB_WEBHOOK_SECRET,
    )
    assert resp.status_code in (401, 403), (
        f"Expected 401/403 for cross-tenant secret on Alice's path, "
        f"got {resp.status_code}: {resp.text}"
    )

    # ── Assert 6: nonexistent dispatch key returns 404 ──

    resp = await post_webhook(
        http_url,
        "telegram:nonexistent-key",
        {
            "update_id": _next_update_id(),
            "message": {
                "message_id": 400,
                "from": {"id": 999, "is_bot": False, "first_name": "Ghost"},
                "chat": {"id": 999, "type": "private"},
                "date": int(time.time()),
                "text": "nobody home",
            },
        },
        secret="irrelevant",
    )
    assert resp.status_code == 404, (
        f"Expected 404 for nonexistent dispatch key, got {resp.status_code}"
    )

    # ── Assert 7: extensions list shows correct per-tenant active status ──

    alice_exts_r = await api_get(
        base_url, "/api/extensions", token=alice["token"]
    )
    alice_exts_r.raise_for_status()
    alice_exts = alice_exts_r.json().get("extensions", [])
    alice_tg = next((e for e in alice_exts if e["name"] == "telegram"), None)
    assert alice_tg is not None, (
        f"Telegram not in Alice's extensions: {alice_exts}"
    )
    assert alice_tg["active"] is True, (
        f"Expected Telegram active for Alice: {alice_tg}"
    )

    bob_exts_r = await api_get(
        base_url, "/api/extensions", token=bob["token"]
    )
    bob_exts_r.raise_for_status()
    bob_exts = bob_exts_r.json().get("extensions", [])
    bob_tg = next((e for e in bob_exts if e["name"] == "telegram"), None)
    assert bob_tg is not None, (
        f"Telegram not in Bob's extensions: {bob_exts}"
    )
    assert bob_tg["active"] is True, (
        f"Expected Telegram active for Bob: {bob_tg}"
    )

    # Charlie never activated — should show inactive
    charlie_exts_r = await api_get(
        base_url, "/api/extensions", token=charlie["token"]
    )
    charlie_exts_r.raise_for_status()
    charlie_exts = charlie_exts_r.json().get("extensions", [])
    charlie_tg = next((e for e in charlie_exts if e["name"] == "telegram"), None)
    assert charlie_tg is not None, (
        f"Telegram not in Charlie's extensions: {charlie_exts}"
    )
    assert charlie_tg["active"] is False, (
        f"Expected Telegram inactive for Charlie (never activated): {charlie_tg}"
    )
