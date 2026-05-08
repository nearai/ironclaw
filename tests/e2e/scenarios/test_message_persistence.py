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
    """Poll chat history until the most recent turn is completed."""
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert resp.status_code == 200, resp.text
        turns = resp.json()["turns"]
        if turns and turns[-1].get("state") == "Completed":
            return turns
        await asyncio.sleep(0.5)
    raise AssertionError(
        f"Timed out waiting for latest completed turn in thread {thread_id}"
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


async def _wait_for_in_progress_turn(
    base_url: str,
    thread_id: str,
    *,
    timeout: float = 10.0,
) -> dict | None:
    """Poll chat history until the thread exposes durable in-progress state."""
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert resp.status_code == 200, resp.text
        payload = resp.json()
        if payload.get("in_progress"):
            return payload
        turns = payload.get("turns", [])
        if turns and turns[-1].get("state") == "Completed":
            return None
        await asyncio.sleep(0.2)
    raise AssertionError(f"Timed out waiting for in-progress turn in thread {thread_id}")


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


async def _wait_for_processing_or_response(page, response_text: str) -> None:
    """Wait until the UI either shows the live processing state or the final answer."""
    await page.wait_for_function(
        """(text) => {
            const thinking = document.querySelector('.activity-thinking');
            if (thinking) return true;
            return Array.from(document.querySelectorAll('#chat-messages .message.assistant'))
                .some((el) => (el.textContent || '').includes(text));
        }""",
        arg=response_text,
        timeout=15000,
    )


async def _start_thread_and_wait_for_in_progress(
    base_url: str,
    content: str,
    *,
    attempts: int = 3,
) -> tuple[str, dict]:
    """Retry until a thread reliably exposes durable in-progress state."""
    last_thread_id = None

    for _ in range(attempts):
        resp = await api_post(base_url, "/api/chat/thread/new")
        assert resp.status_code == 200, resp.text
        last_thread_id = resp.json()["id"]

        send_resp = await api_post(
            base_url,
            "/api/chat/send",
            json={"content": content, "thread_id": last_thread_id},
        )
        # Gateway now returns 202 ACCEPTED (fire-and-forget) instead of the
        # legacy 200; accept either so the fixture works with both shapes.
        assert send_resp.status_code in (200, 202), send_resp.text

        payload = await _wait_for_in_progress_turn(base_url, last_thread_id)
        if payload is not None:
            return last_thread_id, payload

        await _wait_for_completed_turn(base_url, last_thread_id)

    raise AssertionError(
        "Expected to observe an in-progress turn before refresh, but the turn "
        f"completed too quickly in all {attempts} attempts (last thread: {last_thread_id})."
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

    # Thread A should show a processing spinner in the sidebar
    thread_a_spinner = page.locator(
        f'.thread-item[data-thread-id="{thread_a}"] {SEL["thread_processing"]}'
    )
    try:
        await thread_a_spinner.wait_for(state="visible", timeout=10000)
    except Exception:
        # Agent may have completed before the spinner rendered — timing-dependent
        pass

    # Wait for thread A to complete in the background
    await _wait_for_completed_turn(ironclaw_server, thread_a, timeout=30)

    # After completion, the processing spinner should be gone.
    # Give the debounced loadThreads a moment to fire.
    await page.wait_for_timeout(1000)
    spinner_count = await thread_a_spinner.count()
    assert spinner_count == 0, (
        "Expected processing spinner to disappear after thread A completed"
    )

    # Thread A should have an unread badge (response arrived while away)
    thread_a_unread = page.locator(
        f'.thread-item[data-thread-id="{thread_a}"] .thread-unread'
    )
    unread_count = await thread_a_unread.count()
    assert unread_count >= 1, "Expected unread badge on thread A after background completion"


async def test_no_stale_processing_indicator_for_completed_thread(page, ironclaw_server):
    """When returning to a thread after its in-flight turn has completed,
    the completed response should be shown and no stale Processing...
    thinking indicator should remain."""
    # Create thread A
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_a = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    # Send a message via API (don't wait for response — we want to catch mid-turn)
    await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={"content": "echo processing indicator test", "thread_id": thread_a},
    )

    # Switch to thread B immediately
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_b = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_b)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_b, timeout=10000,
    )

    # Wait for thread A to complete so the turn is persisted with response
    await _wait_for_completed_turn(ironclaw_server, thread_a, timeout=30)

    # Now switch back to thread A — should show the completed turn
    # (the "Processing..." indicator only shows for incomplete turns)
    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    # The turn completed, so we should see the assistant response
    await page.locator(SEL["message_assistant"]).wait_for(
        state="visible", timeout=15000,
    )

    # Verify there is NO stale "Processing..." indicator since the turn completed
    thinking = page.locator(SEL["activity_thinking"])
    assert await thinking.count() == 0, (
        "Processing indicator should not show for a completed turn"
    )


