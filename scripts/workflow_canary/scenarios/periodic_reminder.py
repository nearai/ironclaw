"""Script 4 — Periodic Reminder via Telegram (issue #1044), Phase 1A.

Verifies the routine engine picks up a backdated cron routine, fires
its Lightweight action against the (mock) LLM, and records a
``routine_runs`` row with terminal status.

Coverage gained vs. existing canaries:

- Routine engine cron-tick path (`spawn_cron_ticker` → `check_cron_triggers`)
- Lightweight routine action execution (RoutineAction::Lightweight)
- DB-backed routine state machine (`routines.next_fire_at` →
  `routine_runs.status`)

Coverage deliberately *not* included in this phase:

- Telegram channel install + bot-token setup (requires admin secrets
  API; deferred to Phase 1B once we either (a) wire admin auth into
  the canary stack or (b) write directly to the encrypted secrets
  table). The mock Telegram server is started and the
  `IRONCLAW_TEST_HTTP_REMAP=api.telegram.org=...` is wired so the
  next iteration only has to add the channel install + assertion.
- Verifying the routine's prompt actually causes a Telegram
  sendMessage. Requires the channel install above.

Phase 1A intentionally proves the engine + mock LLM scaffolding work
end-to-end. Phase 1B layers on the Telegram side-effect verification.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.routines import (
    SUCCESS_RUN_STATUSES,
    insert_lightweight_cron_routine,
    list_routine_runs,
    wait_for_run,
)

# The mock LLM matches `\bhello\b|\bhi\b|\bhey\b` and returns
# "Hello! How can I help you today?" — see tests/e2e/mock_llm.py
# CANNED_RESPONSES line 19. Keep the prompt simple so the engine
# completes one round-trip quickly without needing tool calls.
REMINDER_PROMPT = "hi"
ROUTINE_NAME = "canary-periodic-reminder"


async def _drain_mock_telegram(mock_telegram_url: str) -> list[dict[str, Any]]:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_telegram_url}/__mock/sent_messages")
        response.raise_for_status()
        return response.json().get("messages", [])


async def _reset_mock_telegram(mock_telegram_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(f"{mock_telegram_url}/__mock/reset")
        response.raise_for_status()


async def run(
    *,
    stack: Any,
    mock_telegram_url: str,
    output_dir: Path,
    log_dir: Path,
) -> list[ProbeResult]:
    db_path = stack.db_path
    owner_user_id = "workflow-canary-owner"

    results: list[ProbeResult] = []
    started = time.perf_counter()

    try:
        await _reset_mock_telegram(mock_telegram_url)

        routine_id = insert_lightweight_cron_routine(
            db_path,
            user_id=owner_user_id,
            name=ROUTINE_NAME,
            prompt=REMINDER_PROMPT,
            schedule="*/1 * * * *",
            description="canary: dog walk reminder",
            fire_immediately=True,
        )
        print(
            f"[periodic_reminder] inserted routine {routine_id}, "
            f"next_fire_at backdated 60 s",
            flush=True,
        )

        # Cron tick is configured to 2 s in run_workflow_canary.py, plus
        # the routine's lightweight action takes a few seconds end-to-end.
        # 30 s is comfortably above the 75th percentile observed in local
        # iteration; 60 s gives headroom for a slow CI runner.
        runs = await wait_for_run(
            db_path, routine_id, min_runs=1, timeout_secs=60.0
        )
        last_run = runs[0]
        latency_ms = int((time.perf_counter() - started) * 1000)
        print(
            f"[periodic_reminder] routine fired: status={last_run['status']}, "
            f"completed_at={last_run['completed_at']}",
            flush=True,
        )

        success = last_run["status"] in SUCCESS_RUN_STATUSES

        results.append(
            ProbeResult(
                provider="routines",
                mode="cron_lightweight_routine",
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status": last_run["status"],
                    "run_count": len(runs),
                    "result_summary": last_run.get("result_summary"),
                },
            )
        )
    except TimeoutError as exc:
        latency_ms = int((time.perf_counter() - started) * 1000)
        observed = (
            list_routine_runs(db_path, locals().get("routine_id", ""))
            if "routine_id" in locals()
            else []
        )
        results.append(
            ProbeResult(
                provider="routines",
                mode="cron_lightweight_routine",
                success=False,
                latency_ms=latency_ms,
                details={
                    "error": f"timeout: {exc}",
                    "observed_runs": len(observed),
                    "observed_statuses": [r["status"] for r in observed],
                },
            )
        )
    except Exception as exc:  # noqa: BLE001
        latency_ms = int((time.perf_counter() - started) * 1000)
        results.append(
            ProbeResult(
                provider="routines",
                mode="cron_lightweight_routine",
                success=False,
                latency_ms=latency_ms,
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        )

    return results
