"""E2E coverage for issue #2409: user messages disappear after typing.

The frontend fix tracks optimistically-shown messages in a
`_pendingUserMessages` map and re-injects them when `loadHistory()`
clears the DOM before the agent loop has persisted them.
"""

from helpers import (
    SEL,
    send_chat_and_wait_for_terminal_message,
)


async def _wait_for_connected(page, *, timeout: int = 10000) -> None:
    """Wait until the frontend reports an active SSE connection."""
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=timeout,
    )


async def _create_new_thread(page) -> str:
    """Click the new-thread button and return the new thread ID.

    `createNewThread()` fires an API call that updates `currentThreadId`
    asynchronously. The previous `!!currentThreadId` wait returned as soon as
    any thread id was set — which was usually true *before* the click (the
    auth flow already pins us to the assistant thread). The result was that
    callers got back the pre-click id while the actual new thread id
    landed moments later, silently de-syncing any state keyed on the
    returned id. Wait for the id to change instead.
    """
    previous = await page.evaluate("() => currentThreadId || null")
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function(
        "(prev) => !!currentThreadId && currentThreadId !== prev",
        arg=previous,
        timeout=10000,
    )
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
    """End-to-end race: real sendMessage() → forced reconnect before persist.

    Drives the production code path:
      1. Stub apiFetch so POST /api/chat/send hangs (message stays in
         "sent but not yet persisted" state — exactly the window the fix
         protects).
      2. Trigger sendMessage() through the real UI so production code
         populates _pendingUserMessages.
      3. Force SSE reconnect, which triggers loadHistory() — the path that
         used to clobber the optimistic DOM entry.
      4. Assert the message is still visible after re-injection.
    """
    await _wait_for_connected(page, timeout=5000)

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Stub apiFetch so /api/chat/send never resolves; everything else passes
    # through. This keeps the pending entry alive across the reconnect.
    await page.evaluate("""() => {
        window._testHistoryLoadCount = 0;
        const origLoadHistory = window.loadHistory;
        window.loadHistory = function() {
            const result = origLoadHistory.apply(this, arguments);
            Promise.resolve(result).then(() => { window._testHistoryLoadCount++; });
            return result;
        };
        const origApiFetch = window.apiFetch;
        window._testOrigApiFetch = origApiFetch;
        window.apiFetch = function(path, options) {
            if (typeof path === 'string' && path.startsWith('/api/chat/send')) {
                // Hang forever so the message stays in pending state.
                return new Promise(() => {});
            }
            return origApiFetch.apply(this, arguments);
        };
    }""")

    try:
        # Drive the real send path through the UI.
        unique_msg = "SSE-reconnect race test 12345"
        await chat_input.fill(unique_msg)
        await chat_input.press("Enter")

        # The production code should have:
        #   (a) optimistically rendered the user message
        #   (b) populated _pendingUserMessages for the current thread
        await page.wait_for_function(
            f"""() => {{
                const msgs = Array.from(document.querySelectorAll(
                    '#chat-messages .message.user'
                )).map(el => el.innerText);
                if (!msgs.some(t => t.includes({unique_msg!r}))) return false;
                const pending = _pendingUserMessages.get(currentThreadId);
                return pending && pending.some(p => p.content === {unique_msg!r});
            }}""",
            timeout=5000,
        )

        load_count_before = await page.evaluate("() => window._testHistoryLoadCount")

        # Force SSE reconnect — triggers loadHistory() which clears+rebuilds DOM.
        # The SSE open handler only reloads history when disconnectMs exceeds
        # SSE_RELOAD_THRESHOLD_MS (a perf optimization for brief tab-visibility
        # reconnects). Age `_sseDisconnectedAt` past that threshold so the
        # test reconnect still exercises the reload+re-inject path.
        await page.evaluate(
            "() => { _sseDisconnectedAt = Date.now() - (SSE_RELOAD_THRESHOLD_MS + 5000); }"
        )
        await page.evaluate("if (eventSource) eventSource.close()")
        await page.evaluate("connectSSE()")

        await page.wait_for_function(
            f"() => window._testHistoryLoadCount > {load_count_before}",
            timeout=10000,
        )
        await page.wait_for_timeout(300)

        # Re-injection must keep the message visible.
        all_text = await page.evaluate(
            """() => Array.from(document.querySelectorAll('#chat-messages .message.user'))
                   .map(el => el.innerText)"""
        )
        assert any(unique_msg in t for t in all_text), (
            f"Expected pending message in DOM after reconnect, got: {all_text}"
        )
    finally:
        # Restore apiFetch so other tests in the same browser context aren't poisoned.
        await page.evaluate(
            "() => { if (window._testOrigApiFetch) { window.apiFetch = window._testOrigApiFetch; } }"
        )


async def test_pending_entry_cleared_when_send_fails(page):
    """If POST /api/chat/send rejects (network error, 5xx), the optimistic
    pending entry must be removed so a subsequent thread switch / loadHistory
    does not re-inject a message the server never accepted.
    """
    await _wait_for_connected(page, timeout=5000)

    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    thread_id = await page.evaluate("() => currentThreadId")
    assert thread_id, "expected an active thread before send"

    pending_before = await page.evaluate(
        """(tid) => {
            const arr = _pendingUserMessages.get(tid);
            return arr ? arr.length : 0;
        }""",
        thread_id,
    )

    # Stub apiFetch so /api/chat/send rejects with a synthetic 500.
    await page.evaluate("""() => {
        const origApiFetch = window.apiFetch;
        window._testOrigApiFetch = origApiFetch;
        window.apiFetch = function(path, options) {
            if (typeof path === 'string' && path.startsWith('/api/chat/send')) {
                const err = new Error('synthetic test failure');
                err.status = 500;
                return Promise.reject(err);
            }
            return origApiFetch.apply(this, arguments);
        };
    }""")

    try:
        unique_msg = "send-failure cleanup test 67890"
        await chat_input.fill(unique_msg)
        await chat_input.press("Enter")

        # Wait until the catch handler has had a chance to run.
        await page.wait_for_function(
            f"""(args) => {{
                const arr = _pendingUserMessages.get(args.tid);
                const count = arr ? arr.length : 0;
                // Either the entry was removed (count back to baseline)
                // or it was added then pruned by .catch().
                if (count !== args.before) return false;
                // Also confirm no orphan entry with our content remains.
                if (arr && arr.some(p => p.content === args.msg)) return false;
                return true;
            }}""",
            arg={"tid": thread_id, "before": pending_before, "msg": unique_msg},
            timeout=5000,
        )
    finally:
        await page.evaluate(
            "() => { if (window._testOrigApiFetch) { window.apiFetch = window._testOrigApiFetch; } }"
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
                id: Date.now(),
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

    # Reload the page (use "domcontentloaded" — SSE keeps connection open so
    # "networkidle" never fires)
    await page.reload(wait_until="domcontentloaded", timeout=15000)
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
