"""Tool execution e2e tests.

Tests the agent loop: user message -> mock LLM returns tool_calls -> tool
executes -> result displayed in chat.  Requires the enhanced mock_llm.py
with TOOL_CALL_PATTERNS support.
"""

from helpers import SEL


async def _send_and_get_response(
    page,
    message: str,
    *,
    expected_fragment: str,
    timeout: int = 30000,
) -> str:
    """Send a message and return the text of the newest assistant response.

    Counts existing assistant messages before sending, then waits for a new
    one to appear and contain the expected final text fragment. This avoids
    reading partial streamed content before the assistant response is complete.
    """
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Count existing assistant messages before sending
    assistant_sel = SEL["message_assistant"]
    before_count = await page.locator(assistant_sel).count()

    await chat_input.fill(message)
    await chat_input.press("Enter")

    # Wait for the final assistant message to exist and include the expected
    # text fragment rather than returning on the first streamed chunk.
    expected = before_count + 1
    await page.wait_for_function(
        """({ assistantSelector, expectedCount, expectedFragment }) => {
            const messages = document.querySelectorAll(assistantSelector);
            if (messages.length < expectedCount) return false;
            const text = (messages[messages.length - 1].innerText || '').trim().toLowerCase();
            return text.includes(expectedFragment.toLowerCase());
        }""",
        arg={
            "assistantSelector": assistant_sel,
            "expectedCount": expected,
            "expectedFragment": expected_fragment,
        },
        timeout=timeout,
    )

    return await page.locator(assistant_sel).last.inner_text()


async def test_builtin_echo_tool(page):
    """Send a message that triggers the echo tool via mock LLM function calling."""
    text = await _send_and_get_response(
        page,
        "echo hello world",
        expected_fragment="hello world",
    )

    # The mock LLM returns "The echo tool returned: <result>"
    assert "echo" in text.lower() or "hello world" in text.lower(), (
        f"Expected echo result in response, got: {text}"
    )


async def test_builtin_time_tool(page):
    """Send a message that triggers the time tool via mock LLM function calling."""
    text = await _send_and_get_response(
        page,
        "what time is it",
        expected_fragment="time",
    )

    # The mock LLM returns "The time tool returned: <json with iso/unix>"
    assert "time" in text.lower(), (
        f"Expected time result in response, got: {text}"
    )


async def test_non_tool_message_still_works(page):
    """Messages that don't match tool patterns still get text responses."""
    text = await _send_and_get_response(
        page,
        "What is 2+2?",
        expected_fragment="4",
        timeout=15000,
    )

    assert "4" in text, (
        f"Expected '4' in response, got: {text}"
    )
