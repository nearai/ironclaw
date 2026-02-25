"""Scenario 5: HTML injection defense in chat messages."""

import pytest
from helpers import SEL


async def test_html_injection_sanitized(page):
    """XSS vectors in LLM responses should be sanitized, not rendered."""
    # Send a message that triggers the mock LLM to return HTML content
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("html test")
    await chat_input.press("Enter")

    # Wait for assistant response
    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=15000)

    # Get the rendered HTML of the assistant message
    inner_html = await assistant_msg.inner_html()

    # The sanitizer should have removed these dangerous elements
    # Script tags must be stripped
    assert "<script>" not in inner_html.lower(), \
        "Script tags were not sanitized from the response"
    assert "alert(" not in inner_html.lower() or "alert(" in (await assistant_msg.text_content()).lower(), \
        "Script code was rendered as executable HTML"

    # iframes must be stripped
    assert "<iframe" not in inner_html.lower(), \
        "iframe tags were not sanitized from the response"

    # Event handlers must be stripped
    assert "onerror=" not in inner_html.lower(), \
        "Event handler attributes were not sanitized"

    # The text content should still contain the safe parts
    text = await assistant_msg.text_content()
    assert "content" in text.lower(), \
        "Safe text was lost during sanitization"


async def test_user_message_not_html_rendered(page):
    """User messages should be plain text, never rendered as HTML."""
    chat_input = page.locator(SEL["chat_input"])
    dangerous_input = '<img src=x onerror="alert(1)">'
    await chat_input.fill(dangerous_input)
    await chat_input.press("Enter")

    # Wait briefly for the message to appear
    user_msg = page.locator(SEL["message_user"]).last
    await user_msg.wait_for(state="visible", timeout=5000)

    # The message should show the raw text, not render an img tag
    text = await user_msg.text_content()
    assert "<img" in text, \
        "User message HTML should be shown as plain text, not stripped"

    # The inner HTML should have the text escaped
    inner = await user_msg.inner_html()
    # If properly escaped, the < should be &lt; in innerHTML
    assert "&lt;img" in inner or "<img" not in inner or "textContent" in inner, \
        "User message was rendered as HTML instead of plain text"


async def test_no_script_execution_in_response(page):
    """Verify that script tags in responses don't actually execute."""
    # Set up a detection mechanism
    await page.evaluate("window.__xss_test = false")

    # Send message triggering HTML response
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("html test")
    await chat_input.press("Enter")

    # Wait for response
    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=15000)

    # Wait a moment for any scripts to potentially execute
    await page.wait_for_timeout(1000)

    # Check our detection flag - if XSS worked, alert() would have fired
    # (We can't easily detect alert(), but we can check for DOM injection)
    # Verify no <script> elements exist in the chat messages
    script_count = await page.locator("#chat-messages script").count()
    assert script_count == 0, \
        f"Found {script_count} unescaped script elements in chat messages"
