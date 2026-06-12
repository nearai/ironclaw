"""Regression: assistant delivers work, not a plan of work.

Pins the user-visible contract from nearai/ironclaw#2544 — "Agent plans
and delegates tasks but never executes or completes them." The QA repro
was: give the agent a multi-step task ("research X and create a
summary"), the agent narrates a plan ("I'll do X, then Y, then Z"), and
the conversation ends there with no tool calls and no concrete output.

What this test deliberately does NOT do
---------------------------------------

It does not assert that any specific *mechanism* fired — the tool-intent
nudge, a hypothetical hard rejection gate, a system-prompt change, a
model swap. The user-facing bug is independent of how the team chooses
to fix it: if the system prompt makes the agent more directive, that is
a valid fix. If the orchestrator nudge is strengthened, that is also a
valid fix. The contract this test pins is the **outcome**:

  Given a realistic user prompt that requires concrete work, the
  assistant's final response must include actual tool execution AND a
  result derived from a tool, not a narration of intent.

Architecture
------------

Live-LLM record/replay via ``live_llm_proxy.py``, mirroring the
``test_mission_gmail_3133.py`` pattern. Re-record the trace whenever
the system prompt, model choice, or orchestrator changes; the assertions
below verify the new configuration still produces actual work.

In replay mode (the default) the test is skipped if the fixture JSON
is missing, so a fresh checkout doesn't hard-fail before someone has
recorded a trace.
"""

import asyncio

import httpx
import pytest

from helpers import AUTH_TOKEN, api_get, api_post


# ── Realistic prompts ─────────────────────────────────────────────────


# A "concrete two-step ask" pinned to *deterministic* tools so the
# live-LLM record/replay harness can hash-match across runs. The first
# `time`/random tools both broke replay — their results differ between
# record and replay, so the second LLM-call hash drifts.
#
# The unique sentinel word here ("foxtrot-juliet-7714") survives only
# if the agent actually echoes-then-writes it; a plan-only response
# ("I'll echo it and save it") cannot satisfy the file-content or the
# terminal-text assertions.
SENTINEL = "foxtrot-juliet-7714"
TWO_STEP_ECHO_THEN_WRITE = (
    f"Use the echo tool to echo the exact string {SENTINEL!r}. "
    f"Then save that echoed string into the workspace at memory/echo.md. "
    f"Once both are done, tell me what you saved."
)


# ── Helpers ──────────────────────────────────────────────────────────


async def _new_thread(base_url: str) -> str:
    response = await api_post(base_url, "/api/chat/thread/new", timeout=15)
    response.raise_for_status()
    return response.json()["id"]


async def _send(base_url: str, thread_id: str, content: str) -> None:
    response = await api_post(
        base_url,
        "/api/chat/send",
        json={"content": content, "thread_id": thread_id},
        timeout=30,
    )
    assert response.status_code in (200, 202), response.text[:400]


async def _wait_for_terminal_assistant(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 180.0,
) -> dict:
    """Poll history until the last turn carries a non-empty assistant
    response. Returns the full history dict."""
    deadline = asyncio.get_event_loop().time() + timeout
    last_history: dict = {}
    while asyncio.get_event_loop().time() < deadline:
        response = await api_get(
            base_url, f"/api/chat/history?thread_id={thread_id}", timeout=15
        )
        response.raise_for_status()
        history = response.json()
        last_history = history
        turns = history.get("turns") or []
        if turns and (turns[-1].get("response") or "").strip():
            return history
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"thread {thread_id} never produced a terminal response within "
        f"{timeout}s. Last history: {last_history}"
    )


async def _read_workspace(base_url: str, path: str) -> str | None:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{base_url}/api/memory/read",
            params={"path": path},
            headers={"Authorization": f"Bearer {AUTH_TOKEN}"},
            timeout=10,
        )
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return response.json().get("content") or ""


def _collect_tool_calls(history: dict) -> list[dict]:
    """Flatten every persisted tool_call across every turn."""
    out: list[dict] = []
    for turn in history.get("turns") or []:
        out.extend(turn.get("tool_calls") or [])
    return out


