"""Telegram channel install + setup helpers for workflow-canary.

The Telegram WASM channel ships in the bundled extensions directory.
For workflow-canary we need it installed AND patched to skip
``validation_endpoint`` (which would call out to the real
``api.telegram.org`` and fail SSRF protection in tests). Mirrors the
pattern in ``tests/e2e/scenarios/test_telegram_e2e.py`` —
``_patch_capabilities_for_testing`` + ``activate_telegram`` — adapted
for the workflow-canary's gateway stack.
"""

from __future__ import annotations

import json
import os
import time
from typing import Any

import httpx

BOT_TOKEN = "111222333:CANARY"
WEBHOOK_SECRET = "canary-webhook-secret"


def patch_capabilities(channels_dir: str) -> dict[str, Any]:
    """Patch the installed Telegram capabilities for canary testing.

    1. Remove ``validation_endpoint`` so setup doesn't attempt a live
       getMe against the real Telegram API.
    2. Ensure ``telegram_webhook_secret`` is in ``required_secrets``
       with ``auto_generate``.
    3. Ensure the webhook block declares ``secret_name`` and
       ``secret_header``.
    """
    cap_path = os.path.join(channels_dir, "telegram.capabilities.json")
    if not os.path.exists(cap_path):
        raise RuntimeError(
            f"Telegram capabilities not found at {cap_path}. "
            f"Was the channel installed first?"
        )
    with open(cap_path, "r", encoding="utf-8") as f:
        caps = json.load(f)

    if "setup" in caps and "validation_endpoint" in caps["setup"]:
        del caps["setup"]["validation_endpoint"]

    setup = caps.setdefault("setup", {})
    required = setup.setdefault("required_secrets", [])
    if not any(s.get("name") == "telegram_webhook_secret" for s in required):
        required.append(
            {
                "name": "telegram_webhook_secret",
                "prompt": "Webhook secret (auto-generated for tests)",
                "optional": True,
                "auto_generate": {"length": 64},
            }
        )

    channel = caps.setdefault("capabilities", {}).setdefault("channel", {})
    webhook = channel.setdefault("webhook", {})
    webhook.setdefault("secret_name", "telegram_webhook_secret")
    webhook.setdefault("secret_header", "X-Telegram-Bot-Api-Secret-Token")

    with open(cap_path, "w", encoding="utf-8") as f:
        json.dump(caps, f, indent=2)
    return caps


async def install_telegram_channel(
    base_url: str, gateway_token: str
) -> dict[str, Any]:
    headers = {"Authorization": f"Bearer {gateway_token}"}
    async with httpx.AsyncClient(timeout=180.0) as client:
        response = await client.post(
            f"{base_url}/api/extensions/install",
            headers=headers,
            json={"name": "telegram", "kind": "wasm_channel"},
        )
    # 200 = freshly installed, 409 = already installed — both are fine
    if response.status_code not in (200, 409):
        raise RuntimeError(
            f"Telegram install failed ({response.status_code}): "
            f"{response.text[:300]}"
        )
    try:
        return response.json()
    except Exception:
        return {}


async def setup_telegram_channel(
    base_url: str,
    gateway_token: str,
    *,
    bot_token: str = BOT_TOKEN,
    webhook_secret: str = WEBHOOK_SECRET,
) -> dict[str, Any]:
    headers = {"Authorization": f"Bearer {gateway_token}"}
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.post(
            f"{base_url}/api/extensions/telegram/setup",
            headers=headers,
            json={
                "secrets": {
                    "telegram_bot_token": bot_token,
                    "telegram_webhook_secret": webhook_secret,
                },
                "fields": {},
            },
        )
    body = response.json() if response.text else {}
    if response.status_code != 200 or not body.get("success"):
        raise RuntimeError(
            f"Telegram setup failed ({response.status_code}): "
            f"{response.text[:300]}"
        )
    return body


def _find_telegram(payload: dict[str, Any]) -> dict[str, Any] | None:
    """Pull the telegram entry out of a `/api/extensions` response.

    The list lives under different envelope keys depending on the
    gateway version that produced the response — accept all three
    historical shapes.
    """
    items = (
        payload.get("extensions")
        or payload.get("items")
        or payload.get("installed")
        or []
    )
    for ext in items:
        if ext.get("name") == "telegram":
            return ext
    return None