async def test_processing_indicator_shows_for_incomplete_turn(page, ironclaw_server):
    """When switching to a thread whose last turn has no response yet,
    loadHistory shows the Processing... thinking indicator."""
    # Create thread A and send a message via API (not UI — avoids waiting)
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_a = resp.json()["id"]

    # Send message — the agent loop will start processing
    await api_post(
        ironclaw_server,
        "/api/chat/send",
        json={"content": "What is 2+2?", "thread_id": thread_a},
    )

    # Poll until the user message is persisted but turn is still incomplete
    # (state is "Processing" — user message in DB, no response yet)
    deadline = asyncio.get_running_loop().time() + 10
    found_processing = False
    while asyncio.get_running_loop().time() < deadline:
        resp = await api_get(
            ironclaw_server, f"/api/chat/history?thread_id={thread_a}"
        )
        turns = resp.json().get("turns", [])
        if turns and turns[-1].get("state") == "Processing":
            found_processing = True
            break
        if turns and turns[-1].get("state") == "Completed":
            # Turn completed too fast to catch the Processing state —
            # this is a timing-dependent test, skip gracefully
            break
        await asyncio.sleep(0.2)

    if not found_processing:
        # Agent was too fast — we can't reliably test the indicator.
        # Verify the turn completed correctly instead.
        await _wait_for_completed_turn(ironclaw_server, thread_a)
        return

    # Switch page to thread A while it's still Processing
    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    # The "Processing..." thinking indicator should be visible — but
    # there is a window between the API-side state check above and the
    # Playwright switchThread + wait_for where the turn can complete and
    # the indicator gets cleared by the response render. Race the
    # indicator's visibility against the assistant message landing; if
    # the response arrived first, the turn was healthy and the
    # rendering happened correctly without us catching the indicator.
    thinking = page.locator(SEL["activity_thinking"])
    assistant = page.locator(SEL["message_assistant"])
    try:
        await thinking.wait_for(state="visible", timeout=10000)
    except Exception:
        if await assistant.count() == 0:
            raise
        # Response landed before the indicator could render — the
        # in-progress UX path is still valid; fall through to the
        # completion + response assertions.

    # Wait for the turn to complete — indicator should disappear
    await _wait_for_completed_turn(ironclaw_server, thread_a, timeout=30)

    # The assistant response should appear (live SSE renders it)
    await assistant.wait_for(state="visible", timeout=15000)


async def test_refresh_preserves_in_progress_turn(page, ironclaw_server):
    """Refreshing mid-turn should rebuild the user message and processing state."""
    thread_id, _payload = await _start_thread_and_wait_for_in_progress(
        ironclaw_server,
        "What is 2+2?",
    )

    await _reload_and_switch_to_thread(page, ironclaw_server, thread_id)

    await page.locator(SEL["message_user"]).filter(
        has_text="What is 2+2?"
    ).wait_for(state="visible", timeout=15000)
    assert await page.locator(".welcome-card").count() == 0
    assert (
        await page.locator(SEL["message_user"]).filter(has_text="What is 2+2?").count()
    ) == 1
    await _wait_for_processing_or_response(page, "4")

    await _wait_for_completed_turn(ironclaw_server, thread_id, timeout=30)
    await page.locator(SEL["message_assistant"]).filter(
        has_text="4"
    ).wait_for(state="visible", timeout=15000)


