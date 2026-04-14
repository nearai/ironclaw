"""DOM and timer resource limit tests for issue #2406.

Verifies that the web UI does not exhaust browser resources during extended
sessions: DOM node count stays bounded, timers are cleaned up on reconnect,
and streaming messages survive pruning.
"""

from helpers import AUTH_TOKEN, SEL


async def _wait_for_connected(page, *, timeout: int = 10000) -> None:
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=timeout,
    )


async def test_dom_pruned_after_many_messages(page):
    """DOM stays bounded at MAX_DOM_MESSAGES after many insertions (#2406)."""
    # Inject 250 messages directly (faster than round-tripping through LLM)
    await page.evaluate("""() => {
        for (let i = 0; i < 250; i++) {
            addMessage(i % 2 === 0 ? 'user' : 'assistant', 'Message ' + i);
        }
        pruneOldMessages();
    }""")

    count = await page.locator(f"{SEL['chat_messages']} .message").count()
    assert count <= 200, f"Expected <= 200 DOM messages after pruning, got {count}"
    assert count >= 100, f"Expected at least 100 DOM messages (not over-pruned), got {count}"


async def test_no_timer_leak_across_reconnects(ironclaw_server, browser):
    """Reconnect cycles do not accumulate leaked setInterval timers (#2406).

    Uses add_init_script() to install the setInterval monkey-patch *before*
    page navigation so timers created during initApp() are also tracked.
    """
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    # Install monkey-patch BEFORE navigation so initApp() timers are tracked
    await page.add_init_script("""() => {
        window.__testIntervalCount = 0;
        const origSet = window.setInterval;
        const origClear = window.clearInterval;
        window.setInterval = function(...args) {
            const id = origSet.apply(this, args);
            window.__testIntervalCount++;
            return id;
        };
        window.clearInterval = function(id) {
            if (id != null) window.__testIntervalCount--;
            return origClear.call(this, id);
        };
    }""")

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await _wait_for_connected(page, timeout=10000)

    baseline = await page.evaluate("window.__testIntervalCount")

    # Force 5 reconnect cycles
    for _ in range(5):
        await page.evaluate("if (eventSource) eventSource.close()")
        await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
        await _wait_for_connected(page, timeout=10000)

    after = await page.evaluate("window.__testIntervalCount")
    # cleanupConnectionState() clears all connection-scoped intervals (including
    # gatewayStatusInterval), so no net new intervals should accumulate.
    assert after <= baseline, (
        f"Interval leak detected: baseline={baseline}, after 5 reconnects={after}"
    )

    await context.close()


async def test_prune_preserves_streaming_message(page):
    """pruneOldMessages must not remove a message with data-streaming=true (#2406)."""
    # Fill the DOM to just under the cap
    await page.evaluate("""() => {
        for (let i = 0; i < 199; i++) {
            addMessage('assistant', 'msg ' + i);
        }
    }""")

    # Mark the last assistant message as actively streaming
    await page.evaluate("""() => {
        const msgs = document.querySelectorAll('#chat-messages .message.assistant');
        msgs[msgs.length - 1].setAttribute('data-streaming', 'true');
    }""")

    # Push over the cap and prune
    await page.evaluate("""() => {
        for (let i = 0; i < 10; i++) {
            addMessage('user', 'overflow ' + i);
        }
        pruneOldMessages();
    }""")

    streaming_count = await page.locator('[data-streaming="true"]').count()
    assert streaming_count == 1, (
        f"Streaming message was pruned: expected 1 element with data-streaming, got {streaming_count}"
    )
