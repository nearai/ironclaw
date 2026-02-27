"""Scenario 2: Chat message round-trip via SSE streaming."""

import pytest
from helpers import SEL


async def test_send_message_and_receive_response(page):
    """Type a message, receive a streamed response from mock LLM."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # Send message
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Wait for assistant response
    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=15000)

    # Verify user message
    user_msgs = page.locator(SEL["message_user"])
    assert await user_msgs.count() >= 1
    last_user = user_msgs.last
    user_text = await last_user.text_content()
    assert "2+2" in user_text or "2 + 2" in user_text

    # Verify assistant response contains "4" (from mock LLM canned response)
    assistant_text = await assistant_msg.text_content()
    assert "4" in assistant_text, f"Expected '4' in response, got: '{assistant_text}'"


async def test_multiple_messages(page):
    """Send two messages, verify both get responses."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    # First message
    await chat_input.fill("Hello")
    await chat_input.press("Enter")

    # Wait for first response
    await page.locator(SEL["message_assistant"]).first.wait_for(
        state="visible", timeout=15000
    )

    # Second message
    await chat_input.fill("What is 2+2?")
    await chat_input.press("Enter")

    # Wait for second response (at least 2 assistant messages)
    await page.wait_for_function(
        """() => document.querySelectorAll('#chat-messages .message.assistant').length >= 2""",
        timeout=15000,
    )

    # Verify counts
    user_count = await page.locator(SEL["message_user"]).count()
    assistant_count = await page.locator(SEL["message_assistant"]).count()
    assert user_count >= 2, f"Expected >= 2 user messages, got {user_count}"
    assert assistant_count >= 2, f"Expected >= 2 assistant messages, got {assistant_count}"


async def test_empty_message_not_sent(page):
    """Pressing Enter with empty input should not create a message."""
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)

    initial_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()

    # Press Enter with empty input
    await chat_input.press("Enter")

    # Wait a moment and verify no new messages
    await page.wait_for_timeout(2000)
    final_count = await page.locator(f"{SEL['message_user']}, {SEL['message_assistant']}").count()
    assert final_count == initial_count, "Empty message should not create new messages"
