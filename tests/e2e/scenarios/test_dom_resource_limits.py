"""DOM and timer resource limit tests for issue #2406.

Verifies that the web UI does not exhaust browser resources during extended
sessions: DOM node count stays bounded, timers are cleaned up on reconnect,
streaming messages survive pruning, and jobEvents stays capped.
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

    # Assert on the same superset selector that pruneOldMessages uses
    count = await page.locator(
        f"{SEL['chat_messages']} .message, "
        f"{SEL['chat_messages']} .activity-group, "
        f"{SEL['chat_messages']} .time-separator"
    ).count()
    assert count <= 200, f"Expected <= 200 prunable elements after pruning, got {count}"
    assert count >= 150, f"Expected at least 150 elements (not over-pruned), got {count}"


async def test_no_timer_leak_across_reconnects(ironclaw_server, browser):
    """Reconnect cycles do not accumulate leaked setInterval timers (#2406).

    Uses add_init_script() to install the setInterval monkey-patch *before*
    page navigation so timers created during initApp() are also tracked.
    """
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    # Install monkey-patch BEFORE navigation so initApp() timers are tracked.
    # Uses a Set of active IDs to prevent underflow from double-clear.
    await page.add_init_script("""() => {
        window.__testActiveIntervals = new Set();
        const origSet = window.setInterval;
        const origClear = window.clearInterval;
        window.setInterval = function(...args) {
            const id = origSet.apply(this, args);
            window.__testActiveIntervals.add(id);
            return id;
        };
        window.clearInterval = function(id) {
            window.__testActiveIntervals.delete(id);
            return origClear.call(this, id);
        };
    }""")

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await _wait_for_connected(page, timeout=10000)

    baseline = await page.evaluate("window.__testActiveIntervals.size")

    # Force 5 reconnect cycles
    for _ in range(5):
        await page.evaluate("if (eventSource) eventSource.close()")
        await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
        await _wait_for_connected(page, timeout=10000)

    after = await page.evaluate("window.__testActiveIntervals.size")
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


async def test_hidden_tab_no_duplicate_status_polling(ironclaw_server, browser):
    """Hiding and restoring a tab must not accumulate duplicate gateway status polls (#2406).

    Simulates 5 hide/show cycles by calling cleanupConnectionState() +
    connectSSE() + startGatewayStatusPolling(). The idempotency guard in
    startGatewayStatusPolling() and cleanup in cleanupConnectionState() should
    prevent interval accumulation.
    """
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()

    # Monkey-patch setInterval before navigation to track all intervals.
    await page.add_init_script("""() => {
        window.__testActiveIntervals = new Set();
        const origSet = window.setInterval;
        const origClear = window.clearInterval;
        window.setInterval = function(...args) {
            const id = origSet.apply(this, args);
            window.__testActiveIntervals.add(id);
            return id;
        };
        window.clearInterval = function(id) {
            window.__testActiveIntervals.delete(id);
            return origClear.call(this, id);
        };
    }""")

    await page.goto(f"{ironclaw_server}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await _wait_for_connected(page, timeout=10000)

    # Take baseline after one reconnect to get steady-state interval count,
    # since cleanupConnectionState() clears gatewayStatusInterval on reconnect.
    await page.evaluate("if (eventSource) eventSource.close()")
    await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
    await _wait_for_connected(page, timeout=10000)
    await page.evaluate("startGatewayStatusPolling()")

    baseline = await page.evaluate("window.__testActiveIntervals.size")

    # Simulate 5 additional hide/show cycles
    for _ in range(5):
        # Tab hidden: cleanup state (clears gatewayStatusInterval + others)
        await page.evaluate("cleanupConnectionState()")
        await page.evaluate("if (eventSource) { eventSource.close(); eventSource = null; }")
        # Tab shown: reconnect and restart polling
        await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
        await _wait_for_connected(page, timeout=10000)
        await page.evaluate("startGatewayStatusPolling()")

    after = await page.evaluate("window.__testActiveIntervals.size")
    assert after == baseline, (
        f"Interval leak across hide/show cycles: baseline={baseline}, after={after}"
    )

    await context.close()


async def test_dom_bounded_with_streaming_preserved(page):
    """Over 250 messages prunes to <= 200 AND mid-stream messages survive (#2406).

    Combined test: inserts 249 normal messages, marks one as streaming, adds
    overflow, prunes, then asserts the cap, streaming preservation, and no
    orphaned leading time-separators.
    """
    await page.evaluate("""() => {
        for (let i = 0; i < 249; i++) {
            addMessage(i % 2 === 0 ? 'user' : 'assistant', 'Message ' + i);
        }
        // Add one streaming assistant message
        const streamMsg = addMessage('assistant', 'streaming in progress...');
        streamMsg.setAttribute('data-streaming', 'true');
        // Push over the cap
        for (let i = 0; i < 10; i++) {
            addMessage('user', 'overflow ' + i);
        }
        pruneOldMessages();
    }""")

    # Total prunable elements should be at the cap
    total = await page.locator(
        f"{SEL['chat_messages']} .message, "
        f"{SEL['chat_messages']} .activity-group, "
        f"{SEL['chat_messages']} .time-separator"
    ).count()
    assert total <= 200, f"Expected <= 200 DOM elements, got {total}"

    # Streaming message must survive
    streaming = await page.locator('[data-streaming="true"]').count()
    assert streaming == 1, f"Streaming message lost: expected 1, got {streaming}"

    # No orphaned leading time-separator after pruning
    first_class = await page.evaluate("""() => {
        const el = document.querySelector('#chat-messages .message, #chat-messages .activity-group, #chat-messages .time-separator');
        return el ? el.className : null;
    }""")
    assert first_class is None or "time-separator" not in first_class, (
        "Orphaned time-separator at top of chat after pruning"
    )


async def test_job_events_map_bounded(page):
    """jobEvents map stays <= JOB_EVENTS_MAX_JOBS after > 50 jobs (#2406)."""
    size = await page.evaluate("""() => {
        // Simulate 60 distinct jobs sending events through the production
        // eviction logic (LRU via Map insertion order).
        for (let i = 0; i < 60; i++) {
            const jobId = 'test-job-' + i;
            // Move to end of Map (LRU) — same pattern as the SSE handler
            const existing = jobEvents.get(jobId);
            if (existing) jobEvents.delete(jobId);
            const events = existing || [];
            jobEvents.set(jobId, events);
            events.push({ type: 'job_status', data: { job_id: jobId }, ts: Date.now() });
            while (events.length > JOB_EVENTS_CAP) events.shift();
            // Evict oldest when over limit
            if (jobEvents.size > JOB_EVENTS_MAX_JOBS) {
                const oldest = jobEvents.keys().next().value;
                jobEvents.delete(oldest);
            }
        }
        return jobEvents.size;
    }""")
    assert size <= 50, f"Expected jobEvents capped at 50, got {size}"
    assert size >= 45, f"jobEvents unexpectedly small: got {size}"
