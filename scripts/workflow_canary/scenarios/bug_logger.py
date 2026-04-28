"""Script 1 — Telegram → Google Sheet Bug Logger (issue #1044), Phase 1A.

Original goal: Telegram messages starting with "bug:" get appended to
a Google Sheet via a 2-minute cron routine.

Phase 1A coverage: cron routine fires, lightweight action completes
end-to-end via mock LLM. Catches regressions in the routine engine
under a Telegram→Sheet pipeline shape.

Phase 1B follow-ups (not in this PR):
- Telegram channel install + bot-token seed (shared with periodic_reminder)
- Mock Google Sheets ``values:append`` mock + IRONCLAW_TEST_HTTP_REMAP
  for ``sheets.googleapis.com``
- Inject a "bug: ..." message into the Telegram mock and assert the
  next routine fire writes a row to the Sheets mock
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
        mock_telegram_url=mock_telegram_url,
        provider="routines",
        mode="bug_logger",
        routine_name="canary-bug-logger",
        prompt_intro=(
            "Scan recent Telegram messages for entries starting with "
            "'bug:' and append each to the bug-tracking sheet, then "
            "send the user a Telegram acknowledgement."
        ),
        description="canary script 1: telegram bugs -> sheet",
    )
    return [result]
