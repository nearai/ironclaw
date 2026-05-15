"""Chat-driven tool_install probe — would have caught the May 8 regression
shipped in #3366, where `tool_install` (and the unified `tool_activate`
it was meant to be subsumed by) was silently dropped from the agent's
callable surface. The HTTP install API kept working, so every existing
canary stayed green for five days while the chat path was broken.

What this probe asserts
-----------------------

1. **Chat send succeeds** (HTTP 202 from `/api/chat/send`).
2. **The agent actually invokes `tool_install`** for the requested
   extension within ``TIMEOUT_S``. Asserted directly from history —
   no inference from secondary effects.
3. **The extension reaches `installed=true`** via `GET /api/extensions`.
4. **No forbidden error/panic substrings** in the rendered history.

Hooks into the existing mock LLM contract
-----------------------------------------

The mock LLM at ``tests/e2e/mock_llm.py`` already ships a canned
flow for ``"check gmail unread"`` (mock_llm.py:1257-1292):

- Turn 1: dispatches a bare ``gmail`` tool call. Engine rejects with
  "Extension not installed:" / "is not callable in this execution
  context".
- Turn 2: emits ``tool_install(name="gmail")``. **This is exactly
  the call #3366 broke** — by hiding ``tool_install`` from the
  agent surface, the mock LLM's tool call would be rejected /
  ignored, gmail would never install, and the existing
  ``auth_recovery`` probe (which only greps for error substrings)
  would still pass.
- Turn 3: re-emits ``gmail``; engine raises an OAuth gate.

We don't drive the OAuth completion here — gmail reaching
``installed=true`` is the failure boundary the regression crosses.

Why a separate probe and not just tightening ``auth_recovery``
--------------------------------------------------------------

``auth_recovery`` is intentionally lenient: it asserts the *recovery
shape* (no 5xx, no panic) rather than that any specific tool ran.
Tightening it would conflate two distinct guarantees — "the engine
recovers gracefully from unauthenticated calls" and "the agent has
an install primitive on its callable surface." Keep both, isolate
the contracts.
"""

from __future__ import annotations

import asyncio
import time
from pathlib import Path
from typing import Any

import httpx

from scripts.live_canary.common import ProbeResult


# Extension we drive through the install flow. Pinned to gmail because
# the mock LLM's ``check gmail unread`` canned response already encodes
# the full chat→tool_install→retry sequence. Swapping to a different
# extension would require a new mock LLM branch.
TARGET_EXTENSION = "gmail"

# Trigger phrase that maps to mock_llm.py's gmail-install canned flow.
# Keep in lockstep with ``mock_llm.py:1257-1292`` — if the trigger
# string changes there, change it here.
TRIGGER_PROMPT = "check gmail unread"

# 60s is generous. Locally the full flow (chat → tool_install →
# install completes → gmail retry → gate) settles in <10s; the budget
# absorbs slow CI runners and any post-install verification the engine
# does (capability seeding, etc).
TIMEOUT_S = 60.0

# Polling cadence on /api/extensions. Cheap call, so tight is fine.
POLL_INTERVAL_S = 0.5

FORBIDDEN_FRAGMENTS = [
    "Error 400",
    "Internal Server Error",
    "panicked",
    "Traceback",
    "rust panic",
]


async def _open_thread(base_url: str, token: str) -> str:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/thread/new",
            headers={"Authorization": f"Bearer {token}"},
        )
        response.raise_for_status()
        return response.json()["id"]


async def _send_chat(
    base_url: str, token: str, thread_id: str, content: str
) -> int:
    async with httpx.AsyncClient(timeout=30.0) as client:
        response = await client.post(
            f"{base_url}/api/chat/send",
            headers={"Authorization": f"Bearer {token}"},
            json={"content": content, "thread_id": thread_id},
        )
        return response.status_code


async def _read_history(
    base_url: str, token: str, thread_id: str
) -> dict[str, Any]:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.get(
            f"{base_url}/api/chat/history",
            headers={"Authorization": f"Bearer {token}"},
            params={"thread_id": thread_id},
        )
        response.raise_for_status()
        return response.json()


async def _get_extension(
    base_url: str, token: str, name: str
) -> dict[str, Any] | None:
    async with httpx.AsyncClient(timeout=15.0) as client:
        response = await client.get(
            f"{base_url}/api/extensions",
            headers={"Authorization": f"Bearer {token}"},
        )
        response.raise_for_status()
        for ext in response.json().get("extensions", []):
            if ext.get("name") == name:
                return ext
    return None


