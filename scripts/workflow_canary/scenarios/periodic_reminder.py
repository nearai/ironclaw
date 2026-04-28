"""Script 4 — Periodic Reminder via Telegram (issue #1044).

End-to-end coverage:

1. Backdated cron routine inserted via libSQL.
2. Routine engine cron-tick picks it up.
3. Lightweight action runs against the mock LLM, which emits an
   ``http`` POST to ``api.telegram.org/.../sendMessage`` (matched on
   the per-scenario ``[CANARY-WORKFLOW-periodic_reminder]`` sentinel
   in ``tests/e2e/mock_llm.py``).
4. ``IRONCLAW_TEST_HTTP_REMAP`` rewrites the Telegram host to the
   loopback ``telegram_mock`` server, which records the message.
5. Probe asserts both (a) ``routine_runs`` reaches a terminal status,
   and (b) ``telegram_mock`` captured a ``sendMessage`` whose text
   contains ``[canary-workflow:periodic_reminder] ack``.

Steps 1–5 are all wired through ``run_routine_probe`` with
``verify_telegram=True`` — see ``scenarios/_common.py``. Channel
*install* (``/api/extensions/install`` + capability patch +
``/api/extensions/telegram/setup`` + pairing) is exercised by the
sibling ``telegram_*`` scenarios; this scenario covers the
routine-driven sendMessage path specifically and intentionally hits
api.telegram.org via the raw http tool rather than through the
installed channel.

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
    mock_sheets_url: str | None = None,
    mock_calendar_url: str | None = None,
    mock_hn_url: str | None = None,
    mock_gmail_url: str | None = None,
    mock_web_search_url: str | None = None,
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
