"""Log-error assertion probe — covers issue #1044's per-script
"watch the logs" fail criteria all at once:

- ``chat_id 'default'`` — Telegram channel sent to wrong chat
- ``parsed naive timestamp without timezone`` — naive-datetime warning
- ``retry after None`` — rate-limit error path with a None retry hint
- ``expected a sequence`` — google-sheets values:append payload shape

This probe runs LAST in the workflow-canary lane (so it sees the full
log surface from prior probes), reads the gateway log file, and
asserts none of these regex patterns appear. A single hit fails the
probe with the specific log line surfaced in details.
"""

from __future__ import annotations

import re
import time
from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult

FAIL_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    (
        "chat_id_default",
        re.compile(r"chat_id\s+'default'", re.IGNORECASE),
    ),
    (
        "naive_timestamp",
        re.compile(
            r"parsed naive timestamp without timezone", re.IGNORECASE
        ),
    ),
    (
        "retry_after_none",
        re.compile(r"retry after None", re.IGNORECASE),
    ),
    (
        "expected_sequence",
        re.compile(r"expected a sequence", re.IGNORECASE),
    ),
]


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
    mode = "log_assertions"

    log_path = log_dir / "gateway.log"
    if not log_path.exists():
        return [
            ProbeResult(
                provider="logs",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"gateway log not found at {log_path}"},
            )
        ]

    text = log_path.read_text(encoding="utf-8", errors="replace")
    hits: list[dict[str, str]] = []
    for label, pattern in FAIL_PATTERNS:
        match = pattern.search(text)
        if match:
            # Find the surrounding line for diagnostic context
            start = text.rfind("\n", 0, match.start()) + 1
            end = text.find("\n", match.end())
            if end == -1:
                end = len(text)
            hits.append(
                {
                    "label": label,
                    "line": text[start:end][:300],
                }
            )

    latency_ms = int((time.perf_counter() - started) * 1000)
    success = len(hits) == 0

    return [
        ProbeResult(
            provider="logs",
            mode=mode,
            success=success,
            latency_ms=latency_ms,
            details={
                "log_path": str(log_path),
                "hits": hits,
                "patterns_checked": [label for label, _ in FAIL_PATTERNS],
            },
        )
    ]