def _history_has_tool_install(
    history: dict[str, Any], extension: str
) -> bool:
    """True iff a ``tool_install`` invocation targeting ``extension``
    appears anywhere in the history payload.

    The gateway has used a handful of envelope shapes over time for tool
    calls — sometimes ``{"tool_calls": [{"name": ..., "arguments":
    {...}}]}`` on assistant messages, sometimes ``<tool_output
    name="...">`` wrapping, sometimes ``tool_name`` / ``action`` on the
    turn record. Walk the whole tree and substring-match instead of
    binding to one shape, so this probe survives benign envelope
    refactors. The phrase "tool_install" + the extension name appearing
    together in a single message is a strong-enough signal — false
    positives would require user text or LLM prose to mention both
    tokens, which doesn't happen with the canned mock LLM responses.
    """
    needle_tool = "tool_install"
    needle_name = f'"{extension}"'

    def _walk(node: Any) -> bool:
        if isinstance(node, str):
            return needle_tool in node and needle_name in node
        if isinstance(node, list):
            return any(_walk(x) for x in node)
        if isinstance(node, dict):
            return any(_walk(v) for v in node.values())
        return False

    return _walk(history)


def _history_text(history: dict[str, Any]) -> str:
    chunks: list[str] = []

    def _walk(node: Any) -> None:
        if isinstance(node, str):
            chunks.append(node)
        elif isinstance(node, list):
            for x in node:
                _walk(x)
        elif isinstance(node, dict):
            for v in node.values():
                _walk(v)

    _walk(history)
    return "\n".join(chunks)


async def _wait_for_install(
    base_url: str, token: str, name: str, deadline: float
) -> tuple[bool, dict[str, Any] | None]:
    """Poll until the extension is `installed=true` or deadline expires.

    Returns (installed, last_extension_seen).
    """
    last: dict[str, Any] | None = None
    while time.perf_counter() < deadline:
        ext = await _get_extension(base_url, token, name)
        if ext is not None:
            last = ext
            if ext.get("installed") is True:
                return True, ext
        await asyncio.sleep(POLL_INTERVAL_S)
    return False, last


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
    mode = "tool_install_chat"
    base_url = stack.base_url
    token = stack.gateway_token

    try:
        thread_id = await _open_thread(base_url, token)
        send_status = await _send_chat(base_url, token, thread_id, TRIGGER_PROMPT)
        if send_status != 202:
            return [
                ProbeResult(
                    provider="extensions",
                    mode=mode,
                    success=False,
                    latency_ms=int((time.perf_counter() - started) * 1000),
                    details={
                        "error": f"chat send returned {send_status}, expected 202",
                        "thread_id": thread_id,
                        "trigger_prompt": TRIGGER_PROMPT,
                    },
                )
            ]

        deadline = time.perf_counter() + TIMEOUT_S
        installed, ext = await _wait_for_install(
            base_url, token, TARGET_EXTENSION, deadline
        )

        history = await _read_history(base_url, token, thread_id)
        text = _history_text(history)
        tool_install_seen = _history_has_tool_install(history, TARGET_EXTENSION)
        forbidden_hits = [frag for frag in FORBIDDEN_FRAGMENTS if frag in text]

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            installed and tool_install_seen and not forbidden_hits
        )

        details: dict[str, Any] = {
            "thread_id": thread_id,
            "trigger_prompt": TRIGGER_PROMPT,
            "target_extension": TARGET_EXTENSION,
            "installed": installed,
            "tool_install_seen_in_history": tool_install_seen,
            "extension_state": (
                {k: ext.get(k) for k in ("installed", "authenticated", "active")}
                if ext is not None
                else None
            ),
            "forbidden_fragments_seen": forbidden_hits,
            "history_length_chars": len(text),
        }
        if not success:
            # Build a short, structured error string so the slack
            # reason field surfaces the actual failure mode and not
            # just "False".
            reasons: list[str] = []
            if not installed:
                reasons.append(
                    f"{TARGET_EXTENSION} did not reach installed=true within "
                    f"{TIMEOUT_S:.0f}s — the agent likely cannot see "
                    "tool_install on its callable surface"
                )
            if not tool_install_seen:
                reasons.append(
                    "no tool_install invocation observed in history — "
                    "agent surface regression"
                )
            if forbidden_hits:
                reasons.append(f"forbidden fragments: {forbidden_hits}")
            details["error"] = "; ".join(reasons)

        return [
            ProbeResult(
                provider="extensions",
                mode=mode,
                success=success,
                latency_ms=latency_ms,
                details=details,
            )
        ]
    except Exception as exc:  # noqa: BLE001
        return [
            ProbeResult(
                provider="extensions",
                mode=mode,
                success=False,
                latency_ms=int((time.perf_counter() - started) * 1000),
                details={"error": f"{type(exc).__name__}: {exc}"},
            )
        ]
