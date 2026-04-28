"""Script 3 — Hacker News Keyword Monitor (issue #1044), Phase 1A.

Original goal: hourly cron checks Hacker News for "Show HN" posts,
sends formatted summaries to Telegram. The script also covers the
"first immediate run" semantics — the routine should fire once at
creation time, not just on the next cron tick.

Phase 1A coverage: cron routine fires, lightweight action completes
end-to-end via mock LLM. Catches regressions in the routine engine
under an HTTP-fetch → format → Telegram pipeline shape.

Phase 1B follow-ups (not in this PR):
- Mock news.ycombinator.com ``/newest`` page (HTTP allowlist for HN +
  IRONCLAW_TEST_HTTP_REMAP for the host)
- Telegram channel install + bot-token seed
- Assert the routine's "first immediate run" fires within the canary
  budget AND the mock Telegram received the formatted summary
- Verify the routine de-duplicates against previously-reported posts
  (per Script 3 PHASE 3)

Reporter: Emil.
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
        mode="hn_monitor",
        routine_name="canary-hn-monitor",
        prompt=(
            "hi — hacker news monitor: scrape the latest 'Show HN' "
            "posts and send a Telegram summary including title, link, "
            "author, and brief description"
        ),
        description="canary script 3: hacker news -> telegram",
    )
    return [result]
