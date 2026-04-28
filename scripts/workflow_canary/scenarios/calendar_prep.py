"""Script 2 — Calendar Prep Assistant (issue #1044), Phase 2.

Original goal: 10 min before each Google Calendar meeting, send a
Telegram message summarizing company background + recent news for
external attendees.

Phase 2 coverage (this scenario):

1. Seed mock_calendar with one upcoming canary event whose summary
   is "Canary kickoff with Acme".
2. Insert a Lightweight cron routine whose prompt carries the
   ``[CANARY-WORKFLOW-CAL-LIST]`` sentinel.
3. Mock LLM emits an http GET to
   ``www.googleapis.com/calendar/v3/.../primary/events`` →
   ``IRONCLAW_TEST_HTTP_REMAP`` routes to ``calendar_mock`` →
   the seeded event comes back in the response.
4. Mock LLM second-iteration matcher (matched on the event title
   appearing in "Tool `http` returned: ...") emits a Telegram
   sendMessage referencing the meeting + a summary line.
5. Assert (a) calendar_mock /__mock/captured saw the events.list call,
   (b) telegram_mock received the prep message, (c) the Telegram
   message text references the seeded event title (catches a
   regression where the lightweight action sends a generic message
   that doesn't reflect actual calendar contents).
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

EVENT_SUMMARY = "Canary kickoff with Acme"


async def _seed_calendar(mock_calendar_url: str) -> None:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(
            f"{mock_calendar_url}/__mock/seed_events",
            json={
                "calendar_id": "primary",
                "events": [
                    {
                        "id": "canary-event-acme",
                        "summary": EVENT_SUMMARY,
                        "description": "Internal sync with Acme Corp.",
                        "start": {"dateTime": "2026-04-28T15:00:00Z"},
                        "end": {"dateTime": "2026-04-28T16:00:00Z"},
                        "attendees": [
                            {"email": "host@example.com"},
                            {"email": "ceo@acme.example"},
                        ],
                    }
                ],
            },
        )
        response.raise_for_status()


async def _captured(mock_url: str) -> list[dict[str, Any]]:
    """Generic /__mock/captured drainer — works for any of our mocks."""
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_url}/__mock/captured")
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
    mode = "calendar_prep"
    expected_text_fragment = EVENT_SUMMARY

    if mock_calendar_url is None:
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "mock_calendar_url not provided"},
            )
        ]

    try:
        await _seed_calendar(mock_calendar_url)

        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-calendar-prep",
            prompt=(
                "List upcoming meetings on the primary calendar and "
                "send a Telegram briefing summarizing the next "
                "external meeting.\n\n[CANARY-WORKFLOW-CAL-LIST]"
            ),
            description="canary script 2: calendar prep -> telegram",
            fire_immediately=True,
        )

        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=60.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Verify calendar mock saw the events.list call
        calendar_captured = await _captured(mock_calendar_url)
        events_list_seen = any(
            entry.get("method") == "GET"
            and "/calendar/v3/calendars/primary/events" in entry.get("path", "")
            for entry in calendar_captured
        )

        # Verify web_search mock saw the company lookup (covers the
        # "company background + recent news" assertion from Script 2).
        web_search_seen = False
        if mock_web_search_url:
            web_search_captured = await _captured(mock_web_search_url)
            web_search_seen = any(
                entry.get("method") == "GET"
                and "/res/v1/web/search" in entry.get("path", "")
                and "acme" in (entry.get("query", {}).get("q", "") or "").lower()
                for entry in web_search_captured
            )

        # Verify telegram captured the prep message referencing the event
        telegram_match: dict[str, Any] | None = None
        if run_terminal:
            for _ in range(20):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    text = m.get("text") or ""
                    if (
                        m.get("method") == "sendMessage"
                        and "[canary-workflow:calendar_prep]" in text
                        and expected_text_fragment in text
                    ):
                        telegram_match = m
                        break
                if telegram_match is not None:
                    break
                await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            run_terminal
            and events_list_seen
            and web_search_seen
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
                    "events_list_seen": events_list_seen,
                    "web_search_seen": web_search_seen,
                    "calendar_captured_count": len(calendar_captured),
                    "telegram_text": (
                        telegram_match.get("text") if telegram_match else None
                    ),
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
