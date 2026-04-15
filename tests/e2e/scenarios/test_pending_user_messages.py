"""E2E coverage for issue #2409: user messages disappear after typing.

The frontend fix tracks optimistically-shown messages in a
`_pendingUserMessages` map and re-injects them when `loadHistory()`
clears the DOM before the agent loop has persisted them.
"""

import asyncio

from helpers import (
    AUTH_TOKEN,
    SEL,
    api_post,
    send_chat_and_wait_for_terminal_message,
)


async def _wait_for_connected(page, *, timeout: int = 10000) -> None:
    """Wait until the frontend reports an active SSE connection."""
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=timeout,
    )


async def _create_new_thread(page) -> str:
    """Click the new-thread button and return the new thread ID."""
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function("() => !!currentThreadId", timeout=10000)
    return await page.evaluate("() => currentThreadId")


async def test_user_message_visible_after_send(page):
    """A sent message should be visible in the chat immediately."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    await chat_input.fill("Pending message test")
    await chat_input.press("Enter")

    # The user message should appear in the DOM right away (optimistic)
    user_msg = page.locator(SEL["message_user"])
    await user_msg.first.wait_for(state="visible", timeout=5000)
    text = await user_msg.last.inner_text()
    assert "Pending message test" in text


async def test_pending_message_survives_sse_reconnect(page):
    """User message should persist across SSE reconnect before agent processes it."""
    await _wait_for_connected(page, timeout=5000)

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Inject a user message into the DOM and the pending map directly,
    # simulating the state right after sendMessage() but before the agent
    # loop persists. We avoid using the real send flow because the mock LLM
    # may respond before we can test the pending window.
    thread_id = await page.evaluate("() => currentThreadId")
    await page.evaluate(
        """(threadId) => {
            addMessage('user', 'SSE-reconnect pending test');
            if (!_pendingUserMessages.has(threadId)) {
                _pendingUserMessages.set(threadId, []);
            }
            _pendingUserMessages.get(threadId).push({
                content: 'SSE-reconnect pending test',
                timestamp: Date.now()
            });
        }""",
        thread_id,
    )

    # Verify message is in the DOM
    user_msgs = page.locator(SEL["message_user"])
    count_before = await user_msgs.count()
    assert count_before >= 1

    # Close SSE and reconnect — this triggers loadHistory() which clears DOM
    await page.evaluate("if (eventSource) eventSource.close()")
    await page.evaluate("connectSSE()")
    await _wait_for_connected(page, timeout=10000)

    # Wait for loadHistory to complete and re-render
    await page.wait_for_timeout(3000)

    # The pending message should have been re-injected
    user_msgs_after = page.locator(SEL["message_user"])
    count_after = await user_msgs_after.count()
    assert count_after >= 1, "Pending user message should survive SSE reconnect"

    # Verify the specific message text is present
    all_text = await page.evaluate(
        """() => Array.from(document.querySelectorAll('#chat-messages .message.user'))
               .map(el => el.innerText)"""
    )
    assert any("SSE-reconnect pending test" in t for t in all_text), (
        f"Expected pending message in DOM, got: {all_text}"
    )


async def test_pending_message_cleared_after_response(page):
    """After the agent responds, pending messages should be cleared."""
    # Send a real message and wait for the full round-trip
    result = await send_chat_and_wait_for_terminal_message(page, "Clear pending test")
    assert result["role"] == "assistant"

    # The pending map should be empty for this thread
    pending_count = await page.evaluate(
        """() => {
            const pending = _pendingUserMessages.get(currentThreadId);
            return pending ? pending.length : 0;
        }"""
    )
    assert pending_count == 0, (
        f"Expected pending messages to be cleared after response, got {pending_count}"
    )


async def test_no_duplicate_after_history_load(page):
    """A message that's in DB should not be duplicated by the pending re-inject."""
    # Send a message and wait for the full round-trip (message is now in DB)
    result = await send_chat_and_wait_for_terminal_message(page, "Duplicate check")
    assert result["role"] == "assistant"

    user_count_before = await page.locator(SEL["message_user"]).count()

    # Force a history reload (simulates what happens on thread switch back)
    await page.evaluate("loadHistory()")
    await page.wait_for_timeout(2000)

    user_count_after = await page.locator(SEL["message_user"]).count()
    assert user_count_after == user_count_before, (
        f"Expected no duplicate messages: before={user_count_before}, after={user_count_after}"
    )


async def test_welcome_card_hidden_when_pending(page):
    """Welcome card should not show when there are pending messages."""
    # Create a new empty thread
    new_thread = await _create_new_thread(page)
    await page.wait_for_timeout(1000)

    # Inject a pending message without actually sending (to avoid triggering LLM)
    await page.evaluate(
        """(threadId) => {
            addMessage('user', 'Welcome card suppression test');
            if (!_pendingUserMessages.has(threadId)) {
                _pendingUserMessages.set(threadId, []);
            }
            _pendingUserMessages.get(threadId).push({
                content: 'Welcome card suppression test',
                timestamp: Date.now()
            });
            // Trigger a history reload to test the welcome card logic
            loadHistory();
        }""",
        new_thread,
    )
    await page.wait_for_timeout(2000)

    # Welcome card should NOT be visible because there's a pending message
    welcome_visible = await page.evaluate(
        """() => {
            const card = document.querySelector('.welcome-card');
            return card && card.offsetParent !== null;
        }"""
    )
    assert not welcome_visible, "Welcome card should be hidden when pending messages exist"


async def test_message_persists_across_page_reload(page, ironclaw_server):
    """After full round-trip, message survives a page reload (DB persistence)."""
    result = await send_chat_and_wait_for_terminal_message(page, "Reload persistence test")
    assert result["role"] == "assistant"

    # Reload the page
    await page.reload(wait_until="networkidle", timeout=15000)
    await page.locator(SEL["auth_screen"]).wait_for(state="hidden", timeout=10000)
    await page.wait_for_timeout(3000)

    # The message should be loaded from DB
    all_text = await page.evaluate(
        """() => Array.from(document.querySelectorAll('#chat-messages .message.user'))
               .map(el => el.innerText)"""
    )
    assert any("Reload persistence test" in t for t in all_text), (
        f"Expected message after reload, got: {all_text}"
    )