async def is_telegram_installed(
    base_url: str, gateway_token: str, *, timeout_secs: float = 30.0
) -> bool:
    """Poll /api/extensions until the telegram entry is **present**.

    "Installed" is a strictly weaker condition than "active" — an
    extension can be installed but inactive (mid-setup, awaiting auth,
    activation_error). Use ``wait_for_telegram_active`` instead when
    callers need to gate on actual runtime readiness; the
    distinction matters because skipping ``setup_telegram_channel()``
    for an installed-but-inactive extension is exactly the bug
    Copilot AI flagged on PR #2874.
    """
    import asyncio

    headers = {"Authorization": f"Bearer {gateway_token}"}
    deadline = time.monotonic() + timeout_secs
    async with httpx.AsyncClient(timeout=15.0) as client:
        while time.monotonic() < deadline:
            response = await client.get(
                f"{base_url}/api/extensions", headers=headers
            )
            if response.status_code == 200:
                if _find_telegram(response.json()) is not None:
                    return True
            await asyncio.sleep(0.5)
    return False


async def wait_for_telegram_active(
    base_url: str, gateway_token: str, *, timeout_secs: float = 30.0
) -> bool:
    """Poll /api/extensions until the telegram entry has ``active=true``.

    Mirrors the contract the canary's setup flow expects after a
    successful ``/api/extensions/telegram/setup`` call: the extension
    is installed AND its runtime activation has succeeded (channel
    opened, hooks registered, credentials bound — see
    `.claude/rules/lifecycle.md`). An ``activation_error``-bearing or
    needs-setup entry returns False so callers re-run setup.
    """
    import asyncio

    headers = {"Authorization": f"Bearer {gateway_token}"}
    deadline = time.monotonic() + timeout_secs
    async with httpx.AsyncClient(timeout=15.0) as client:
        while time.monotonic() < deadline:
            response = await client.get(
                f"{base_url}/api/extensions", headers=headers
            )
            if response.status_code == 200:
                ext = _find_telegram(response.json())
                if ext is not None and ext.get("active") is True:
                    return True
            await asyncio.sleep(0.5)
    return False


async def post_telegram_webhook(
    http_url: str,
    update: dict[str, Any],
    *,
    secret: str = WEBHOOK_SECRET,
) -> httpx.Response:
    """POST a Telegram-shaped update to IronClaw's webhook endpoint."""
    headers = {
        "Content-Type": "application/json",
        "X-Telegram-Bot-Api-Secret-Token": secret,
    }
    async with httpx.AsyncClient(timeout=15.0) as client:
        return await client.post(
            f"{http_url}/webhook/telegram",
            json=update,
            headers=headers,
        )


import re as _re

PAIRING_CODE_RE = _re.compile(r"approve telegram ([A-Z0-9]+)|`([A-Z0-9]+)`")


def extract_pairing_code(messages: list[dict[str, Any]]) -> str | None:
    """Pull a pairing code out of a list of mock_telegram messages."""
    for message in reversed(messages):
        text = message.get("text", "") or ""
        match = PAIRING_CODE_RE.search(text)
        if match:
            return match.group(1) or match.group(2)
    return None


async def approve_pairing(
    base_url: str, gateway_token: str, code: str
) -> None:
    headers = {"Authorization": f"Bearer {gateway_token}"}
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.post(
            f"{base_url}/api/pairing/telegram/approve",
            headers=headers,
            json={"code": code},
        )
    if response.status_code != 200:
        raise RuntimeError(
            f"Pairing approval failed ({response.status_code}): "
            f"{response.text[:300]}"
        )
    body = response.json()
    if not body.get("success"):
        raise RuntimeError(f"Pairing approval returned: {body}")


async def pair_telegram_user(
    base_url: str,
    gateway_token: str,
    http_url: str,
    mock_telegram_url: str,
    *,
    user_id: int,
    first_name: str,
    update_id: int,
) -> bool:
    """Send a pairing message → wait for code → approve.

    Returns True if pairing succeeded, False otherwise. Mirrors the
    test_telegram_e2e.py flow but adapted to workflow_canary's mock
    URLs. Resets mock_telegram before so we capture only the pairing
    prompt.
    """
    import asyncio as _asyncio

    async with httpx.AsyncClient(timeout=5.0) as client:
        await client.post(f"{mock_telegram_url}/__mock/reset")

    response = await post_telegram_webhook(
        http_url,
        {
            "update_id": update_id,
            "message": {
                "message_id": 1,
                "from": {
                    "id": user_id,
                    "is_bot": False,
                    "first_name": first_name,
                },
                "chat": {"id": user_id, "type": "private"},
                "date": int(time.time()),
                "text": "hello",
            },
        },
        secret=WEBHOOK_SECRET,
    )
    if response.status_code != 200:
        return False

    deadline = time.monotonic() + 30.0
    code: str | None = None
    while time.monotonic() < deadline:
        async with httpx.AsyncClient(timeout=5.0) as client:
            r = await client.get(f"{mock_telegram_url}/__mock/sent_messages")
        messages = r.json().get("messages", [])
        code = extract_pairing_code(messages)
        if code:
            break
        await _asyncio.sleep(0.5)

    if code is None:
        return False

    await approve_pairing(base_url, gateway_token, code)
    return True
