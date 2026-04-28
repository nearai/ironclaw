"""NL-driven routine creation — covers issue #1044's
"create a routine that … every N minutes" assertions across all 5
scripts.

Original NL surface:
- Script 1 PHASE 3.1: "create a routine that checks for new Telegram
  messages every 2 minutes"
- Script 2 PHASE 3.1: "10 minutes before each Google Calendar
  meeting, send me a Telegram message…"
- Script 3 PHASE 2.1: "check Hacker News every hour for Show HN posts"
- Script 4 PHASE 2.1: "remind me to take my dog for a walk every 2
  minutes"
- Script 5 PHASE 4.1: "check my emails every hour and add any inbound
  sales leads to a Google Sheet"

All five resolve to the same back-end mechanism: the agent receives
the NL message in chat, decides to call ``routine_create``, the tool
inserts a row into the ``routines`` table, and the engine picks it up
on the next cron tick.

Coverage in this probe:

1. POST /api/chat/thread/new to get a thread.
2. POST /api/chat/send with a message tagged [CANARY-WORKFLOW-NL-CREATE].
   The mock LLM matches that sentinel and emits a deterministic
   ``routine_create`` tool call (see TOOL_CALL_PATTERNS in
   tests/e2e/mock_llm.py).
3. Poll the libSQL ``routines`` table for a row with the expected
   name. The agent's tool-dispatch path runs the
   ``routine_create`` tool inline.
4. Cross-check via ``GET /api/routines`` that the routine is also
   visible via the public API.
"""

from __future__ import annotations

import asyncio
import sqlite3
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import list_routines_via_api

EXPECTED_ROUTINE_NAME = "canary-nl-created"


async def _open_thread(base_url: str, gateway_token: str) -> str:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/thread/new",
            headers={"Authorization": f"Bearer {gateway_token}"},
        )
        response.raise_for_status()
        return response.json()["id"]


async def _send_chat(
    base_url: str, gateway_token: str, thread_id: str, content: str
) -> None:
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/send",
            headers={"Authorization": f"Bearer {gateway_token}"},
            json={"content": content, "thread_id": thread_id},
        )
        if response.status_code != 202:
            response.raise_for_status()


def _routine_exists(db_path: str | Path, name: str) -> dict[str, Any] | None:
    with sqlite3.connect(str(db_path)) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT id, name, trigger_type, trigger_config, "
            "action_type, action_config, enabled "
            "FROM routines WHERE name = ?",
            (name,),
        ).fetchone()
    return dict(row) if row else None


async def _wait_for_routine(
    db_path: str | Path, name: str, *, timeout_secs: float = 30.0
) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        row = _routine_exists(db_path, name)
        if row is not None:
            return row
        await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Routine {name!r} not found in DB within {timeout_secs:.0f}s"
    )


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
    mode = "nl_routine_create"

    try:
        thread_id = await _open_thread(stack.base_url, stack.gateway_token)
        await _send_chat(
            stack.base_url,
            stack.gateway_token,
            thread_id,
            (
                "Please create a cron routine called 'canary-nl-created' "
                "that fires every minute and sends a Telegram acknowledgement.\n\n"
                "[CANARY-WORKFLOW-NL-CREATE]"
            ),
        )

        # The agent dispatches routine_create as the chat turn proceeds.
        # We poll the DB rather than the chat history because the chat
        # turn may not finish until after the LLM produces a final text
        # response — but the routine row lands as soon as the tool
        # executes, which is what we care about.
        routine = await _wait_for_routine(
            stack.db_path, EXPECTED_ROUTINE_NAME, timeout_secs=45.0
        )

        # Cross-check via the public API
        listed = await list_routines_via_api(stack.base_url, stack.gateway_token)
        api_match = any(r.get("id") == routine["id"] for r in listed)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            routine["trigger_type"] == "cron"
            and routine["action_type"] == "lightweight"
            and bool(routine["enabled"])
            and api_match
        )

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine["id"],
                    "routine_name": routine["name"],
                    "trigger_type": routine["trigger_type"],
                    "action_type": routine["action_type"],
                    "enabled": bool(routine["enabled"]),
                    "api_match": api_match,
                },
            )
        ]
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=latency_ms,
                details={"error": f"timeout: {exc}"},
            )
        ]
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=latency_ms,
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        ]
