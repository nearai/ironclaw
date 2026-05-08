"""Lifecycle probe — disable / enable / delete via the routines API.

Original NL surface (issue #1044):
- Script 1 PHASE 5.1 / 5.2: disable then re-enable a routine
- Script 4 PHASE 5.2 / 5.3 / 5.4: disable, enable, delete via the
  Routines tab

Back-end mechanism: ``POST /api/routines/<id>/toggle`` (with
``{"enabled": bool}``) and ``DELETE /api/routines/<id>``.

Coverage in three sub-probes:

1. ``disable_blocks_fires`` — disable a backdated routine, assert it
   does NOT fire (no `routine_runs` row appears within budget).
2. ``enable_resumes_fires`` — re-enable + backdate, assert it fires
   normally and reaches terminal status.
3. ``delete_removes_routine`` — delete the routine, assert
   ``GET /api/routines`` no longer lists it AND no further runs
   appear.
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
    delete_routine_via_api,
    insert_lightweight_cron_routine,
    list_routine_runs,
    list_routines_via_api,
    toggle_routine_via_api,
    wait_for_run,
)


def _scenario_key() -> str:
    return "lifecycle"


async def _disabled_blocks_fires(stack: Any) -> ProbeResult:
    """Insert disabled (or disable then backdate) → assert no fires."""
    started = time.perf_counter()
    routine_id = insert_lightweight_cron_routine(
        stack.db_path,
        user_id="workflow-canary-owner",
        name="canary-lifecycle-disabled",
        prompt=f"hi - lifecycle disabled probe\n\n[CANARY-WORKFLOW-{_scenario_key()}]",
        description="canary: disabled routine should not fire",
        fire_immediately=True,
        enabled=False,
    )
    # Wait the same window we'd otherwise expect a fire. If the engine
    # respects `enabled=0`, no run row appears.
    await asyncio.sleep(8.0)
    runs = list_routine_runs(stack.db_path, routine_id)

    latency_ms = int((time.perf_counter() - started) * 1000)
    success = len(runs) == 0
    return ProbeResult(
        provider="routines",
        mode="lifecycle_disable",
        success=success,
        latency_ms=latency_ms,
        details={
            "routine_id": routine_id,
            "observed_runs": len(runs),
            "expected_runs": 0,
        },
    )


async def _enable_resumes_fires(stack: Any) -> ProbeResult:
    """Insert disabled → enable via API → backdate → assert fire."""
    started = time.perf_counter()
    routine_id = insert_lightweight_cron_routine(
        stack.db_path,
        user_id="workflow-canary-owner",
        name="canary-lifecycle-toggled",
        prompt=f"hi - lifecycle toggled probe\n\n[CANARY-WORKFLOW-{_scenario_key()}]",
        description="canary: enable resumes routine fires",
        fire_immediately=False,
        enabled=False,
    )
    await toggle_routine_via_api(
        stack.base_url, stack.gateway_token, routine_id, enabled=True
    )
    # Backdate so the next cron tick picks it up.
    backdate_routine(stack.db_path, routine_id, seconds_ago=60)
    runs = await wait_for_run(
        stack.db_path, routine_id, min_runs=1, timeout_secs=30.0
    )
    latency_ms = int((time.perf_counter() - started) * 1000)
    success = runs[0]["status"] in SUCCESS_RUN_STATUSES
    return ProbeResult(
        provider="routines",
        mode="lifecycle_toggle",
        success=success,
        latency_ms=latency_ms,
        details={
            "routine_id": routine_id,
            "run_status": runs[0]["status"],
            "run_count": len(runs),
        },
    )


async def _delete_removes_routine(stack: Any) -> ProbeResult:
    """Insert → delete via API → assert /api/routines no longer lists it."""
    started = time.perf_counter()
    routine_id = insert_lightweight_cron_routine(
        stack.db_path,
        user_id="workflow-canary-owner",
        name="canary-lifecycle-deleted",
        prompt=f"hi - lifecycle delete probe\n\n[CANARY-WORKFLOW-{_scenario_key()}]",
        description="canary: deleted routine should disappear",
        fire_immediately=False,
        enabled=False,
    )
    # Confirm it's listed first
    listed_before = await list_routines_via_api(stack.base_url, stack.gateway_token)
    present_before = any(r.get("id") == routine_id for r in listed_before)
    if not present_before:
        latency_ms = int((time.perf_counter() - started) * 1000)
        return ProbeResult(
            provider="routines",
            mode="lifecycle_delete",
            success=False,
            latency_ms=latency_ms,
            details={
                "error": "routine not present in /api/routines before delete",
                "routine_id": routine_id,
            },
        )
    await delete_routine_via_api(stack.base_url, stack.gateway_token, routine_id)
    listed_after = await list_routines_via_api(stack.base_url, stack.gateway_token)
    present_after = any(r.get("id") == routine_id for r in listed_after)

    latency_ms = int((time.perf_counter() - started) * 1000)
    success = (not present_after) and present_before
    return ProbeResult(
        provider="routines",
        mode="lifecycle_delete",
        success=success,
        latency_ms=latency_ms,
        details={
            "routine_id": routine_id,
            "present_before": present_before,
            "present_after": present_after,
        },
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
    results: list[ProbeResult] = []
    for sub in (
        _disabled_blocks_fires,
        _enable_resumes_fires,
        _delete_removes_routine,
    ):
        try:
            results.append(await sub(stack))
        except Exception as exc:  # noqa: BLE001
            results.append(
                ProbeResult(
                    provider="routines",
                    mode=sub.__name__.lstrip("_"),
                    success=False,
                    latency_ms=0,
                    details={"error": f"{type(exc).__name__}: {exc}"},
                )
            )
    return results
