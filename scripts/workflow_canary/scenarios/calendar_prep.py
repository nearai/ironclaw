"""Script 2 — Calendar Prep Assistant (issue #1044), Phase 1A.

Original goal: 10 min before each Google Calendar meeting, send a
Telegram message summarizing company background + recent news for
external attendees.

Phase 1A coverage: cron routine fires, lightweight action completes
end-to-end via mock LLM. Catches regressions in the routine engine
under a Calendar→summarize→Telegram pipeline shape.

Phase 1B follow-ups (not in this PR):
- Mock Google Calendar reads (events list mock, IRONCLAW_TEST_HTTP_REMAP
  for ``calendar.googleapis.com``)
- Mock web search (``web-search`` WASM tool credential + HTTP mock)
- Telegram channel install + bot-token seed
- Assert mock Calendar got the events query and Telegram mock got
  the prep message at the expected lead-time

Reporter: Nick.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.scenarios._common import run_routine_probe


async def run(
    *,
    stack: Any,
    mock_telegram_url: str,
    output_dir: Path,
    log_dir: Path,
) -> list[ProbeResult]:
    result = await run_routine_probe(
        stack=stack,
        provider="routines",
        mode="calendar_prep",
        routine_name="canary-calendar-prep",
        prompt=(
            "hi — calendar prep: list upcoming meetings, look up each "
            "external attendee's company background, and send a "
            "Telegram briefing 10 minutes before each meeting"
        ),
        description="canary script 2: calendar prep -> telegram",
    )
    return [result]
