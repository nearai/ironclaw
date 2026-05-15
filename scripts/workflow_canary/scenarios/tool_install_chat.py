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
import os
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


async def _open_thread(client: httpx.AsyncClient, base_url: str) -> str:
    response = await client.post(f"{base_url}/api/chat/thread/new", timeout=15.0)
    response.raise_for_status()
    return response.json()["id"]


async def _send_chat(
    client: httpx.AsyncClient, base_url: str, thread_id: str, content: str
) -> int:
    response = await client.post(
        f"{base_url}/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30.0,
    )
    return response.status_code


async def _read_history(
    client: httpx.AsyncClient, base_url: str, thread_id: str
) -> dict[str, Any]:
    response = await client.get(
        f"{base_url}/api/chat/history",
        params={"thread_id": thread_id},
        timeout=15.0,
    )
    response.raise_for_status()
    return response.json()


async def _get_extension(
    client: httpx.AsyncClient, base_url: str, name: str
) -> dict[str, Any] | None:
    response = await client.get(f"{base_url}/api/extensions", timeout=15.0)
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


# When the probe fails, we want enough breadcrumbs in the artifact to
# diagnose what the agent actually did without re-running the canary.
# The slack notifier surfaces `details.error` plus the structured
# fields it knows about; extra keys we drop into `details` show up in
# the artifact for whoever opens the failing-lane drilldown.
_TOOL_CALL_KEYS = ("tool_name", "name", "function", "action")


def _collect_tool_calls(history: dict[str, Any]) -> list[str]:
    """Best-effort enumeration of tool-call names found in history.

    Matches a few common envelope shapes the gateway has used:
    ``tool_calls: [{"name": ...}]`` on assistant messages, top-level
    ``tool_name``/``action`` on turn records, ``<tool_output name="...">``
    on tool-result content. Deduplicates while preserving order so the
    diagnostic doesn't double-count parallel dispatches.
    """
    seen: list[str] = []

    def _add(name: Any) -> None:
        if isinstance(name, str) and name and name not in seen:
            seen.append(name)

    def _walk(node: Any) -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "tool_calls" and isinstance(value, list):
                    for call in value:
                        if isinstance(call, dict):
                            _add(call.get("name") or call.get("function"))
                elif key in _TOOL_CALL_KEYS and isinstance(value, str):
                    _add(value)
                else:
                    _walk(value)
        elif isinstance(node, list):
            for item in node:
                _walk(item)
        elif isinstance(node, str):
            # `<tool_output name="...">` wrapping
            import re

            for m in re.finditer(r'<tool_output\s+name="([^"]+)"', node):
                _add(m.group(1))

    _walk(history)
    return seen


def _last_assistant_text(history: dict[str, Any]) -> str:
    """Pull the last assistant-side text we can find for diagnostics.

    Tolerates the ``turns: [{response: "..."}]`` shape the gateway uses
    today plus common message-list shapes. Truncated so the artifact
    stays small.
    """
    candidates: list[str] = []

    def _walk(node: Any) -> None:
        if isinstance(node, dict):
            for key in ("response", "content", "text"):
                value = node.get(key)
                if isinstance(value, str) and value:
                    candidates.append(value)
            for value in node.values():
                if not isinstance(value, str):
                    _walk(value)
        elif isinstance(node, list):
            for item in node:
                _walk(item)

    _walk(history)
    if not candidates:
        return ""
    return candidates[-1][:300]


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
    client: httpx.AsyncClient, base_url: str, name: str, deadline: float
) -> tuple[bool, dict[str, Any] | None]:
    """Poll until the extension appears in /api/extensions or deadline expires.

    The /api/extensions response carries no boolean ``installed`` field —
    presence in the list is itself the install confirmation. The runtime
    state lives in ``authenticated`` / ``active`` / ``tools`` which can
    legitimately stay false for a freshly-installed extension that
    still needs OAuth (the natural end state for this probe — gmail
    parks on an auth gate after install, which is exactly the flow we
    want to verify).

    Returns (registered, last_extension_seen).
    """
    last: dict[str, Any] | None = None
    while time.perf_counter() < deadline:
        ext = await _get_extension(client, base_url, name)
        if ext is not None:
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

    # Single client for the whole probe — the install-poll loop calls
    # /api/extensions ~120 times across TIMEOUT_S at POLL_INTERVAL_S
    # cadence. Reusing the client keeps HTTP keepalives warm and avoids
    # the per-call TCP/TLS dance.
    auth_headers = {"Authorization": f"Bearer {token}"}
    try:
        async with httpx.AsyncClient(headers=auth_headers) as client:
            thread_id = await _open_thread(client, base_url)
            send_status = await _send_chat(
                client, base_url, thread_id, TRIGGER_PROMPT
            )
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
            registered, ext = await _wait_for_install(
                client, base_url, TARGET_EXTENSION, deadline
            )

            history = await _read_history(client, base_url, thread_id)
        text = _history_text(history)
        # Use the structured tool-call enumerator rather than the
        # substring walker — production tool calls live as
        # `{"name": "tool_install", "arguments": {"name": "gmail"}}`
        # where the two needles live in different string fields, so a
        # same-string-only walker silently misses them. The
        # _history_has_tool_install helper is kept as a defensive
        # back-stop for envelope shapes where the name does appear
        # inline as a string.
        tool_install_seen = (
            "tool_install" in _collect_tool_calls(history)
            or _history_has_tool_install(history, TARGET_EXTENSION)
        )
        forbidden_hits = [frag for frag in FORBIDDEN_FRAGMENTS if frag in text]

        latency_ms = int((time.perf_counter() - started) * 1000)
        success = (
            registered and tool_install_seen and not forbidden_hits
        )

        details: dict[str, Any] = {
            "thread_id": thread_id,
            "trigger_prompt": TRIGGER_PROMPT,
            "target_extension": TARGET_EXTENSION,
            "extension_registered": registered,
            "tool_install_seen_in_history": tool_install_seen,
            "extension_state": (
                {k: ext.get(k) for k in ("authenticated", "active", "needs_setup")}
                if ext is not None
                else None
            ),
            "forbidden_fragments_seen": forbidden_hits,
            "history_length_chars": len(text),
            # Diagnostic surface — only meaningful on failure but cheap
            # enough to always emit. The probe's primary failure mode
            # ("agent didn't reach tool_install") has too many possible
            # root causes (LLM-surface regression / approval gate parked
            # / auth env not propagated / wrong engine version) to
            # distinguish without seeing what the agent actually did.
            "tool_calls_observed": _collect_tool_calls(history),
            "pending_gate": (history.get("pending_gate") if isinstance(history, dict) else None),
            "last_assistant_text": _last_assistant_text(history),
            "agent_auto_approve_env": os.environ.get("AGENT_AUTO_APPROVE_TOOLS"),
            "allow_local_tools_env": os.environ.get("ALLOW_LOCAL_TOOLS"),
        }
        if not success:
            # Build a short, structured error string so the slack
            # reason field surfaces the actual failure mode and not
            # just "False".
            reasons: list[str] = []
            if not registered:
                reasons.append(
                    f"{TARGET_EXTENSION} did not appear in /api/extensions "
                    f"within {TIMEOUT_S:.0f}s — install never reached the "
                    "extension manager"
                )
            if not tool_install_seen:
                reasons.append(
                    "no tool_install invocation observed in history — "
                    "agent surface regression (tool_install hidden from "
                    "callable surface)"
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
