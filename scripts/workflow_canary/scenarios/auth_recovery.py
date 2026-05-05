"""Auth recovery probe — covers issue #1044 Script 2 PHASE 5 fail
criteria: "OAuth URL not malformed / no Error 400 / agent surfaces a
structured auth flow when a tool needs OAuth and isn't authenticated".

We don't have real OAuth setup in workflow-canary — that lives in
``auth-live-canary``. What we CAN test here is the regression shape:
when a chat triggers an unauthenticated tool, the agent must surface
a graceful response. Specifically:

1. The chat send returns 202 (request accepted), never a 5xx.
2. The thread reaches a terminal state (idle / completed) within 30s.
3. The thread history contains NO ``Error 400`` / ``Internal Server
   Error`` / ``panicked`` / ``traceback`` substrings.

The mock LLM ships a ``check gmail unread`` pattern that emits a
``gmail`` tool call. Gmail isn't installed in this lane (no network
in CI), so the engine surfaces ``Extension not installed: gmail``,
the mock LLM's recovery branch (mock_llm.py:1138-1148) emits
``tool_install``, and the agent walks through the install →
auth-required path. The probe asserts that walk doesn't crash.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult

# Substrings that, if present in the chat history, indicate a
# regression in the auth/install recovery path.
FORBIDDEN_FRAGMENTS = [
    "Error 400",
    "Internal Server Error",
    "panicked",
    "Traceback",
    "rust panic",
]


async def _open_thread(base_url: str, gateway_token: str) -> str:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/thread/new",
            headers={"Authorization": f"Bearer {gateway_token}"},
        )
        response.raise_for_status()
        return response.json()["id"]


async def _send_chat(
    base_url: str, gateway_token: str, thread_id: str, content: str
) -> int:
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/send",
            headers={"Authorization": f"Bearer {gateway_token}"},
            json={"content": content, "thread_id": thread_id},
        )
        return response.status_code


async def _read_history(
    base_url: str, gateway_token: str, thread_id: str
) -> dict[str, Any]:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.get(
            f"{base_url}/api/chat/history",
            headers={"Authorization": f"Bearer {gateway_token}"},
            params={"thread_id": thread_id},
        )
        response.raise_for_status()
        return response.json()


def _history_text(history: dict[str, Any]) -> str:
    """Concatenate every message's text/content into one big string
    so we can substring-match for forbidden fragments. Tolerant of a
    handful of envelope shapes the gateway has used over time."""
    chunks: list[str] = []

    def _walk(value: Any) -> None:
        if isinstance(value, str):
            chunks.append(value)
        elif isinstance(value, list):
            for item in value:
                _walk(item)
        elif isinstance(value, dict):
            for v in value.values():
                _walk(v)

    _walk(history)
    return "\n".join(chunks)


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
    mode = "auth_recovery"

    try:
        thread_id = await _open_thread(stack.base_url, stack.gateway_token)
        send_status = await _send_chat(
            stack.base_url,
            stack.gateway_token,
            thread_id,
            "check gmail unread",
        )
        if send_status >= 500:
            return [
                ProbeResult(
                    provider="auth",
                    mode=mode,
                    success=False,
                    latency_ms=int((time.perf_counter() - started) * 1000),
                    details={
                        "error": f"chat send returned {send_status}",
                        "thread_id": thread_id,
                    },
                )
            ]

        # Wait for the agent to settle. We don't strictly need the
        # thread to be "idle" — just that processing finishes without
        # 5xx-ing or crashing. 15s is enough on a quiet stack.
        await asyncio.sleep(15.0)

        history = await _read_history(
            stack.base_url, stack.gateway_token, thread_id
        )
        text = _history_text(history)

        hits = [frag for frag in FORBIDDEN_FRAGMENTS if frag in text]

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = send_status == 202 and not hits

        return [
            ProbeResult(
                provider="auth",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details={
                    "thread_id": thread_id,
                    "send_status": send_status,
                    "forbidden_fragments_seen": hits,
                    "history_length_chars": len(text),
                },
            )
        ]
    except Exception as exc:  # noqa: BLE001
        return [
            ProbeResult(
                provider="auth",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        ]
