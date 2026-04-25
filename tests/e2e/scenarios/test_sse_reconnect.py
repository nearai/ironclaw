"""SSE and connectivity end-to-end coverage for issue #1784."""

import asyncio

from helpers import (
    AUTH_TOKEN,
    SEL,
    api_get,
    api_post,
    send_chat_and_wait_for_terminal_message,
    sse_stream,
    wait_for_sse_comment,
)


async def _open_gateway_page(browser, base_url: str):
    """Open an authenticated page against a specific gateway base URL."""
    context = await browser.new_context(viewport={"width": 1280, "height": 720})
    page = await context.new_page()
    await page.goto(f"{base_url}/?token={AUTH_TOKEN}")
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await page.wait_for_function("() => !!currentThreadId", timeout=15000)
    await _wait_for_connected(page, timeout=15000)
    return context, page


async def _wait_for_connected(page, *, timeout: int = 10000) -> None:
    """Wait until the frontend reports an active SSE connection.

    Uses the ``sseHasConnectedBefore`` JS flag which is set to ``true``
    inside ``EventSource.onopen``.  This is more reliable than checking
    CSS state on ``#sse-dot`` because the dot starts without the
    ``disconnected`` class before SSE even connects.
    """
    await page.wait_for_function(
        "() => typeof sseHasConnectedBefore !== 'undefined' && sseHasConnectedBefore === true",
        timeout=timeout,
    )


async def _wait_for_last_event_id(page, *, timeout: int = 15000) -> str:
    """Wait until the browser has recorded at least one SSE event ID."""
    await page.wait_for_function(
        "() => !!(window.__e2e && window.__e2e.lastSseEventId)",
        timeout=timeout,
    )
    return await page.evaluate("() => window.__e2e.lastSseEventId")


async def _wait_for_turn_in_history(base_url: str, thread_id: str, expected_response: str) -> None:
    """Poll chat history until the expected assistant response is persisted."""
    deadline = asyncio.get_running_loop().time() + 20
    while asyncio.get_running_loop().time() < deadline:
        response = await api_get(base_url, f"/api/chat/history?thread_id={thread_id}")
        assert response.status_code == 200, response.text
        turns = response.json()["turns"]
        if any((turn.get("response") or "") == expected_response for turn in turns):
            return
        await asyncio.sleep(0.5)
    raise AssertionError(f"Timed out waiting for history to contain response: {expected_response}")


async def test_sse_status_shows_connected(page):
    """SSE dot should show connected state after page load."""
    dot = page.locator("#sse-dot")
    cls = await dot.get_attribute("class") or ""
    assert "disconnected" not in cls, f"Expected connected dot, got class='{cls}'"


async def test_sse_reconnect_after_disconnect(page):
    """After programmatic disconnect, SSE should reconnect."""
    await _wait_for_connected(page, timeout=5000)
    await page.evaluate("if (eventSource) eventSource.close()")
    # Reset the flag so _wait_for_connected can detect the new onopen.
    # The history-reload path (sseHasConnectedBefore=true on reconnect)
    # is covered by test_sse_reconnect_preserves_chat_history.
    await page.evaluate("sseHasConnectedBefore = false; connectSSE()")
    await _wait_for_connected(page, timeout=10000)


async def test_sse_reconnect_preserves_chat_history(page):
    """Messages sent before disconnect should still be visible after reconnect."""
    await send_chat_and_wait_for_terminal_message(page, "Hello")
    await page.wait_for_timeout(3000)

    await page.evaluate("if (eventSource) eventSource.close()")
    await page.evaluate("connectSSE()")
    await _wait_for_connected(page, timeout=10000)
    await page.wait_for_timeout(3000)

    total_messages = await page.locator("#chat-messages .message").count()
    assert total_messages >= 1, "Expected at least 1 message after reconnect history load"

    user_msgs = await page.locator(SEL["message_user"]).count()
    assert user_msgs >= 1, "User message should be preserved after reconnect"


