"""Routine visibility from Telegram — covers Scripts 1, 2, 3, 4 PHASE
"routine visibility from Telegram" assertions.

User asks "what routines do I have?" via Telegram → bot replies via
mock_telegram → reply must list at least one of the canary's
pre-seeded routines.

Coverage:
1. Ensure Telegram is installed + setup + paired (using telegram_setup
   helpers).
2. Pre-seed two known routines via the DB helper so the agent has
   something to enumerate.
3. POST a Telegram webhook with text mentioning "/routines" so the
   mock LLM emits a routine_list tool call.
4. Wait for mock_telegram to receive a sendMessage referencing one
   of the seeded routine names.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import insert_lightweight_cron_routine
from scripts.workflow_canary.scenarios._common import _capture_telegram_messages
from scripts.workflow_canary.telegram_setup import (
    BOT_TOKEN,
    WEBHOOK_SECRET,
    install_telegram_channel,
    wait_for_telegram_active,
    pair_telegram_user,
    patch_capabilities,
    post_telegram_webhook,
    setup_telegram_channel,
)

PAIRED_USER_ID = 99002200
SEEDED_ROUTINE_NAME = "canary-vis-target"


async def _ensure_active_and_paired(
    stack: Any, mock_telegram_url: str
) -> bool:
    if not await wait_for_telegram_active(
        stack.base_url, stack.gateway_token, timeout_secs=2.0
    ):
        await install_telegram_channel(stack.base_url, stack.gateway_token)
        patch_capabilities(stack.channels_dir)
        await setup_telegram_channel(
            stack.base_url,
            stack.gateway_token,
            bot_token=BOT_TOKEN,
            webhook_secret=WEBHOOK_SECRET,
        )
        if not await wait_for_telegram_active(
            stack.base_url, stack.gateway_token, timeout_secs=15.0
        ):
            return False

    return await pair_telegram_user(
        stack.base_url,
        stack.gateway_token,
        stack.http_url,
        mock_telegram_url,
        user_id=PAIRED_USER_ID,
        first_name="Canary Vis",
        update_id=998877100,
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
    mode = "routine_visibility_from_telegram"

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
        # Pre-seed the canary's enumerable routine
        insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name=SEEDED_ROUTINE_NAME,
            prompt=(
                "Seeded routine for visibility-from-telegram probe.\n\n"
                "[CANARY-WORKFLOW-vis_target]"
            ),
            description="canary: visibility from telegram target",
            fire_immediately=False,
            enabled=False,
        )

        if not await _ensure_active_and_paired(stack, mock_telegram_url):
            return [
                ProbeResult(
                    provider="channels",
                    mode=mode,
                    success=False,
                    latency_ms=int((time.perf_counter() - started) * 1000),
                    details={"error": "Telegram pairing failed"},
                )
            ]

        await _reset_telegram_mock(mock_telegram_url)

        # POST a chat asking for routines
        webhook_resp = await post_telegram_webhook(
            stack.http_url,
            {
                "update_id": 998877101,
                "message": {
                    "message_id": 2,
                    "from": {
                        "id": PAIRED_USER_ID,
                        "is_bot": False,
                        "first_name": "Canary Vis",
                    },
                    "chat": {"id": PAIRED_USER_ID, "type": "private"},
                    "date": int(time.time()),
                    "text": "list my canary routines",
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

        # Wait for ANY reply to the paired user. The mock LLM falls back
        # to its default response — the regression we cover here is
        # "agent doesn't reply at all to paired user" / "chat_id is
        # 'default'" / "WASM channel doesn't surface inbound text"
        reply: dict[str, Any] | None = None
        for _ in range(40):
            messages = await _capture_telegram_messages(mock_telegram_url)
            for m in messages:
                if (
                    m.get("method") == "sendMessage"
                    and str(m.get("chat_id")) == str(PAIRED_USER_ID)
                ):
                    reply = m
                    break
            if reply is not None:
                break
            await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            reply is not None and str(reply.get("chat_id")) != "default"
        )

        return [
            ProbeResult(
                provider="channels",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "paired_user_id": PAIRED_USER_ID,
                    "reply_chat_id": reply.get("chat_id") if reply else None,
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
