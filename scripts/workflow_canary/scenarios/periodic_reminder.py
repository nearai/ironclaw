"""Script 4 — Periodic Reminder via Telegram (issue #1044), Phase 1A.

Verifies the routine engine picks up a backdated cron routine, fires
its Lightweight action against the (mock) LLM, and records a
``routine_runs`` row with terminal status. The original NL script
calls for a Telegram delivery side-effect; that layer (Telegram channel
install + bot-token seed + sendMessage assertion against the mock
Telegram server) is Phase 1B follow-up work.

Reporter: Henry.
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
        mode="periodic_reminder",
        routine_name="canary-periodic-reminder",
        prompt_intro="Send the user a Telegram reminder to walk the dog.",
        description="canary script 4: dog walk reminder",
        # Phase 1B: with the routine engine's http_interceptor
        # propagation fix in place, the http tool dispatch from
        # the Lightweight action now reaches the mock Telegram
        # bot via IRONCLAW_TEST_HTTP_REMAP. Verify the bot
        # captured a sendMessage with the expected ack text.
        verify_telegram=True,
    )
    return [result]
