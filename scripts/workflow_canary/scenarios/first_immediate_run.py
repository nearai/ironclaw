"""First-immediate-run probe — covers Script 3 PHASE 2.1:
"Run the first check immediately."

When a routine is created with ``fire_immediately=True`` (the
``next_fire_at`` is set to "now" instead of waiting for the next cron
tick), the engine should pick it up on the very next cron tick (≤2s
under the canary's ROUTINES_CRON_INTERVAL=2). This probe asserts a
routine inserted with fire_immediately=True reaches terminal status
within 10s — a hard upper bound that catches "first check is delayed
to the next hour".
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
    mode = "first_immediate_run"

    try:
        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-first-immediate-run",
            prompt=(
                "Send a Telegram acknowledgement that the first check "
                "fired immediately.\n\n"
                "[CANARY-WORKFLOW-first_immediate_run]"
            ),
            schedule="0 * * * *",  # would otherwise wait an hour
            description="canary: first immediate run",
            fire_immediately=True,
        )

        # Hard upper bound: 10s. A "first check delayed to next cron"
        # regression on a "0 * * * *" schedule would push the fire to the
        # top of the next hour — this assertion catches that shape.
        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=10.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status": last_run["status"],
                    "run_count": len(runs),
                    # Latency captures the fire-immediately path under
                    # ROUTINES_CRON_INTERVAL=2 — < 4000 ms is the
                    # expected band; anything larger means the engine
                    # is doing something the canary doesn't expect.
                    "fire_latency_ms": latency_ms,
                },
            )
        ]
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        observed = (
            list_routine_runs(stack.db_path, locals().get("routine_id", ""))
            if "routine_id" in locals()
            else []
        )
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=latency_ms,
                details={
                    "error": f"timeout: {exc}",
                    "observed_runs": len(observed),
                    "observed_statuses": [r["status"] for r in observed],
                },
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
