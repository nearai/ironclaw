"""Script 5 — Email → CRM Inbound Tracker (issue #1044), Phase 4.

Original goal: hourly cron reads Gmail inbox, the LLM classifies
inbound sales leads, each lead is appended to a Google Sheet
"Inbound CRM" with structured columns (Company, Contact Name, Email,
Status, Notes, Next Action). Optional Telegram summary on each fire.

Phase 4 coverage (this scenario):

1. Pre-seed mock_gmail with 3 messages (1 lead, 1 newsletter, 1
   receipt) and pre-seed mock_sheets with the CRM spreadsheet +
   header row.
2. Insert a Lightweight cron routine whose prompt carries the
   ``[CANARY-WORKFLOW-CRM-CLASSIFY]`` sentinel.
3. Mock LLM emits a parallel triplet:
   - http GET to gmail.googleapis.com (assertion: gmail mock saw it)
   - http POST to sheets.googleapis.com values:append for the lead
     ROW only (with all 6 expected columns)
   - http POST to api.telegram.org sendMessage with the ack count
4. Assert (a) gmail mock saw the messages list call, (b) sheet has
   exactly ONE appended row beyond the header (no newsletter, no
   receipt), (c) the appended row has all 6 columns populated.
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
from scripts.workflow_canary.scenarios._common import _capture_telegram_messages

CRM_SPREADSHEET_ID = "canary-crm-tracker"
CRM_HEADERS = [
    "Company",
    "Contact Name",
    "Email",
    "Status",
    "Notes",
    "Next Action",
]


async def _seed_gmail(mock_gmail_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(
            f"{mock_gmail_url}/__mock/seed_messages",
            json={
                "messages": [
                    {
                        "id": "msg-canary-lead",
                        "subject": "Interested in your enterprise tier",
                        "from": "Jane Lead <jane.lead@acme.example>",
                        "snippet": (
                            "Hi — Acme Corp is evaluating vendors for Q2 "
                            "and we want to discuss your enterprise tier."
                        ),
                    },
                    {
                        "id": "msg-canary-newsletter",
                        "subject": "Weekly digest",
                        "from": "newsletter@example.com",
                        "snippet": "This week in tech: nothing actionable.",
                    },
                    {
                        "id": "msg-canary-receipt",
                        "subject": "Your receipt from Coffee Shop",
                        "from": "no-reply@coffee.example",
                        "snippet": "Thank you for your purchase: $4.50.",
                    },
                ]
            },
        )
        response.raise_for_status()


async def _seed_crm_sheet(mock_sheets_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(
            f"{mock_sheets_url}/__mock/seed_spreadsheet",
            json={
                "spreadsheet_id": CRM_SPREADSHEET_ID,
                "title": "Canary inbound CRM",
                "headers": CRM_HEADERS,
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


async def _gmail_captured(mock_gmail_url: str) -> list[dict[str, Any]]:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_gmail_url}/__mock/captured")
        response.raise_for_status()
        return response.json().get("captured", [])


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
    mode = "crm_tracker"

    if mock_sheets_url is None or mock_gmail_url is None:
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "mock_sheets_url / mock_gmail_url required"},
            )
        ]

    try:
        await _seed_gmail(mock_gmail_url)
        await _seed_crm_sheet(mock_sheets_url)

        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-crm-tracker",
            prompt=(
                "Scan recent Gmail messages for inbound sales leads, "
                "classify each into Company / Contact Name / Email / "
                "Status / Notes / Next Action, append rows to the "
                "Inbound CRM sheet, and send a Telegram summary.\n\n"
                "[CANARY-WORKFLOW-CRM-CLASSIFY]"
            ),
            description="canary script 5: gmail -> sheets CRM",
            fire_immediately=True,
        )

        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=60.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # gmail mock saw the list call
        gmail_captured = await _gmail_captured(mock_gmail_url)
        gmail_list_seen = any(
            entry.get("method") == "GET"
            and "/gmail/v1/users/me/messages" in entry.get("path", "")
            for entry in gmail_captured
        )

        # Sheet has the lead row (and only the lead row)
        rows: list[list[str]] = []
        appended: list[list[str]] = []
        if run_terminal:
            for _ in range(20):
                rows = await _read_sheet_rows(mock_sheets_url, CRM_SPREADSHEET_ID)
                appended = rows[1:]  # skip header
                if appended:
                    break
                await asyncio.sleep(0.5)

        # Telegram captured the ack (referencing 1 lead)
        telegram_match: dict[str, Any] | None = None
        if run_terminal:
            for _ in range(20):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    text = m.get("text") or ""
                    if (
                        m.get("method") == "sendMessage"
                        and "[canary-workflow:crm_tracker]" in text
                        and "1 new lead" in text
                    ):
                        telegram_match = m
                        break
                if telegram_match is not None:
                    break
                await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            run_terminal
            and gmail_list_seen
            and len(appended) == 1
            and len(appended[0]) == len(CRM_HEADERS)
            and telegram_match is not None
        )

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status": last_run["status"],
                    "gmail_list_seen": gmail_list_seen,
                    "appended_row_count": len(appended),
                    "appended_row": appended[0] if appended else None,
                    "telegram_text": (
                        telegram_match.get("text") if telegram_match else None
                    ),
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
