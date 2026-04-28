"""Shared scenario plumbing for workflow-canary scripts.

All five scenarios from issue #1044 share the same Phase 1A shape:
insert a Lightweight cron routine with a script-specific prompt,
backdate ``next_fire_at`` so the engine fires it on the next tick,
poll ``routine_runs`` for terminal status, surface a ``ProbeResult``.

This module factors out that shape into ``run_routine_probe`` so each
scenario file can stay narrow — just the prompt + assertions specific
to that scenario. Phase 1B adds per-scenario side-effect verification
(mock Telegram sendMessage, mock Sheets values:append, mock HN scrape)
on top of this base — those layer in by extending ``ProbeResult.details``,
not by reimplementing the routine plumbing.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    insert_lightweight_cron_routine,
    list_routine_runs,
    wait_for_run,
)


async def run_routine_probe(
    *,
    stack: Any,
    provider: str,
    mode: str,
    routine_name: str,
    prompt: str,
    description: str = "",
    schedule: str = "*/1 * * * *",
    timeout_secs: float = 60.0,
    extra_details: dict[str, Any] | None = None,
) -> ProbeResult:
    """Insert a lightweight cron routine, fire it, return the probe result.

    Caller supplies:
      - ``provider`` / ``mode`` — labels surfaced in results.json (the
        same axes used by every other canary lane)
      - ``routine_name`` — DB unique constraint key (must be unique
        per probe within one stack)
      - ``prompt`` — the lightweight action text the routine fires; the
        mock LLM responds based on its CANNED_RESPONSES patterns
      - ``timeout_secs`` — total budget for engine cron tick + LLM
        round-trip; default 60 s gives ~3.5x headroom on a 2 s tick
    """
    db_path = stack.db_path
    owner_user_id = "workflow-canary-owner"
    started = time.perf_counter()
    extra = extra_details or {}

    try:
        routine_id = insert_lightweight_cron_routine(
            db_path,
            user_id=owner_user_id,
            name=routine_name,
            prompt=prompt,
            schedule=schedule,
            description=description,
            fire_immediately=True,
        )

        runs = await wait_for_run(
            db_path, routine_id, min_runs=1, timeout_secs=timeout_secs
        )
        last_run = runs[0]
        latency_ms = int((time.perf_counter() - started) * 1000)
        success = last_run["status"] in SUCCESS_RUN_STATUSES

        details = {
            "routine_id": routine_id,
            "run_status": last_run["status"],
            "run_count": len(runs),
            "result_summary": last_run.get("result_summary"),
            **extra,
        }
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=success,
            latency_ms=latency_ms,
            details=details,
        )
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        observed = (
            list_routine_runs(db_path, locals().get("routine_id", ""))
            if "routine_id" in locals()
            else []
        )
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=False,
            latency_ms=latency_ms,
            details={
                "error": f"timeout: {exc}",
                "observed_runs": len(observed),
                "observed_statuses": [r["status"] for r in observed],
                **extra,
            },
        )
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        return ProbeResult(
            provider=provider,
            mode=mode,
            success=False,
            latency_ms=latency_ms,
            details={
                "error": f"{type(exc).__name__}: {exc}",
                **extra,
            },
        )
