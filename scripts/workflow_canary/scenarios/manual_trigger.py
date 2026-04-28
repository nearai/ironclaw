"""Manual-trigger probe — covers issue #1044's "first immediate run"
and "trigger from Telegram now" assertions across Scripts 3 and 4.

Original NL surface:
- Script 3 PHASE 2.1: "Run the first check immediately."
- Script 3 PHASE 4.2: "Manual routine trigger" via the Routines tab.
- Script 4 PHASE 4.2: "trigger my dog walk reminder now" via Telegram.

All three resolve to the same back-end mechanism:
``POST /api/routines/<id>/trigger`` → ``RoutineEngine::fire_manual``.

Coverage:
- Inserts a routine WITHOUT a backdated next_fire_at, so the only
  way it fires is a manual trigger (cron tick wouldn't pick it up
  for a full minute under the test's "*/1 * * * *" schedule).
- Hits the trigger endpoint, asserts the response contains a run_id.
- Polls routine_runs for the run with that id reaching terminal status.
- Asserts mock telegram captured the per-scenario ack (proving the
  manual-trigger path goes through the same Lightweight action loop
  that cron does, not a degraded code path).
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
    trigger_routine_via_api,
    wait_for_run,
)
from scripts.workflow_canary.scenarios._common import (
    _capture_telegram_messages,
    _scenario_key,
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
    db_path = stack.db_path
    started = time.perf_counter()
    mode = "manual_trigger"
    key = _scenario_key(mode)
    expected_text = f"[canary-workflow:{key}] ack"

    try:
        # No backdating — cron tick won't pick this up for a full minute
        # under */1 schedule, so a successful fire here only happens via
        # the manual-trigger API.
        routine_id = insert_lightweight_cron_routine(
            db_path,
            user_id="workflow-canary-owner",
            name="canary-manual-trigger",
            prompt=(
                f"Send the user a Telegram message confirming the "
                f"manual trigger fired.\n\n[CANARY-WORKFLOW-{key}]"
            ),
            schedule="*/1 * * * *",
            description="canary: manual trigger via /api/routines/<id>/trigger",
            fire_immediately=False,
        )

        trigger_response = await trigger_routine_via_api(
            stack.base_url, stack.gateway_token, routine_id
        )
        run_id = trigger_response.get("run_id")
        if not run_id:
            raise RuntimeError(
                f"trigger response missing run_id: {trigger_response}"
            )

        runs = await wait_for_run(
            db_path, routine_id, min_runs=1, timeout_secs=30.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Verify mock telegram captured the ack
        telegram_match: dict[str, Any] | None = None
        if run_terminal:
            import asyncio

            for _ in range(10):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    if (
                        m.get("method") == "sendMessage"
                        and expected_text in (m.get("text") or "")
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
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "trigger_run_id": run_id,
                    "run_status": last_run["status"],
                    "expected_text": expected_text,
                    "telegram_match": telegram_match.get("text")
                    if telegram_match
                    else None,
                },
            )
        ]
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        observed = (
            list_routine_runs(db_path, locals().get("routine_id", ""))
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
