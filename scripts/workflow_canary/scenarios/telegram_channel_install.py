"""Telegram channel install probe — covers issue #1044's
"HTTP 404 on valid token" fail criterion (Script 4 PHASE 1.1) and
"channel reaches Active state" assertion.

Coverage:
1. POST /api/extensions/install with kind=wasm_channel.
2. Patch the installed capabilities to skip validation_endpoint
   (would 404 against the real Telegram API in tests).
3. POST /api/extensions/telegram/setup with a canary bot token.
4. Poll /api/extensions to confirm the channel appears installed.
"""

from __future__ import annotations

import time
from pathlib import Path
from typing import Any

from scripts.live_canary.common import ProbeResult
from scripts.workflow_canary.telegram_setup import (
    BOT_TOKEN,
    WEBHOOK_SECRET,
    install_telegram_channel,
    wait_for_telegram_active,
    patch_capabilities,
    setup_telegram_channel,
)


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
    mode = "telegram_channel_install"

    if not stack.channels_dir:
        return [
            ProbeResult(
                provider="extensions",
                mode=mode,
                success=False,
                latency_ms=0,
                details={"error": "stack.channels_dir not populated"},
            )
        ]

    install_response: dict[str, Any] = {}
    setup_response: dict[str, Any] = {}
    active = False
    error: str | None = None

    try:
        install_response = await install_telegram_channel(
            stack.base_url, stack.gateway_token
        )
        # The install API copies the bundle into stack.channels_dir;
        # patch it BEFORE setup so validation_endpoint is gone.
        patch_capabilities(stack.channels_dir)
        setup_response = await setup_telegram_channel(
            stack.base_url,
            stack.gateway_token,
            bot_token=BOT_TOKEN,
            webhook_secret=WEBHOOK_SECRET,
        )
        active = await wait_for_telegram_active(
            stack.base_url, stack.gateway_token, timeout_secs=15.0
        )
    except Exception as exc:  # noqa: BLE001
        error = f"{type(exc).__name__}: {exc}"

    latency_ms = int((time.perf_counter() - started) * 1000)
    success = error is None and bool(setup_response.get("success")) and active

    details: dict[str, Any] = {
        "install_status": install_response.get("status"),
        "setup_success": setup_response.get("success"),
        "setup_activated": setup_response.get("activated"),
        "active": active,
    }
    if error is not None:
        details["error"] = error

    return [
        ProbeResult(
            provider="extensions",
            mode=mode,
            success=success,
            latency_ms=latency_ms,
            details=details,
        )
    ]
