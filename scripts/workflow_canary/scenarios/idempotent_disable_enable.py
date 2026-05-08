"""Idempotent disable/enable probe — covers Script 1 PHASE 5.1 / 5.2
"disable then enable should resume cleanly".

Coverage:
1. Insert a routine that's enabled but not backdated (so it doesn't
   immediately fire).
2. Toggle disable → toggle disable again (idempotent — should not
   error).
3. Toggle enable → toggle enable again (idempotent).
4. Backdate next_fire_at and assert the routine fires.
5. Toggle disable while still in enabled state, snapshot the run
   count, sleep 4s, assert no NEW runs appear.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    backdate_routine,
    insert_lightweight_cron_routine,
    list_routine_runs,
    toggle_routine_via_api,
    wait_for_run,
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
    mode = "idempotent_disable_enable"

    try:
        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-idempotent-toggle",
            prompt=(
                "Send a Telegram ack — idempotent toggle probe.\n\n"
                "[CANARY-WORKFLOW-idempotent_disable_enable]"
            ),
            description="canary: idempotent disable/enable",
            fire_immediately=False,
            enabled=True,
        )

        # Idempotent disable
        await toggle_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id, enabled=False
        )
        await toggle_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id, enabled=False
        )
        # Idempotent enable
        await toggle_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id, enabled=True
        )
        await toggle_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id, enabled=True
        )

        # Now backdate so the engine fires on next tick
        backdate_routine(stack.db_path, routine_id, seconds_ago=60)
        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=30.0
        )
        run_terminal = runs[0]["status"] in SUCCESS_RUN_STATUSES

        # Disable → wait → assert no new fires appear
        await toggle_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id, enabled=False
        )
        backdate_routine(stack.db_path, routine_id, seconds_ago=60)
        snapshot = len(runs)
        await asyncio.sleep(6.0)
        post_runs = list_routine_runs(stack.db_path, routine_id)
        no_new_fires_after_disable = len(post_runs) == snapshot

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and no_new_fires_after_disable

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status_after_enable": runs[0]["status"],
                    "runs_after_enable": snapshot,
                    "runs_after_disable": len(post_runs),
                    "no_new_fires_after_disable": no_new_fires_after_disable,
                },
            )
        ]
    except TimeoutError as exc:
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"timeout: {exc}"},
            )
        ]
    except Exception as exc:  # noqa: BLE001
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        ]
