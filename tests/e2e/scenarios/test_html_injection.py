"""Scenario 5: HTML injection defense in chat messages."""

import pytest
from helpers import SEL


XSS_PAYLOAD = (
    'Here is some content: <script>alert("xss")</script> and '
    '<img src=x onerror="alert(1)"> and '
    '<iframe src="javascript:alert(2)"></iframe> end of content.'
)


async def test_html_injection_sanitized(page):
    """XSS vectors in assistant messages should be sanitized by renderMarkdown."""
    # Inject an assistant message with XSS vectors directly via JS.
    # This tests the sanitization pipeline (renderMarkdown → sanitizeRenderedHtml)
    # without depending on the full LLM round-trip.
    await page.evaluate(
        "content => addMessage('assistant', content)", XSS_PAYLOAD
    )

    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=5000)

    inner_html = await assistant_msg.inner_html()

    # Script tags must be stripped
    assert "<script>" not in inner_html.lower(), \
        "Script tags were not sanitized from the response"

    # iframes must be stripped
    assert "<iframe" not in inner_html.lower(), \
        "iframe tags were not sanitized from the response"

    # Event handlers must be stripped
    assert "onerror=" not in inner_html.lower(), \
        "Event handler attributes were not sanitized"

    # The safe text content should still be present
    text = await assistant_msg.text_content()
    assert "content" in text.lower(), \
        "Safe text was lost during sanitization"


async def test_user_message_not_html_rendered(page):
    """User messages should be plain text, never rendered as HTML."""
    chat_input = page.locator(SEL["chat_input"])
    dangerous_input = '<img src=x onerror="alert(1)">'
    await chat_input.fill(dangerous_input)
    await chat_input.press("Enter")

    user_msg = page.locator(SEL["message_user"]).last
    await user_msg.wait_for(state="visible", timeout=5000)

    # The message should show the raw text, not render an img tag
    text = await user_msg.text_content()
    assert "<img" in text, \
        "User message HTML should be shown as plain text, not stripped"

    # The inner HTML should have the text escaped (< becomes &lt;)
    inner = await user_msg.inner_html()
    assert "&lt;img" in inner, \
        "User message was rendered as HTML instead of plain text"


async def test_no_script_elements_after_injection(page):
    """Verify that script tags in responses don't create DOM script elements."""
    await page.evaluate(
        "content => addMessage('assistant', content)", XSS_PAYLOAD
    )

    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=5000)

    # Wait a moment for any scripts to potentially execute
    await page.wait_for_timeout(500)

    # Verify no <script> elements exist in the chat messages
    script_count = await page.locator("#chat-messages script").count()
    assert script_count == 0, \
        f"Found {script_count} unescaped script elements in chat messages"