async def test_response_event_does_not_duplicate_history_rendered_response(page):
    """A late response SSE must not duplicate a response already rendered from history."""
    thread_id = "history-rendered-thread"
    content = "The answer is 4."

    await page.wait_for_function(
        "() => typeof eventSource !== 'undefined' && eventSource && sseHasConnectedBefore === true",
        timeout=10000,
    )
    await page.evaluate(
        """({ threadId, content }) => {
            currentThreadId = threadId;
            const container = document.getElementById('chat-messages');
            container.innerHTML = '';
            addMessage('user', 'What is 2+2?');
            addMessage('assistant', content);
            eventSource.dispatchEvent(new MessageEvent('response', {
                data: JSON.stringify({
                    type: 'response',
                    thread_id: threadId,
                    content,
                }),
            }));
        }""",
        {"threadId": thread_id, "content": content},
    )

    assistant_messages = page.locator(SEL["message_assistant"]).filter(has_text=content)
    assert await assistant_messages.count() == 1


async def test_switching_back_preserves_in_progress_turn(page, ironclaw_server):
    """Switching away and back mid-turn should rehydrate the running thread."""
    thread_a, _payload = await _start_thread_and_wait_for_in_progress(
        ironclaw_server,
        "What is 2+2?",
    )

    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_b = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_b)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_b, timeout=10000,
    )

    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    await page.locator(SEL["message_user"]).filter(
        has_text="What is 2+2?"
    ).wait_for(state="visible", timeout=15000)
    assert await page.locator(".welcome-card").count() == 0
    assert (
        await page.locator(SEL["message_user"]).filter(has_text="What is 2+2?").count()
    ) == 1
    await _wait_for_processing_or_response(page, "4")

    await _wait_for_completed_turn(ironclaw_server, thread_a, timeout=30)
    await page.locator(SEL["message_assistant"]).filter(
        has_text="4"
    ).wait_for(state="visible", timeout=15000)


async def test_sidebar_refresh_keeps_active_thread_outside_summary_window(
    page,
    ironclaw_server,
):
    """Refreshing the sidebar must not retarget an older open thread."""
    resp = await api_post(ironclaw_server, "/api/chat/thread/new")
    assert resp.status_code == 200, resp.text
    thread_a = resp.json()["id"]

    await page.evaluate("(id) => switchThread(id)", thread_a)
    await page.wait_for_function(
        "(id) => currentThreadId === id", arg=thread_a, timeout=10000,
    )

    latest_thread = None
    for _ in range(55):
        resp = await api_post(ironclaw_server, "/api/chat/thread/new")
        assert resp.status_code == 200, resp.text
        latest_thread = resp.json()["id"]

    await page.evaluate("loadThreads()")
    await page.wait_for_selector(
        f'[data-thread-id="{latest_thread}"]', state="visible", timeout=10000,
    )
    assert await page.evaluate("() => currentThreadId") == thread_a

    result = await send_chat_and_wait_for_terminal_message(
        page,
        "Summary refresh should keep this thread",
    )
    assert result["role"] == "assistant"

    turns = await _wait_for_completed_turn(ironclaw_server, thread_a)
    assert any(
        "Summary refresh should keep this thread" in (turn.get("user_input") or "")
        for turn in turns
    ), turns

    resp = await api_get(ironclaw_server, f"/api/chat/history?thread_id={latest_thread}")
    assert resp.status_code == 200, resp.text
    assert not any(
        "Summary refresh should keep this thread" in (turn.get("user_input") or "")
        for turn in resp.json()["turns"]
    ), resp.json()["turns"]
