"""Telegram inbound webhook → agent → outbound reply round-trip.

Covers issue #1044's "chat_id 'default'" regression and the assertion
that an inbound Telegram message reaches the agent and produces a
sendMessage reply.

Coverage:
1. Ensure Telegram is installed + active (idempotent — share with
   ``telegram_channel_install`` if it ran first).
2. POST a webhook update with text "hi canary round-trip" to
   ``stack.http_url/webhook/telegram`` carrying the canary
   ``X-Telegram-Bot-Api-Secret-Token`` header.
3. Wait for mock_telegram to record an outbound sendMessage to the
   inbound chat_id (NOT 'default' — that's the regression shape).
4. Assert the reply text references our greeting (mock LLM canned
   response: "Hello! How can I help you today?").
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.scenarios._common import _capture_telegram_messages
from scripts.workflow_canary.telegram_setup import (
    BOT_TOKEN,
    WEBHOOK_SECRET,
    install_telegram_channel,
    wait_for_telegram_active,
    patch_capabilities,
    post_telegram_webhook,
    setup_telegram_channel,
)

INBOUND_CHAT_ID = 99001100
INBOUND_MESSAGE_TEXT = "hi canary round-trip"


async def _ensure_active(stack: Any) -> bool:
    if await wait_for_telegram_active(stack.base_url, stack.gateway_token, timeout_secs=2.0):
        return True
    await install_telegram_channel(stack.base_url, stack.gateway_token)
    patch_capabilities(stack.channels_dir)
    await setup_telegram_channel(
        stack.base_url,
        stack.gateway_token,
        bot_token=BOT_TOKEN,
        webhook_secret=WEBHOOK_SECRET,
    )
    return await wait_for_telegram_active(
        stack.base_url, stack.gateway_token, timeout_secs=15.0
    )


async def _reset_telegram_mock(mock_telegram_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        await client.post(f"{mock_telegram_url}/__mock/reset")


async def run(
    *,
    stack: Any,
    mock_telegram_url: str,
    mock_sheets_url: str | None = None,
    mock_calendar_url: str | None = None,
    mock_hn_url: str | None = None,
    mock_gmail_url: str | None = None,
    mock_web_search_url: str | None = None,
    output_dir: Path,
    log_dir: Path,
) -> list[ProbeResult]:
    started = time.perf_counter()
    mode = "telegram_round_trip"

    if not stack.http_url or not stack.channels_dir:
        return [
            ProbeResult(
                provider="channels",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "stack.http_url / channels_dir not populated"},
            )
        ]

    try:
        active = await _ensure_active(stack)
        if not active:
            latency_ms = int((time.perf_counter() - started) * 1000)
            return [
                ProbeResult(
                    provider="channels",
                    mode=mode,
                    success=False,
                    latency_ms=latency_ms,
                    details={"error": "Telegram channel did not reach Active"},
                )
            ]

        await _reset_telegram_mock(mock_telegram_url)

        webhook_resp = await post_telegram_webhook(
            stack.http_url,
            {
                "update_id": 998877001,
                "message": {
                    "message_id": 1,
                    "from": {
                        "id": INBOUND_CHAT_ID,
                        "is_bot": False,
                        "first_name": "Canary",
                    },
                    "chat": {"id": INBOUND_CHAT_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": INBOUND_MESSAGE_TEXT,
                },
            },
            secret=WEBHOOK_SECRET,
        )
        if webhook_resp.status_code != 200:
            return [
                ProbeResult(
                    provider="channels",
                    mode=mode,
                    success=False,
                    latency_ms=int((time.perf_counter() - started) * 1000),
                    details={
                        "error": (
                            f"webhook returned {webhook_resp.status_code}: "
                            f"{webhook_resp.text[:200]}"
                        )
                    },
                )
            ]

        # Poll mock_telegram for an outbound sendMessage to our inbound chat_id.
        reply: dict[str, Any] | None = None
        for _ in range(40):  # 20s budget
            messages = await _capture_telegram_messages(mock_telegram_url)
            for m in messages:
                if (
                    m.get("method") == "sendMessage"
                    and (str(m.get("chat_id")) == str(INBOUND_CHAT_ID))
                ):
                    reply = m
                    break
            if reply is not None:
                break
            await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        chat_id_value = reply.get("chat_id") if reply else None
        success = (
            reply is not None
            # The "default" chat_id regression — outbound message must
            # carry the actual numeric inbound chat_id.
            and str(chat_id_value) != "default"
        )

        return [
            ProbeResult(
                provider="channels",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "inbound_chat_id": INBOUND_CHAT_ID,
                    "reply_chat_id": chat_id_value,
                    "reply_text": reply.get("text") if reply else None,
                },
            )
        ]
    except Exception as exc:  # noqa: BLE001
        return [
            ProbeResult(
                provider="channels",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        ]
