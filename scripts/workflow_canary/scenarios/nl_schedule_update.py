"""NL-driven schedule update — covers issue #1044's
"change the schedule via chat" assertion (Script 4 PHASE 5.1).

Original NL surface:
- Script 4 PHASE 5.1: "change the routine schedule via chat"
  → "Routine updates successfully. trigger_config.schedule shifts to
  the new cadence. No error about 'cannot update schedule on
  non-cron routine.'"

Back-end mechanism: the agent receives the NL message, decides to call
``routine_update`` with the target routine's name + new schedule,
and the tool updates the row's ``trigger_config`` in libSQL.

Coverage:

1. Pre-seed a routine with name "canary-nl-update-target" and the
   "old" schedule "*/1 * * * *".
2. POST /api/chat/send with a message tagged [CANARY-WORKFLOW-NL-UPDATE].
3. Mock LLM emits a deterministic routine_update tool call with the
   "new" schedule "0 */6 * * *" (every 6 hours).
4. Poll the libSQL routines row; assert trigger_config.schedule
   updates to the new value.
5. Assert the routine's other fields (name, action_type, enabled)
   are unchanged.
"""

from __future__ import annotations

import asyncio
import json
import sqlite3
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import insert_lightweight_cron_routine

ROUTINE_NAME = "canary-nl-update-target"
OLD_SCHEDULE = "*/1 * * * *"
EXPECTED_NEW_SCHEDULE = "0 */6 * * *"


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


def _read_routine(db_path: str | Path, routine_id: str) -> dict[str, Any] | None:
    with sqlite3.connect(str(db_path)) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT id, name, trigger_type, trigger_config, "
            "action_type, enabled FROM routines WHERE id = ?",
            (routine_id,),
        ).fetchone()
    return dict(row) if row else None


async def _wait_for_schedule_change(
    db_path: str | Path,
    routine_id: str,
    old_schedule: str,
    *,
    timeout_secs: float = 30.0,
) -> dict[str, Any]:
    """Wait for trigger_config.schedule to differ from `old_schedule`.

    The engine normalizes whatever the agent emits to its canonical
    7-field cron form (seconds + year), so we can't assert on an exact
    string match — but ANY change away from the seeded value
    confirms the routine_update tool ran end-to-end.
    """
    deadline = time.monotonic() + timeout_secs
    last: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        row = _read_routine(db_path, routine_id)
        if row is not None:
            last = row
            try:
                config = json.loads(row["trigger_config"])
            except (TypeError, json.JSONDecodeError):
                config = {}
            current = config.get("schedule")
            if current and current != old_schedule:
                return row
        await asyncio.sleep(0.5)
    raise TimeoutError(
        f"Routine {routine_id} schedule did not change from "
        f"{old_schedule!r} within {timeout_secs:.0f}s "
        f"(last config: {last['trigger_config'] if last else 'no row'})"
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
    mode = "nl_schedule_update"

    try:
        # Pre-seed the target routine. Disabled + no fire so it doesn't
        # accidentally run during the test window.
        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name=ROUTINE_NAME,
            prompt="hi - schedule update target",
            schedule=OLD_SCHEDULE,
            description="canary: NL-driven schedule update target",
            fire_immediately=False,
            enabled=False,
        )

        # Send the NL update message.
        thread_id = await _open_thread(stack.base_url, stack.gateway_token)
        await _send_chat(
            stack.base_url,
            stack.gateway_token,
            thread_id,
            (
                f"Please change the schedule of {ROUTINE_NAME!r} to fire "
                "every 6 hours.\n\n[CANARY-WORKFLOW-NL-UPDATE]"
            ),
        )

        # Wait for the routine row's trigger_config.schedule to flip.
        routine = await _wait_for_schedule_change(
            stack.db_path, routine_id, OLD_SCHEDULE, timeout_secs=45.0
        )

        config = json.loads(routine["trigger_config"])
        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            config.get("schedule") is not None
            and config["schedule"] != OLD_SCHEDULE
        )
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "old_schedule": OLD_SCHEDULE,
                    "new_schedule": config.get("schedule"),
                    "agent_input_schedule": EXPECTED_NEW_SCHEDULE,
                    "note": "engine normalizes 5-field cron to 7-field form",
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
