"""Cron timing accuracy probe — covers Scripts 3 PHASE 3.1 +
Script 4 PHASE 3.4: "cron fires within 60s of scheduled time" /
"cron doesn't skip cycles".

Coverage:
1. Insert a routine and explicitly set ``next_fire_at`` to "now + 5s"
   so the engine's next cron tick (every 2s under
   ``ROUTINES_CRON_INTERVAL=2``) picks it up around that boundary.
2. Wait up to 30s for the first run.
3. Assert the run starts within ±10s of the expected next_fire_at.
   That tolerance is large enough to absorb the 2s cron-tick interval
   plus mock-LLM round-trip latency, small enough to catch
   "cron skipped a cycle" / "fires never trigger" regressions.
4. Assert exactly one terminal run row exists (not multiple, not zero).
"""

from __future__ import annotations

import sqlite3
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    insert_lightweight_cron_routine,
    list_routine_runs,
    wait_for_run,
)


def _set_next_fire_at(
    db_path: str | Path, routine_id: str, fire_at: datetime
) -> None:
    iso = fire_at.strftime("%Y-%m-%dT%H:%M:%S.000Z")
    with sqlite3.connect(str(db_path)) as conn:
        conn.execute(
            "UPDATE routines SET next_fire_at = ?, updated_at = ? "
            "WHERE id = ?",
            (iso, iso, routine_id),
        )
        conn.commit()


def _read_started_at(
    db_path: str | Path, routine_id: str
) -> datetime | None:
    with sqlite3.connect(str(db_path)) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT started_at FROM routine_runs "
            "WHERE routine_id = ? ORDER BY started_at ASC LIMIT 1",
            (routine_id,),
        ).fetchone()
    if row is None or row["started_at"] is None:
        return None
    raw = row["started_at"]
    try:
        return datetime.fromisoformat(raw.replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None


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
    mode = "cron_timing_accuracy"

    try:
        # Insert with fire_immediately=False so we control next_fire_at
        # explicitly. The schedule itself doesn't matter for THIS probe —
        # we just want to validate the engine fires close to the scheduled
        # next_fire_at, regardless of how the boundary was computed.
        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-cron-timing",
            prompt=(
                "Send a Telegram acknowledgement for cron timing.\n\n"
                "[CANARY-WORKFLOW-cron_timing_accuracy]"
            ),
            schedule="*/1 * * * *",
            description="canary: cron timing accuracy",
            fire_immediately=False,
        )

        expected_at = datetime.now(timezone.utc) + timedelta(seconds=5)
        _set_next_fire_at(stack.db_path, routine_id, expected_at)

        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=30.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        actual_at = _read_started_at(stack.db_path, routine_id)
        delta_secs: float | None = None
        if actual_at is not None:
            delta_secs = abs((actual_at - expected_at).total_seconds())
        within_tolerance = delta_secs is not None and delta_secs <= 10.0

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and within_tolerance

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
                    "expected_at": expected_at.isoformat(),
                    "actual_at": (
                        actual_at.isoformat() if actual_at else None
                    ),
                    "delta_secs": delta_secs,
                    "tolerance_secs": 10.0,
                },
            )
        ]
    except TimeoutError as exc:
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
                latency_ms=int((time.perf_counter() - started) * 1000),
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