async def test_tab_switch_does_not_reload_chat_history(page):
    """Regression for #2404: brief tab hide/show must not re-render the chat DOM.

    Before the fix, every ``visibilitychange`` round-trip triggered
    ``loadHistory()`` in ``onopen`` unconditionally, wiping ``#chat-messages``
    and losing scroll position. The fix time-gates the reload on
    ``_sseDisconnectedAt``.  This test drives the real ``visibilitychange``
    handler (rather than the helper it consults) so that a future refactor
    of ``onopen`` cannot silently reintroduce the regression.
    """
    await send_chat_and_wait_for_terminal_message(page, "Hello")

    # Tag the rendered user message with a sentinel. ``loadHistory()`` clears
    # ``#chat-messages`` and re-renders from scratch, which would strip this
    # attribute — its presence after the tab switch proves the DOM survived.
    await page.evaluate(
        """
        () => {
          const msg = document.querySelector('#chat-messages .message.user');
          if (msg) msg.setAttribute('data-e2e-preserved', 'yes');
        }
        """
    )
    tagged = await page.locator('[data-e2e-preserved="yes"]').count()
    assert tagged == 1, "precondition: exactly one tagged user message"

    history_requests: list[str] = []

    def on_request(request) -> None:
        if "/api/chat/history" in request.url:
            history_requests.append(request.url)

    page.on("request", on_request)
    try:
        await page.evaluate(
            """
            () => {
              Object.defineProperty(document, 'hidden', {
                configurable: true, get: () => true,
              });
              document.dispatchEvent(new Event('visibilitychange'));
            }
            """
        )
        await page.wait_for_function("() => eventSource === null", timeout=5000)
        await page.evaluate(
            """
            () => {
              Object.defineProperty(document, 'hidden', {
                configurable: true, get: () => false,
              });
              document.dispatchEvent(new Event('visibilitychange'));
            }
            """
        )
        await page.wait_for_function(
            "() => eventSource && eventSource.readyState === 1",
            timeout=10000,
        )
        # Give any deferred history reload time to fire before asserting none did.
        await page.wait_for_timeout(1500)
    finally:
        page.remove_listener("request", on_request)

    assert not history_requests, (
        f"expected no /api/chat/history calls on brief tab switch, got: {history_requests}"
    )
    preserved = await page.locator('[data-e2e-preserved="yes"]').count()
    assert preserved == 1, "chat DOM was re-rendered (sentinel attribute lost)"


async def _create_new_user_thread(page) -> str:
    """Click the "new thread" button and return the newly created thread's id.

    The wait condition must check that ``currentThreadId`` changed to a *new*
    value. Prior tests in the session may have left the server's
    ``active_thread`` pointing at an older user-created thread, in which case
    the initial page load already sets ``currentThreadId`` and a looser wait
    would pass immediately — before ``createNewThread()`` resolves.
    """
    prev_thread_id = await page.evaluate("() => currentThreadId")
    await page.locator("#thread-new-btn").click()
    await page.wait_for_function(
        """(prev) => !!currentThreadId && currentThreadId !== prev""",
        arg=prev_thread_id,
        timeout=15000,
    )
    return await page.evaluate("() => currentThreadId")


async def test_refresh_without_hash_reopens_active_thread_history(browser, managed_gateway_server):
    """Refreshing should reopen the server active thread when the URL has no thread hash."""
    context, page = await _open_gateway_page(browser, managed_gateway_server.base_url)
    try:
        thread_id = await _create_new_user_thread(page)

        send_response = await api_post(
            managed_gateway_server.base_url,
            "/api/chat/send",
            json={
                "thread_id": thread_id,
                "content": "Refresh should keep this thread",
            },
        )
        assert send_response.status_code == 202, send_response.text
        deadline = asyncio.get_running_loop().time() + 15
        while asyncio.get_running_loop().time() < deadline:
            history_response = await api_get(
                managed_gateway_server.base_url,
                f"/api/chat/history?thread_id={thread_id}",
            )
            assert history_response.status_code == 200, history_response.text
            if history_response.json().get("turns"):
                break
            await asyncio.sleep(0.5)
        else:
            raise AssertionError("Timed out waiting for persisted thread history before refresh")

        await page.evaluate(
            "() => history.replaceState(null, '', location.pathname + location.search)"
        )
        await page.reload()
        await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
        await _wait_for_connected(page, timeout=15000)
        await page.locator(SEL["message_user"]).filter(
            has_text="Refresh should keep this thread"
        ).wait_for(state="visible", timeout=30000)
        current_thread = await page.evaluate(
            "() => typeof currentThreadId === 'undefined' ? null : currentThreadId"
        )
        assert current_thread == thread_id or thread_id in page.url
    finally:
        await context.close()


