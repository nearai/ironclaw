"""Scenario 3: SSE reconnection preserves history."""

import pytest
from helpers import SEL


async def test_sse_status_shows_connected(page):
    """SSE status should show Connected after page load."""
    status = page.locator(SEL["sse_status"])
    await status.wait_for(state="visible", timeout=5000)
    text = await status.text_content()
    assert text == "Connected", f"Expected 'Connected', got '{text}'"


async def test_sse_reconnect_after_disconnect(page):
    """After programmatic disconnect, SSE should reconnect and show Connected."""
    # Verify initial connection
    status = page.locator(SEL["sse_status"])
    await page.wait_for_function(
        'document.getElementById("sse-status").textContent === "Connected"',
        timeout=5000,
    )

    # Close the EventSource to simulate disconnect
    await page.evaluate("if (eventSource) eventSource.close()")

    # Status should change to Reconnecting... or similar
    # (EventSource auto-reconnects, but we closed it manually so we need to reconnect)
    await page.evaluate("connectSSE()")

    # Wait for reconnection
    await page.wait_for_function(
        'document.getElementById("sse-status").textContent === "Connected"',
        timeout=10000,
    )
    text = await status.text_content()
    assert text == "Connected"


async def test_sse_reconnect_preserves_chat_history(page):
    """Messages sent before disconnect should still be visible after reconnect."""
    # Send a message first
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.fill("Hello")
    await chat_input.press("Enter")

    # Wait for assistant response
    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=15000)

    # Count messages before disconnect
    user_count_before = await page.locator(SEL["message_user"]).count()
    assistant_count_before = await page.locator(SEL["message_assistant"]).count()
    assert user_count_before >= 1
    assert assistant_count_before >= 1

    # Simulate disconnect and reconnect
    await page.evaluate("if (eventSource) eventSource.close()")
    await page.evaluate("connectSSE()")

    # Wait for reconnection
    await page.wait_for_function(
        'document.getElementById("sse-status").textContent === "Connected"',
        timeout=10000,
    )

    # Wait for history reload (loadHistory is called on reconnect)
    await page.wait_for_timeout(2000)

    # Messages should still be present (possibly refreshed from server)
    user_count_after = await page.locator(SEL["message_user"]).count()
    assistant_count_after = await page.locator(SEL["message_assistant"]).count()
    assert user_count_after >= user_count_before, \
        f"User messages lost: {user_count_before} -> {user_count_after}"
    assert assistant_count_after >= assistant_count_before, \
        f"Assistant messages lost: {assistant_count_before} -> {assistant_count_after}"
