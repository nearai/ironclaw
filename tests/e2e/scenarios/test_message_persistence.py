"""E2E tests for message persistence and history rendering.

Verifies that user messages, assistant responses, and tool call cards
survive page reloads and thread switches — the round-trip from the database.
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


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


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


async def _wait_for_tool_in_history(
    base_url: str,
    thread_id: str,
    tool_name: str,
    *,
    timeout: float = 30.0,
) -> list:
    """Poll chat history until a turn with the named tool call appears."""
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert resp.status_code == 200, resp.text
        turns = resp.json()["turns"]
        for t in turns:
            for tc in t.get("tool_calls", []):
                if tc.get("name") == tool_name and tc.get("has_result"):
                    return turns
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for tool '{tool_name}' in thread {thread_id}"
    )


async def _reload_and_switch_to_thread(page, ironclaw_server, thread_id):
    """Reload the page and navigate back to the given thread."""
    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}", timeout=15000)
    await page.wait_for_selector(SEL["auth_screen"], state="hidden", timeout=10000)
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=10000,
    )
    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_id, timeout=10000,
    )


# ---------------------------------------------------------------------------
# Message persistence
# ---------------------------------------------------------------------------


async def test_message_persists_across_page_reload(page, ironclaw_server):
    """Happy-path: send a message, reload the page, both user message and
    assistant response survive the full round-trip from the database."""
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_id, timeout=10000,
    )

    result = await send_chat_and_wait_for_terminal_message(page, "What is 2+2?")
    assert result["role"] == "assistant"
    assert "4" in result["text"], result

    await _wait_for_completed_turn(ironclaw_server, thread_id)
    await _reload_and_switch_to_thread(page, ironclaw_server, thread_id)

    await page.locator(SEL["message_user"]).filter(
        has_text="What is 2+2?"
    ).wait_for(state="visible", timeout=15000)

    await page.locator(SEL["message_assistant"]).filter(
        has_text="4"
    ).wait_for(state="visible", timeout=15000)

    resp = await api_get(ironclaw_server, f"/api/chat/history?thread_id={thread_id}")
    assert resp.status_code == 200, resp.text
    turns = resp.json()["turns"]
    user_turns = [t for t in turns if t.get("user_input")]
    assert len(user_turns) == 1, (
        f"Expected exactly 1 user turn, got {len(user_turns)}: {user_turns}"
    )
    assert "2+2" in user_turns[0]["user_input"] or "2 + 2" in user_turns[0]["user_input"]
    assert user_turns[0].get("response") and "4" in user_turns[0]["response"]
    assert user_turns[0]["state"] == "Completed", user_turns[0]["state"]


# ---------------------------------------------------------------------------
# Tool call activity cards in history
# ---------------------------------------------------------------------------


async def test_tool_calls_rendered_as_activity_cards_after_reload(page, ironclaw_server):
    """After a tool call completes and the page reloads, the most recent turn
    should show rich activity cards (not the flat summary)."""
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_id, timeout=10000,
    )

    result = await send_chat_and_wait_for_terminal_message(page, "echo hello world")
    assert result["role"] == "assistant"

    await _wait_for_tool_in_history(ironclaw_server, thread_id, "echo")
    await _reload_and_switch_to_thread(page, ironclaw_server, thread_id)

    # Verify rich activity group (not flat summary)
    activity_group = page.locator(SEL["activity_group"])
    await activity_group.wait_for(state="visible", timeout=15000)

    # Verify the echo tool card exists with success status
    echo_card = page.locator(f'{SEL["activity_tool_card"]}[data-tool-name="echo"]')
    await echo_card.wait_for(state="attached", timeout=5000)
    assert await echo_card.get_attribute("data-status") == "success"


async def test_tool_calls_expandable_after_reload(page, ironclaw_server):
    """Activity cards from history should be expandable — click summary to
    show cards, click card header to show output."""
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_id = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_id)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_id, timeout=10000,
    )

    result = await send_chat_and_wait_for_terminal_message(page, "echo expand test")
    assert result["role"] == "assistant"

    await _wait_for_tool_in_history(ironclaw_server, thread_id, "echo")
    await _reload_and_switch_to_thread(page, ironclaw_server, thread_id)

    # Cards container should start hidden (group is collapsed)
    cards_container = page.locator(SEL["activity_cards_container"])
    await cards_container.wait_for(state="attached", timeout=10000)
    assert await cards_container.is_hidden()

    # Click summary to expand
    await page.locator(SEL["activity_summary"]).click()
    await cards_container.wait_for(state="visible", timeout=5000)

    # Click card header to expand body
    card_selector = f'{SEL["activity_tool_card"]}[data-tool-name="echo"]'
    await page.locator(f"{card_selector} .activity-tool-header").click()

    await page.wait_for_function(
        """(sel) => {
            const el = document.querySelector(sel);
            return el && el.classList.contains('expanded');
        }""",
        arg=f"{card_selector} {SEL['activity_tool_body']}",
        timeout=5000,
    )


# ---------------------------------------------------------------------------
# Thread processing indicator
# ---------------------------------------------------------------------------


async def test_background_thread_shows_processing_indicator(page, ironclaw_server):
    """When a thread is processing in the background, its sidebar entry should
    show a processing spinner. The spinner disappears when processing completes."""
    # Create thread A and send a message that triggers a tool call
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_a = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    # Send a message and wait for assistant response (turn completes)
    result = await send_chat_and_wait_for_terminal_message(page, "echo background test")
    assert result["role"] == "assistant"

    # Now send another message on thread A, then immediately switch away
    # before it completes, to catch the processing state
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Immediately switch to a new thread B
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_b = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_b)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_b, timeout=10000,
    )

    # Wait for thread A to complete in the background
    await _wait_for_completed_turn(ironclaw_server, thread_a, timeout=30)

    # After completion, the processing spinner should be gone.
    # Give the debounced loadThreads a moment to fire.
    await page.wait_for_timeout(1000)

    # Thread A should have an unread badge (response arrived while away)
    thread_a_unread = page.locator(f'.thread-item .thread-unread')
    unread_count = await thread_a_unread.count()
    assert unread_count >= 1, "Expected unread badge on thread A after background completion"
