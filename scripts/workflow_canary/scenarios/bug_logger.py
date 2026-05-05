"""Script 1 — Telegram → Google Sheet Bug Logger (issue #1044), Phase 1.

Original goal: Telegram messages starting with "bug:" get appended to
a Google Sheet via a 2-minute cron routine.

Phase 1 coverage (this scenario):

1. Pre-seed mock Google Sheets with a spreadsheet whose id is
   ``canary-bug-logger`` and a header row.
2. Insert a Lightweight cron routine whose prompt carries the
   ``[CANARY-WORKFLOW-SHEET-APPEND]`` sentinel.
3. The mock LLM matches that sentinel and emits a deterministic
   ``http`` POST to ``sheets.googleapis.com/v4/spreadsheets/.../
   values/Sheet1:append``. ``IRONCLAW_TEST_HTTP_REMAP`` (set in
   ``run_workflow_canary.py``) routes the call to ``sheets_mock``.
4. Poll mock_sheets ``/__mock/spreadsheets`` until the canary row
   appears. Assert the shape: timestamp / message / source.

Catches the canonical ``"expected a sequence"`` regression — the mock
rejects any payload whose ``values`` is not a list-of-lists, with the
same error shape Google's real API returns. If a future code change
sends a string instead of a sequence, ``values_append`` returns 400
and the lightweight tool execution surfaces the error.
"""

from __future__ import annotations

import asyncio
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

SPREADSHEET_ID = "canary-bug-logger"
SHEET_HEADERS = ["timestamp", "message", "source"]


async def _seed_spreadsheet(mock_sheets_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(
            f"{mock_sheets_url}/__mock/seed_spreadsheet",
            json={
                "spreadsheet_id": SPREADSHEET_ID,
                "title": "Canary bug tracker",
                "headers": SHEET_HEADERS,
            },
        )
        response.raise_for_status()


async def _read_sheet_rows(
    mock_sheets_url: str, spreadsheet_id: str
) -> list[list[str]]:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_sheets_url}/__mock/spreadsheets")
        response.raise_for_status()
        payload = response.json()
    for s in payload.get("spreadsheets", []):
        if s.get("spreadsheetId") == spreadsheet_id:
            return list(s.get("rows", []))
    return []


async def _wait_for_appended_row(
    mock_sheets_url: str,
    spreadsheet_id: str,
    *,
    timeout_secs: float = 30.0,
) -> list[str] | None:
    """Wait for a non-header row to appear in the seeded spreadsheet.

    The seed installs the header row; any row beyond index 0 is the
    canary's appended payload."""
    deadline = time.monotonic() + timeout_secs
    while time.monotonic() < deadline:
        rows = await _read_sheet_rows(mock_sheets_url, spreadsheet_id)
        if len(rows) > 1:
            return rows[-1]
        await asyncio.sleep(0.5)
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
    mode = "bug_logger"

    if mock_sheets_url is None:
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "mock_sheets_url not provided"},
            )
        ]

    try:
        await _seed_spreadsheet(mock_sheets_url)

        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-bug-logger",
            prompt=(
                "Scan recent Telegram messages for entries starting "
                "with 'bug:' and append each to the bug-tracking "
                "sheet.\n\n[CANARY-WORKFLOW-SHEET-APPEND]"
            ),
            description="canary script 1: telegram bugs -> sheet",
            fire_immediately=True,
        )

        # Wait for the lightweight action to finish.
        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=60.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Even if the routine reaches terminal status, we want to assert
        # the http call landed at mock_sheets — that's the actual
        # regression surface this scenario covers.
        appended_row: list[str] | None = None
        if run_terminal:
            appended_row = await _wait_for_appended_row(
                mock_sheets_url, SPREADSHEET_ID, timeout_secs=15.0
            )

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and appended_row is not None and len(appended_row) >= 3

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status": last_run["status"],
                    "spreadsheet_id": SPREADSHEET_ID,
                    "appended_row": appended_row,
                    "result_summary": last_run.get("result_summary"),
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
