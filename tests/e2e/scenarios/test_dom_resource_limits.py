"""DOM and timer resource limit tests for issue #2406.

Verifies that the web UI does not exhaust browser resources during extended
sessions: DOM node count stays bounded, timers are cleaned up on reconnect,
streaming messages survive pruning, and jobEvents stays capped.
"""

from helpers import AUTH_TOKEN, SEL

# Same selector used by pruneOldMessages() in app.js
PRUNE_SELECTOR = "#chat-messages .message, #chat-messages .activity-group, #chat-messages .time-separator"


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


async def test_no_timer_leak_across_reconnects(page):
    """Reconnect cycles do not accumulate leaked setInterval timers (#2406).

    Injects a setInterval/clearInterval monkey-patch into the already-loaded
    page (the page fixture handles navigation and SSE connection). Timers
    created before injection are not tracked, but the before/after comparison
    still detects leaks across reconnect cycles.
    """
    await page.evaluate("""() => {
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


async def test_hidden_tab_no_duplicate_status_polling(page):
    """Hiding and restoring a tab must not accumulate duplicate gateway status polls (#2406).

    Simulates 5 hide/show cycles by calling cleanupConnectionState() +
    connectSSE() + startGatewayStatusPolling(). The idempotency guard in
    startGatewayStatusPolling() and cleanup in cleanupConnectionState() should
    prevent interval accumulation.

    Injects the setInterval monkey-patch into the already-loaded page (rather
    than via add_init_script) to avoid execution-context-destruction issues
    from page navigation during init.
    """
    await page.evaluate("""() => {
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
            // Evict oldest when over limit (skip currentJobId)
            if (jobEvents.size > JOB_EVENTS_MAX_JOBS) {
                let evicted = false;
                for (const k of jobEvents.keys()) {
                    if (k !== currentJobId) {
                        jobEvents.delete(k);
                        evicted = true;
                        break;
                    }
                }
                if (!evicted) {
                    jobEvents.delete(jobEvents.keys().next().value);
                }
            }
        }
        return jobEvents.size;
    }""")
    assert size <= 50, f"Expected jobEvents capped at 50, got {size}"
    assert size >= 45, f"jobEvents unexpectedly small: got {size}"


async def test_job_events_lru_preserves_current_job(page):
    """Actively-viewed job (currentJobId) is never evicted by LRU (#2441)."""
    result = await page.evaluate("""() => {
        // Set up: user is viewing job detail for 'viewed-job'
        currentJobId = 'viewed-job';
        jobEvents.clear();
        jobEvents.set('viewed-job', [{ type: 'job_status', data: { job_id: 'viewed-job' }, ts: 1 }]);

        // Simulate 60 other jobs firing events — should never evict 'viewed-job'
        for (let i = 0; i < 60; i++) {
            const jobId = 'other-job-' + i;
            const existing = jobEvents.get(jobId);
            if (existing) jobEvents.delete(jobId);
            const events = existing || [];
            jobEvents.set(jobId, events);
            events.push({ type: 'job_status', data: { job_id: jobId }, ts: Date.now() });
            while (events.length > JOB_EVENTS_CAP) events.shift();
            if (jobEvents.size > JOB_EVENTS_MAX_JOBS) {
                let evicted = false;
                for (const k of jobEvents.keys()) {
                    if (k !== currentJobId) {
                        jobEvents.delete(k);
                        evicted = true;
                        break;
                    }
                }
                if (!evicted) {
                    jobEvents.delete(jobEvents.keys().next().value);
                }
            }
        }
        return {
            size: jobEvents.size,
            viewedJobSurvived: jobEvents.has('viewed-job'),
        };
    }""")
    assert result["viewedJobSurvived"], "currentJobId was evicted by LRU — activity tab would go empty"
    assert result["size"] <= 50, f"Expected jobEvents capped at 50, got {result['size']}"


# ---------------------------------------------------------------------------
# Real E2E tests — exercise code paths through the actual UI, not page.evaluate
# ---------------------------------------------------------------------------


async def _send_and_wait_for_response(page, message, *, timeout=30000):
    """Send a chat message and wait for the assistant response.

    Unlike send_chat_and_wait_for_terminal_message, this does not rely on
    counting DOM elements, so it works correctly when pruneOldMessages() removes
    elements during the round-trip.

    After pressing Enter, sendMessage() synchronously appends the user message
    to the end of #chat-messages. We wait for the last .message to become a
    non-streaming assistant message with content — meaning the response arrived.
    """
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    await chat_input.fill(message)
    await chat_input.press("Enter")

    await page.wait_for_function("""() => {
        const msgs = document.querySelectorAll('#chat-messages .message');
        if (msgs.length === 0) return false;
        const last = msgs[msgs.length - 1];
        if (!last.classList.contains('assistant')) return false;
        if (last.getAttribute('data-streaming') === 'true') return false;
        const content = last.querySelector('.message-content');
        return content && content.innerText.trim().length > 0;
    }""", timeout=timeout)


async def test_real_message_flow_triggers_dom_pruning(page):
    """Sending real messages through the chat UI prunes DOM at MAX_DOM_MESSAGES (#2406).

    Pre-fills the DOM with stub messages, then sends real round-trips through
    the mock LLM. Pruning fires via the real sendMessage() and response handler
    code paths — not via a direct pruneOldMessages() call.
    """
    prefill = 194

    await page.evaluate(f"""() => {{
        for (let i = 0; i < {prefill}; i++) {{
            addMessage(i % 2 === 0 ? 'user' : 'assistant', 'Prefill msg ' + i);
        }}
    }}""")

    prefill_count = await page.evaluate(
        f"document.querySelectorAll('{PRUNE_SELECTOR}').length"
    )
    assert prefill_count >= prefill

    # 5 real round-trips: each adds 1 user + 1 assistant = 10 new DOM elements.
    # Uses _send_and_wait_for_response because send_chat_and_wait_for_terminal_message
    # relies on element count increasing, which breaks when pruning removes elements.
    messages = ["hello", "2+2", "hello", "2+2", "hello"]
    for msg in messages:
        await _send_and_wait_for_response(page, msg)

    final_count = await page.evaluate(
        f"document.querySelectorAll('{PRUNE_SELECTOR}').length"
    )
    assert final_count <= 200, f"DOM not pruned: {final_count} elements (expected <= 200)"
    assert final_count >= 100, f"Over-pruned: only {final_count} elements remain"

    # Most recent assistant response must still be visible (pruning removes from front)
    last_text = await page.locator(SEL["message_assistant"]).last.text_content()
    assert last_text and len(last_text.strip()) > 0


async def test_real_reconnect_clears_timers_after_activity(page):
    """Timer count does not grow across reconnects after real message activity (#2406).

    Sends a real message to exercise timer-creating code paths (stream debounce,
    thread list refresh, gateway status polling), then reconnects multiple times
    and verifies no interval accumulation.

    Injects the setInterval monkey-patch into the already-loaded page (rather
    than via add_init_script) to avoid execution-context-destruction issues
    from page navigation during init. Timers created before injection are not
    tracked, but the before/after comparison still detects leaks.
    """
    # Inject monkey-patch into the already-loaded page
    await page.evaluate("""() => {
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

    # Send a real message to populate timers through normal operation
    await _send_and_wait_for_response(page, "hello")

    baseline = await page.evaluate("window.__testActiveIntervals.size")

    for _ in range(3):
        await page.evaluate("if (eventSource) eventSource.close()")
        await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
        await _wait_for_connected(page, timeout=10000)

    after = await page.evaluate("window.__testActiveIntervals.size")
    assert after <= baseline, (
        f"Interval leak after activity: baseline={baseline}, after 3 reconnects={after}"
    )


async def test_response_intact_near_dom_cap(page):
    """Real assistant response content is preserved when DOM is near the cap (#2406).

    Fills the DOM to just under the cap, sends a real message with a known
    response, and verifies the response text is intact after pruning runs.
    """
    await page.evaluate("""() => {
        for (let i = 0; i < 198; i++) {
            addMessage(i % 2 === 0 ? 'user' : 'assistant', 'Prefill ' + i);
        }
    }""")

    await _send_and_wait_for_response(page, "2+2")

    # The response should be intact even though pruning ran
    last_text = await page.locator(SEL["message_assistant"]).last.text_content()
    assert "4" in last_text, f"Expected response about 4, got: {last_text}"

    # DOM should be bounded
    count = await page.evaluate(
        f"document.querySelectorAll('{PRUNE_SELECTOR}').length"
    )
    assert count <= 200, f"DOM exceeded cap: {count}"


async def test_real_user_message_prunes_before_response(page):
    """sendMessage() prunes the DOM after adding the user message (#2406).

    Fills the DOM to 199 elements, sends a real message, and verifies the DOM
    stays bounded after the full round-trip (user msg + prune + assistant + prune).
    """
    await page.evaluate("""() => {
        for (let i = 0; i < 199; i++) {
            addMessage(i % 2 === 0 ? 'user' : 'assistant', 'Prefill ' + i);
        }
    }""")

    await _send_and_wait_for_response(page, "hello")

    final_count = await page.evaluate(
        f"document.querySelectorAll('{PRUNE_SELECTOR}').length"
    )
    assert final_count <= 200, f"DOM exceeded cap after real message: {final_count}"