async def test_refresh_skips_readonly_external_active_thread(page):
    """When the server active_thread is an external-channel (HTTP/Telegram) thread,
    a refresh without a hash should fall through to the writable gateway
    conversation with the chat input enabled, not land on the read-only thread."""

    # 1. Create a secondary thread and send a message so it becomes active_thread.
    # Seed it through the API instead of waiting on a browser round-trip here;
    # this test only cares that refresh sees a writable gateway thread with
    # persisted history before we patch its reported channel to read-only.
    ext_thread_id = await _create_new_user_thread(page)
    base_url = await page.evaluate("() => location.origin")

    response = await api_post(
        base_url,
        "/api/chat/send",
        json={
            "thread_id": ext_thread_id,
            "content": "Readonly channel test message",
        },
    )
    assert response.status_code == 202, response.text

    deadline = asyncio.get_running_loop().time() + 15
    while asyncio.get_running_loop().time() < deadline:
        history_response = await api_get(
            base_url,
            f"/api/chat/history?thread_id={ext_thread_id}",
        )
        assert history_response.status_code == 200, history_response.text
        if history_response.json().get("turns"):
            break
        await asyncio.sleep(0.5)
    else:
        raise AssertionError("Timed out waiting for persisted history before refresh")

    # 2. Strip the URL hash so reload relies on active_thread
    await page.evaluate(
        "() => history.replaceState(null, '', location.pathname + location.search)"
    )

    # 3. Intercept /api/chat/threads to mark the active thread as "http" channel.
    # The frontend polls this endpoint (loadThreads runs on page load and on
    # every SSE reconnect via debouncedLoadThreads), so the handler can be
    # mid-`route.fetch()` when the page context tears down — always pair this
    # with `page.unroute_all(behavior="ignoreErrors")` at test end to drain
    # in-flight callbacks, otherwise the cancelled fetch surfaces as a
    # TargetClosedError on the next test's Browser.new_context() call.
    fallback_gateway_thread_id = None

    async def patch_threads_response(route):
        nonlocal fallback_gateway_thread_id
        response = await route.fetch()
        body = await response.json()
        assistant_thread = body.get("assistant_thread") or {}
        fallback_gateway_thread_id = assistant_thread.get("id")
        for t in body.get("threads", []):
            if t["id"] == ext_thread_id:
                t["channel"] = "http"
        await route.fulfill(response=response, json=body)

    await page.route("**/api/chat/threads", patch_threads_response)

    # 4. Reload — loadThreads() should skip the "http" active thread
    await page.reload()
    await page.wait_for_selector("#auth-screen", state="hidden", timeout=15000)
    await _wait_for_connected(page, timeout=15000)

    # 5. Assert we landed on the writable gateway conversation, not the
    # patched read-only external thread, and that the legacy pinned assistant
    # chrome is gone from the DOM.
    assert fallback_gateway_thread_id, "expected a gateway fallback thread id"
    await page.wait_for_function(
        "(expected) => currentThreadId === expected",
        arg=fallback_gateway_thread_id,
        timeout=15000,
    )
    assert await page.locator("#assistant-thread").count() == 0

    # 6. Chat input should be enabled (not disabled by read-only state)
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    is_disabled = await chat_input.is_disabled()
    assert not is_disabled, "Chat input should be enabled on the fallback gateway thread"

    # Drain in-flight route callbacks before the `page` fixture closes the
    # context (see the setup comment at step 3 for the root cause). The
    # `ignoreErrors` behavior swallows the cancellation of any mid-flight
    # `route.fetch()` so it cannot surface on an unrelated later test.
    await page.unroute_all(behavior="ignoreErrors")


async def test_sse_keepalive_comments_arrive(managed_gateway_server):
    """Idle SSE connections should receive keepalive comments within 30 seconds."""
    async with sse_stream(managed_gateway_server.base_url, timeout=50) as response:
        assert response.status == 200
        keepalive = await wait_for_sse_comment(response, timeout=40)
        assert keepalive.startswith(":")


async def test_multiple_tabs_receive_same_response(browser, managed_gateway_server):
    """A message sent in one tab should fan out to another tab via SSE."""
    ctx_a, page_a = await _open_gateway_page(browser, managed_gateway_server.base_url)
    ctx_b, page_b = await _open_gateway_page(browser, managed_gateway_server.base_url)

    try:
        before_b = await page_b.locator(SEL["message_assistant"]).count()
        result_a = await send_chat_and_wait_for_terminal_message(page_a, "What is 2+2?")
        assert result_a["role"] == "assistant"
        assert "4" in result_a["text"], result_a

        await page_b.wait_for_function(
            """(count) => document.querySelectorAll('#chat-messages .message.assistant').length > count""",
            arg=before_b,
            timeout=15000,
        )
        assistant_b = await page_b.locator(SEL["message_assistant"]).last.text_content()
        assert assistant_b is not None
        assert "4" in assistant_b, assistant_b
    finally:
        await ctx_a.close()
        await ctx_b.close()


