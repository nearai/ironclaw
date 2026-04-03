"""Agent-loop recovery regressions for issue #1780."""

from helpers import SEL, send_chat_and_wait_for_terminal_message


async def _create_new_thread(page) -> None:
    previous_thread = await page.evaluate("() => currentThreadId")
    await page.evaluate("createNewThread()")
    await page.wait_for_function(
        """(prevThread) => {
            return !!currentThreadId
                && currentThreadId !== prevThread
                && document.querySelectorAll('#chat-messages .message').length === 0;
        }""",
        arg=previous_thread,
        timeout=10000,
    )


async def test_tool_failure_recovery_shows_final_response(page):
    """A failed tool call should surface a final assistant response and not hang."""
    await _create_new_thread(page)
    result = await send_chat_and_wait_for_terminal_message(
        page,
        "issue 1780 tool failure",
    )

    assert result["role"] == "assistant", result
    text = result["text"].lower()
    assert "time tool returned" in text, result
    assert (
        "unknown operation" in text
        or "broken-operation" in text
        or "error" in text
    ), result
    assert await page.locator(SEL["chat_input"]).is_enabled()


async def test_truncated_tool_call_is_discarded_gracefully(length_preserving_page):
    """A length-truncated tool call should be ignored and the turn should recover."""
    await _create_new_thread(length_preserving_page)
    result = await send_chat_and_wait_for_terminal_message(
        length_preserving_page,
        "issue 1780 truncated tool call",
    )

    assert result["role"] == "assistant", result
    assert "recovered after discarding a truncated tool call" in result["text"].lower(), result
    assert await length_preserving_page.locator(SEL["approval_card"]).count() == 0
    assert await length_preserving_page.locator(SEL["chat_input"]).is_enabled()


async def test_empty_reply_uses_chat_fallback(page):
    """An empty LLM reply should terminate visibly instead of hanging."""
    await _create_new_thread(page)
    result = await send_chat_and_wait_for_terminal_message(
        page,
        "issue 1780 empty reply",
    )

    assert "error:" in result["text"].lower(), result
    assert "empty" in result["text"].lower(), result
    assert await page.locator(SEL["chat_input"]).is_enabled()


async def test_looping_tool_calls_terminate_under_low_iteration_limit(loop_limited_page):
    """A looping tool-call pattern should terminate once force-text kicks in."""
    await _create_new_thread(loop_limited_page)
    result = await send_chat_and_wait_for_terminal_message(
        loop_limited_page,
        "issue 1780 loop forever",
        timeout=20000,
    )

    assert result["role"] == "assistant", result
    assert "recovered after hitting the tool iteration limit" in result["text"].lower(), result
    assert await loop_limited_page.locator(SEL["chat_input"]).is_enabled()
