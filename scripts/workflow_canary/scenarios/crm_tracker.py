"""Script 5 — Email → CRM Inbound Tracker (issue #1044), Phase 1A.

Original goal: hourly cron reads Gmail inbox, the LLM classifies
inbound sales leads, each lead is appended to a Google Sheet
"Inbound CRM" with structured columns (Company, Contact Name, Email,
Status, Notes, Next Action). Optional Telegram summary on each fire.

Phase 1A coverage: cron routine fires, lightweight action completes
end-to-end via mock LLM. Catches regressions in the routine engine
under a Gmail-read → LLM-classify → Sheets-write pipeline shape.

Phase 1B follow-ups (not in this PR):
- Mock Gmail ``messages.list`` + ``messages.get`` (already partial via
  the auth-live-canary mock; extend with a deterministic inbox seed)
- Mock Google Sheets ``values:append`` mock + headers row creation
- Mock LLM canned responses that classify deterministic seed emails
  into structured CRM rows (this is the LLM-driven part — needs the
  mock LLM to issue structured tool calls based on email content)
- Optional Telegram summary verification against the mock Telegram
- De-duplication assertion across multiple routine fires (Script 5
  Phase 5.5: "Verify no duplicates on second run")

Reporter: Cameron.
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
        mode="crm_tracker",
        routine_name="canary-crm-tracker",
        prompt_intro=(
            "Scan recent Gmail messages for inbound sales leads, "
            "classify each into Company / Contact Name / Email / "
            "Status / Notes / Next Action, append rows to the "
            "'Inbound CRM' sheet, and send a Telegram summary."
        ),
        description="canary script 5: gmail -> sheets CRM",
    )
    return [result]
