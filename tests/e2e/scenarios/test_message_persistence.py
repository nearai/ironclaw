"""E2E tests for user message persistence.

Verifies that user messages and assistant responses survive a full page
reload — the round-trip from the database.
"""

import asyncio
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from helpers import (
    AUTH_TOKEN,
    SEL,
    api_get,
    api_post,
    send_chat_and_wait_for_terminal_message,
)


async def _wait_for_completed_turn(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 20.0,
) -> list:
    """Poll chat history until a completed turn appears."""
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert resp.status_code == 200, resp.text
        turns = resp.json()["turns"]
        if any(t.get("state") == "Completed" for t in turns):
            return turns
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for completed turn in thread {thread_id}"
    )


async def test_message_persists_across_page_reload(page, ironclaw_server):
    """Happy-path: send a message, reload the page, both user message and
    assistant response survive the full round-trip from the database."""
    # Create an isolated thread
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    # Switch the page to this thread
    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id",
        arg=thread_id,
        timeout=10000,
    )

    # Send a message and wait for the assistant response
    result = await send_chat_and_wait_for_terminal_message(page, "What is 2+2?")
    assert result["role"] == "assistant"
    assert "4" in result["text"], result

    # Poll history API until the turn is completed (avoids flaky fixed sleep)
    await _wait_for_completed_turn(ironclaw_server, thread_id)

    # Reload the page — clears all client-side state (JS vars, SSE, DOM)
    await page.goto(
        f"{ironclaw_server}/?token={AUTH_TOKEN}",
        timeout=15000,
    )
    await page.wait_for_selector(SEL["auth_screen"], state="hidden", timeout=10000)
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )

    # Switch back to the original thread
    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id",
        arg=thread_id,
        timeout=10000,
    )

    # Verify user message survived the reload
    await page.locator(SEL["message_user"]).filter(
        has_text="What is 2+2?"
    ).wait_for(state="visible", timeout=15000)

    # Verify assistant response survived the reload
    await page.locator(SEL["message_assistant"]).filter(
        has_text="4"
    ).wait_for(state="visible", timeout=15000)

    # Cross-check via API: exactly 1 user turn with a response
    resp = await api_get(
        ironclaw_server,
        f"/api/chat/history?thread_id={thread_id}",
    )
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]
    user_turns = [t for t in turns if t.get("user_input")]
    assert len(user_turns) == 1, (
        f"Expected exactly 1 user turn, got {len(user_turns)}: {user_turns}"
    )
    assert "2+2" in user_turns[0]["user_input"] or "2 + 2" in user_turns[0]["user_input"]
    assert user_turns[0].get("response") and "4" in user_turns[0]["response"]
    assert user_turns[0]["state"] == "Completed", user_turns[0]["state"]
