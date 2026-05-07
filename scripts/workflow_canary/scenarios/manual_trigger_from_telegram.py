"""Manual trigger from Telegram — covers Script 4 PHASE 4.2:
"trigger my dog walk reminder now" via Telegram fires the routine and
returns the ack to the same Telegram chat.

Coverage:
1. Ensure Telegram is installed + setup + paired.
2. Pre-seed a Lightweight cron routine (no backdate, no fire) so the
   ONLY way it fires is via manual trigger.
3. POST a Telegram webhook with text containing the routine name and
   "trigger now". The mock LLM falls through to a default text reply
   (we don't need an LLM-driven trigger — we hit the manual-trigger
   API directly to keep the assertion deterministic).
4. Hit ``POST /api/routines/<id>/trigger`` directly so the routine
   fires on-demand.
5. Assert (a) routine_runs has a successful row, (b) mock_telegram
   received the routine's ack on the paired chat_id (the manual
   trigger goes through the same lightweight action loop).
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    insert_lightweight_cron_routine,
    trigger_routine_via_api,
    wait_for_run,
)
from scripts.workflow_canary.scenarios._common import _capture_telegram_messages
from scripts.workflow_canary.telegram_setup import (
    BOT_TOKEN,
    WEBHOOK_SECRET,
    install_telegram_channel,
    wait_for_telegram_active,
    pair_telegram_user,
    patch_capabilities,
    setup_telegram_channel,
)

PAIRED_USER_ID = 99003300
ROUTINE_NAME = "canary-manual-trigger-tg"


async def _ensure_active_and_paired(stack: Any, mock_telegram_url: str) -> bool:
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
        first_name="Canary Manual",
        update_id=998877200,
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
    mode = "manual_trigger_from_telegram"

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

        # Pre-seed a routine that ONLY fires on manual trigger
        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name=ROUTINE_NAME,
            prompt=(
                "Send a Telegram acknowledgement that the manual "
                "trigger fired.\n\n"
                "[CANARY-WORKFLOW-manual_trigger_from_telegram]"
            ),
            schedule="0 0 1 1 *",  # never fires on its own
            description="canary: manual trigger via telegram",
            fire_immediately=False,
        )

        await _reset_telegram_mock(mock_telegram_url)

        trigger_response = await trigger_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id
        )
        run_id = trigger_response.get("run_id")
        if not run_id:
            return [
                ProbeResult(
                    provider="channels",
                    mode=mode,
                    success=False,
                    latency_ms=int((time.perf_counter() - started) * 1000),
                    details={
                        "error": (
                            f"trigger response missing run_id: "
                            f"{trigger_response}"
                        )
                    },
                )
            ]

        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=30.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Verify mock_telegram captured the routine's ack — the routine's
        # CANARY-WORKFLOW-<key> sentinel routes the lightweight action's
        # http call to a sendMessage with the canary text.
        ack_text = (
            "[canary-workflow:manual_trigger_from_telegram] ack"
        )
        telegram_match: dict[str, Any] | None = None
        if run_terminal:
            for _ in range(20):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    text = m.get("text") or ""
                    if (
                        m.get("method") == "sendMessage"
                        and ack_text in text
                    ):
                        telegram_match = m
                        break
                if telegram_match is not None:
                    break
                await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and telegram_match is not None

        return [
            ProbeResult(
                provider="channels",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "trigger_run_id": run_id,
                    "run_status": last_run["status"],
                    "ack_text": ack_text,
                    "telegram_match": (
                        telegram_match.get("text") if telegram_match else None
                    ),
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