# ── Test ─────────────────────────────────────────────────────────────


async def test_two_step_task_produces_work_not_plan_2544(intent_live_server):
    """User asks for a two-step concrete task. Assert the assistant
    actually does it.

    A plan-only response ("I'll echo it and save it") cannot satisfy
    any of the asserts below. Each assertion targets a distinct piece
    of observable work:

      1. At least one ``echo`` tool call ran (the agent dispatched the
         first step of the task, not just narrated it).
      2. At least one ``memory_write`` tool call ran (the agent
         persisted the value the user asked it to persist).
      3. The workspace actually contains ``memory/echo.md``, and its
         content includes the sentinel string — proof the write
         carried real data, not a placeholder.
      4. The terminal assistant response quotes the sentinel back, so
         we know the agent reported the real result, not a generic
         "I'll let you know once I'm done."

    The four together form the contract from #2544: the assistant did
    the work, not just described it.

    Tools are deliberately deterministic (``echo`` + ``memory_write``)
    so the live-LLM record/replay harness can hash-match across runs.
    A non-deterministic tool like ``time`` would produce different
    tool-result messages on every run, drifting the hash and missing
    every recorded entry after the first turn.
    """
    server = intent_live_server["base_url"]
    mode = intent_live_server["mode"]
    print(f"[#2544] running in {mode} mode against {server}")

    thread_id = await _new_thread(server)
    await _send(server, thread_id, TWO_STEP_ECHO_THEN_WRITE)
    history = await _wait_for_terminal_assistant(server, thread_id, timeout=180)

    tool_calls = _collect_tool_calls(history)
    tool_names = {
        (tc.get("name") or tc.get("tool_name") or "").lower()
        for tc in tool_calls
    }

    # 1. The echo tool ran.
    assert "echo" in tool_names, (
        f"#2544 contract: assistant must call the `echo` tool to "
        f"perform the first step, not invent or narrate it. Tool "
        f"calls in history: {sorted(tool_names)}"
    )

    # 2. The memory_write tool ran. (The user asked the agent to save
    #    the echoed string into the workspace; a plan-only response
    #    would skip this step.)
    assert "memory_write" in tool_names, (
        f"#2544 contract: assistant must call the `memory_write` tool "
        f"to persist the sentinel the user asked for. Tool calls in "
        f"history: {sorted(tool_names)}"
    )

    # 3. The workspace file actually exists and contains the sentinel.
    #    Load-bearing side-effect assertion: a plan-only response with
    #    no real tool dispatch leaves the workspace empty.
    content = await _read_workspace(server, "memory/echo.md")
    assert content is not None, (
        "#2544 contract: workspace must contain memory/echo.md after "
        "the two-step task; the assistant narrated saving without "
        "actually writing the file"
    )
    assert SENTINEL in content, (
        f"#2544 contract: memory/echo.md must contain the sentinel "
        f"{SENTINEL!r}, not a placeholder or partial value. "
        f"Got: {content!r}"
    )

    # 4. Terminal response quotes the sentinel back. A plan-only
    #    response ("I'll echo and save…") cannot satisfy this because
    #    it lacks the sentinel.
    terminal = (history["turns"][-1].get("response") or "").strip()
    assert terminal, "#2544 contract: terminal assistant response is empty"
    assert SENTINEL in terminal, (
        f"#2544 contract: terminal response must quote the sentinel "
        f"{SENTINEL!r} back to the user; that's the contract for "
        f"'told me what you saved'. Got: {terminal[:400]!r}"
    )

    # Live-mode sanity: a non-zero number of LLM calls were recorded.
    if mode == "record":
        from live_harness import proxy_state
        st = await proxy_state(intent_live_server["live_proxy_url"])
        assert st["record_count"] > 0, (
            f"record mode should have captured LLM calls: {st}"
        )
        print(
            f"[#2544] recorded {st['record_count']} LLM call(s) into "
            f"{intent_live_server['fixture']}"
        )