async def test_reconnect_after_server_restart_rebuilds_history(browser, managed_gateway_server):
    """After a server restart, reconnect should reload chat history from the DB."""
    context, page = await _open_gateway_page(browser, managed_gateway_server.base_url)

    try:
        result = await send_chat_and_wait_for_terminal_message(page, "What is 2+2?")
        assert result["role"] == "assistant"
        assert "4" in result["text"], result

        thread_id = await page.evaluate("() => currentThreadId")
        assert thread_id is not None

        async with page.expect_response(
            lambda response: (
                response.request.method == "GET"
                and response.ok
                and response.url.startswith(
                    f"{managed_gateway_server.base_url}/api/chat/history"
                )
                and f"thread_id={thread_id}" in response.url
            ),
            timeout=30000,
        ):
            # Simulate a long disconnect so the reconnect exercises the
            # history-reload path. Real restart cycles on fast hardware can
            # complete inside the SSE_RELOAD_THRESHOLD_MS window; the ||=
            # pattern in onerror preserves this pre-seeded timestamp.
            await page.evaluate("_sseDisconnectedAt = Date.now() - 15000")
            await managed_gateway_server.restart()
            await _wait_for_connected(page, timeout=30000)

        user_texts = await page.locator(SEL["message_user"]).all_text_contents()
        assistant_texts = await page.locator(SEL["message_assistant"]).all_text_contents()
        assert any("2+2" in text or "2 + 2" in text for text in user_texts), user_texts
        assert any("4" in text for text in assistant_texts), assistant_texts
    finally:
        await context.close()


async def test_reconnect_with_stale_last_event_id_does_not_duplicate_messages(
    browser,
    managed_gateway_server,
):
    """Reconnecting with an older event ID should rebuild history without duplicates."""
    context, page = await _open_gateway_page(browser, managed_gateway_server.base_url)

    try:
        first_result = await send_chat_and_wait_for_terminal_message(page, "Hello")
        assert first_result["role"] == "assistant"
        old_event_id = await _wait_for_last_event_id(page)
        thread_id = await page.evaluate("() => currentThreadId")
        assert thread_id is not None

        await page.evaluate("if (eventSource) eventSource.close()")

        response = await api_post(
            managed_gateway_server.base_url,
            "/api/chat/send",
            json={"content": "What is 2+2?", "thread_id": thread_id},
        )
        assert response.status_code == 202, response.text
        await _wait_for_turn_in_history(
            managed_gateway_server.base_url,
            thread_id,
            "The answer is 4.",
        )

        async with page.expect_response(
            lambda response: (
                response.request.method == "GET"
                and response.ok
                and response.url.startswith(
                    f"{managed_gateway_server.base_url}/api/chat/history"
                )
                and f"thread_id={thread_id}" in response.url
            ),
            timeout=20000,
        ):
            await page.evaluate("_sseDisconnectedAt = Date.now() - 15000")
            await page.evaluate("(eventId) => connectSSE(eventId)", old_event_id)
            await _wait_for_connected(page, timeout=20000)

        user_texts = await page.locator(SEL["message_user"]).all_text_contents()
        assistant_texts = await page.locator(SEL["message_assistant"]).all_text_contents()

        two_plus_two_users = [
            text for text in user_texts if "2+2" in text or "2 + 2" in text
        ]
        four_answers = [text for text in assistant_texts if "The answer is 4." in text]

        assert len(two_plus_two_users) == 1, user_texts
        assert len(four_answers) == 1, assistant_texts
    finally:
        await context.close()


async def test_connection_limit_returns_503_for_excess_sse_connection(limited_gateway_server):
    """Excess SSE connections should be rejected once the configured cap is reached."""
    async with sse_stream(limited_gateway_server.base_url, timeout=15) as first:
        assert first.status == 200
        async with sse_stream(limited_gateway_server.base_url, timeout=15) as second:
            assert second.status == 200
            async with sse_stream(limited_gateway_server.base_url, timeout=15) as third:
                body = await third.text()
                assert third.status == 503, body
                assert "Too many connections" in body
