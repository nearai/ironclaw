"""Script 3 — Hacker News Keyword Monitor (issue #1044), Phase 3.

Original goal: hourly cron checks Hacker News for "Show HN" posts,
sends formatted summaries to Telegram. The script also covers the
"first immediate run" semantics — the routine should fire once at
creation time, not just on the next cron tick.

Phase 3 coverage (this scenario):

1. Verify ``hn_mock`` defaults are seeded (the mock state ships two
   "Show HN" posts on startup; we re-set defaults with a deterministic
   seed call to insulate against probe ordering).
2. Insert a Lightweight cron routine whose prompt carries
   ``[CANARY-WORKFLOW-HN-FETCH]``.
3. Mock LLM emits a parallel triplet (well, pair): http GET /newest
   + http POST sendMessage. ``IRONCLAW_TEST_HTTP_REMAP`` routes
   ``news.ycombinator.com`` → ``hn_mock``, ``api.telegram.org`` →
   ``telegram_mock``.
4. Assert (a) hn_mock /__mock/captured saw the GET /newest fetch,
   (b) telegram_mock received the formatted summary, (c) the
   summary text mentions BOTH seeded posts (Alpha + Beta).
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


async def _seed_hn(mock_hn_url: str) -> None:
    """Re-seed canonical canary posts so the run is deterministic
    regardless of any earlier scenario's state mutations."""
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.post(
            f"{mock_hn_url}/__mock/seed_posts",
            json={
                "posts": [
                    {
                        "title": "Show HN: Canary Post Alpha",
                        "url": "https://example.com/alpha",
                        "by": "canary_alpha",
                    },
                    {
                        "title": "Show HN: Canary Post Beta",
                        "url": "https://example.com/beta",
                        "by": "canary_beta",
                    },
                ]
            },
        )
        response.raise_for_status()


async def _captured(mock_hn_url: str) -> list[dict[str, Any]]:
    async with httpx.AsyncClient(timeout=5.0) as client:
        response = await client.get(f"{mock_hn_url}/__mock/captured")
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
    mode = "hn_monitor"

    if mock_hn_url is None:
        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "mock_hn_url not provided"},
            )
        ]

    try:
        await _seed_hn(mock_hn_url)

        routine_id = insert_lightweight_cron_routine(
            stack.db_path,
            user_id="workflow-canary-owner",
            name="canary-hn-monitor",
            prompt=(
                "Scrape the latest Hacker News and Telegram me a "
                "summary of the new Show HN posts.\n\n"
                "[CANARY-WORKFLOW-HN-FETCH]"
            ),
            description="canary script 3: hacker news -> telegram",
            fire_immediately=True,
        )

        runs = await wait_for_run(
            stack.db_path, routine_id, min_runs=1, timeout_secs=60.0
        )
        last_run = runs[0]
        run_terminal = last_run["status"] in SUCCESS_RUN_STATUSES

        # Verify hn_mock saw the GET /newest fetch
        hn_captured = await _captured(mock_hn_url)
        newest_seen = any(
            entry.get("method") == "GET" and entry.get("path") == "/newest"
            for entry in hn_captured
        )

        # Verify telegram captured the summary referencing BOTH posts
        telegram_match: dict[str, Any] | None = None
        if run_terminal:
            for _ in range(20):
                messages = await _capture_telegram_messages(mock_telegram_url)
                for m in messages:
                    text = m.get("text") or ""
                    if (
                        m.get("method") == "sendMessage"
                        and "[canary-workflow:hn_monitor]" in text
                        and "Canary Post Alpha" in text
                        and "Canary Post Beta" in text
                    ):
                        telegram_match = m
                        break
                if telegram_match is not None:
                    break
                await asyncio.sleep(0.5)

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = run_terminal and newest_seen and telegram_match is not None

        return [
            ProbeResult(
                provider="routines",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "routine_id": routine_id,
                    "run_status": last_run["status"],
                    "newest_seen": newest_seen,
                    "hn_captured_count": len(hn_captured),
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
