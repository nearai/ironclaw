"""Cross-fire dedup probe — covers issue #1044's "no duplicate"
assertions across Scripts 1, 3, 5.

Original NL surface:
- Script 1 PHASE 4.4: "Existing rows are not duplicated or overwritten."
- Script 3 PHASE 3.2: "Only posts not already reported in the previous
  run are included (no duplicate alerts for the same post)."
- Script 5 PHASE 5.5: "Verify no duplicates on second run."

User-script dedup is application-level (the agent decides not to
re-process an item it already saw) and lives outside the canary's
deterministic-mock surface. The closest engine-level mechanism we
CAN test deterministically is **routine cooldown_secs** — the
engine refuses to fire a routine within `cooldown_secs` of its
previous fire. This is the same suppression that protects Script 1's
"every 2 minutes" cron from doubling up if a fire takes longer than
the interval.

Coverage:
- Insert a routine with cooldown_secs=30 and backdate next_fire_at.
- Wait for the first fire → assert routine_runs has exactly 1 row
  in terminal status.
- Backdate again immediately → wait again.
- Assert there's still exactly 1 row (cooldown suppressed the second).
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
    db_path = stack.db_path
    mode = "dedup_cooldown"

    try:
        routine_id = insert_lightweight_cron_routine(
            db_path,
            user_id="workflow-canary-owner",
            name="canary-dedup-cooldown",
            prompt=f"hi - dedup probe\n\n[CANARY-WORKFLOW-{mode}]",
            description="canary: cooldown suppresses back-to-back fires",
            fire_immediately=True,
            cooldown_secs=30,
        )

        # First fire — should land within ~5 s of cron tick.
        runs = await wait_for_run(
            db_path, routine_id, min_runs=1, timeout_secs=30.0
        )
        first_run = runs[0]
        if first_run["status"] not in SUCCESS_RUN_STATUSES:
            raise RuntimeError(
                f"first fire didn't reach success: {first_run['status']}"
            )

        # Immediately backdate again. Without cooldown this would fire
        # on the next cron tick. With cooldown_secs=30 the engine should
        # refuse (since the first run completed <30 s ago).
        backdate_routine(db_path, routine_id, seconds_ago=60)

        # Wait long enough that a second fire would have been observable
        # if cooldown weren't honored. With ROUTINES_CRON_INTERVAL=2 s,
        # 8 s is 4 ticks — plenty of opportunity.
        await asyncio.sleep(8.0)

        runs_after = list_routine_runs(db_path, routine_id)
        latency_ms = int((time.perf_counter() - started) * 1000)
        success = len(runs_after) == 1
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "cooldown_secs": 30,
                    "observed_runs": len(runs_after),
                    "expected_runs": 1,
                    "first_run_status": first_run["status"],
                },
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
